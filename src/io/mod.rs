//! File and project workflow for the command-line formatter.
//!
//! This module deliberately contains no formatting rules. It selects sources,
//! builds one project context, delegates bytes to the library formatter, and
//! performs the requested output operation.
//!
//! The work is split by the question each part answers: `sources` and `select`
//! decide which files the invocation reads, `context` turns them into the
//! analysis and runs the formatter, `diff` and `write` deliver the result, and
//! `report` says what was declined. What stays here is the sequencing —
//! [`execute`] and the phases it walks through.

mod context;
mod diff;
mod exclude;
mod report;
mod select;
mod sources;
mod write;

use context::{analyze_sources, format_one, format_targets, isolated_context, project_context};
use diff::unified_diff;
use exclude::ExcludeMatcher;
use report::{fixed_message, input_name, skips_fixed_form, DeclineReporter};
use select::{deduplicate_indices, select_paths, Loaded, Scope, Selection};
use sources::{display_path, read_source, resolve_input, tracked_sources_without_submodules};
use write::write_all_stdout;

pub use sources::{repository_root, tracked_sources, validate_extension};
pub use write::atomic_replace;

use crate::{
    analysis::{analyze_file, analyze_file_at},
    cli::Invocation,
    error::FormatError,
    format_source,
    source::SourceForm,
};
use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Instant,
};

/// A workflow failure carries the exit-status class required by the CLI.
#[derive(Debug)]
pub enum WorkflowError {
    Usage(String),
    Io(io::Error),
    Format(FormatError),
    /// A formatting failure with the input it came from already rendered.
    ///
    /// `FormatError` is about a source buffer, so it cannot name a file — the
    /// same division `report` describes for declined wraps. A bulk run
    /// formats every target before it writes anything, so an unnamed failure
    /// there reports that nothing was written without saying which of the
    /// selected files caused it.
    FormatIn {
        input: String,
        error: FormatError,
    },
}

impl WorkflowError {
    pub fn status(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Io(_) | Self::Format(_) | Self::FormatIn { .. } => 1,
        }
    }

    pub fn is_broken_pipe(&self) -> bool {
        match self {
            Self::Io(error) => error.kind() == io::ErrorKind::BrokenPipe,
            Self::Format(error) | Self::FormatIn { error, .. } => error.is_broken_pipe(),
            Self::Usage(_) => false,
        }
    }

    /// Name the input a formatting failure came from.
    ///
    /// Only formatting failures are renamed: an I/O error already names the
    /// path it failed on, and a usage error belongs to the invocation rather
    /// than to any one input.
    fn in_input(self, input: String) -> Self {
        match self {
            Self::Format(error) => Self::FormatIn { input, error },
            other => other,
        }
    }
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Io(error) => error.fmt(f),
            Self::Format(error) => error.fmt(f),
            Self::FormatIn { input, error } => write!(f, "{input}: {error}"),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl From<io::Error> for WorkflowError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FormatError> for WorkflowError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

/// Name the input that a formatting call failed on.
///
/// The formatter is handed a source buffer and cannot name a file, so every
/// route that formats a named input adds the name here — the same division
/// `report` describes for declined wraps.
fn in_input<T, E: Into<WorkflowError>>(
    result: Result<T, E>,
    path: Option<&Path>,
    root: Option<&Path>,
) -> Result<T, WorkflowError> {
    result.map_err(|error| error.into().in_input(input_name(path, root)))
}

fn execute_query_format(invocation: Invocation) -> Result<i32, WorkflowError> {
    if invocation.stdin || (invocation.paths.is_empty() && !invocation.all && !invocation.all_files)
    {
        let mut source = Vec::new();
        io::stdin().read_to_end(&mut source)?;
        println!("{}", source_form_name(crate::source::detect(&source)));
        return Ok(0);
    }

    let cwd = env::current_dir()?;
    let all_scope = if invocation.all || invocation.all_files {
        invocation
            .paths
            .first()
            .map(|path| {
                let candidate = resolve_input(path, None);
                let canonical = fs::canonicalize(&candidate).map_err(|error| {
                    WorkflowError::Usage(format!(
                        "--all/--all-files directory does not exist: {} ({error})",
                        candidate.display()
                    ))
                })?;
                if !fs::metadata(&canonical)?.is_dir() {
                    return Err(WorkflowError::Usage(format!(
                        "--all/--all-files requires a directory: {}",
                        candidate.display()
                    )));
                }
                Ok(canonical)
            })
            .transpose()?
    } else {
        None
    };
    let root = if let Some(scope) = all_scope.as_deref() {
        repository_root(scope)?
    } else {
        repository_root(&cwd)?
    };
    let exclude_matcher = ExcludeMatcher::new(&invocation.exclude_patterns());

    let paths: Vec<PathBuf> = if invocation.all || invocation.all_files {
        let root = root
            .as_deref()
            .ok_or_else(|| WorkflowError::Usage("--all requires a valid Git checkout".into()))?;
        let reader = if invocation.no_submodules || invocation.all_files {
            tracked_sources_without_submodules
        } else {
            tracked_sources
        };
        let paths = reader(root)?
            .into_iter()
            .filter(|path| !exclude_matcher.is_excluded(root, path));
        match all_scope.as_deref() {
            Some(scope) => paths.filter(|path| path.starts_with(scope)).collect(),
            None => paths.collect(),
        }
    } else {
        invocation
            .paths
            .iter()
            .map(|path| resolve_input(path, root.as_deref()))
            .collect()
    };

    for path in paths {
        let source = read_source(&path, false)?.ok_or_else(|| {
            WorkflowError::Usage(format!(
                "Fortran source file does not exist: {}",
                path.display()
            ))
        })?;
        println!("{}", source_form_name(source.form));
    }
    Ok(0)
}

fn source_form_name(form: SourceForm) -> &'static str {
    match form {
        SourceForm::Free => "free",
        SourceForm::Fixed => "fixed",
    }
}

/// A lone positional argument that names a directory selects that directory's
/// tracked sources, matching `--all-files DIR` and the directory-recursion
/// convention of other formatters. `--stdout` and `--isolated` keep their
/// stricter single-file semantics, so a directory there is left to fail with
/// its existing "not a source file" diagnostic.
fn promote_directory_argument(mut invocation: Invocation) -> Invocation {
    if invocation.all
        || invocation.all_files
        || invocation.stdout
        || invocation.isolated
        || invocation.paths.len() != 1
    {
        return invocation;
    }
    if resolve_input(&invocation.paths[0], None).is_dir() {
        invocation.all_files = true;
    }
    invocation
}

/// Execute one parsed invocation. Return value is the process status for a
/// successful operation: 0 clean/success, 1 differences found.
///
/// The body is the sequence and nothing else. Each phase below answers one
/// question — where are we rooted, which files, what do they say, where does
/// the output go — and the early returns here are the routes that can answer
/// the last one before the ones in between have to be paid for.
pub fn execute(invocation: Invocation) -> Result<i32, WorkflowError> {
    let invocation = promote_directory_argument(invocation);
    if invocation.query_format {
        return execute_query_format(invocation);
    }
    let all_selection = invocation.all || invocation.all_files;
    let stdin_mode = invocation.stdin || (invocation.paths.is_empty() && !all_selection);
    // A buffer on stdin with no project to read needs none of the discovery
    // below, and this is the route an editor takes on every keystroke, so it
    // does not pay for any of it.
    if stdin_mode && invocation.project_context.is_none() && invocation.context_paths.is_empty() {
        return format_bare_stdin(&invocation);
    }

    let profile = Profile::new();
    let cwd = env::current_dir()?;
    let stdin_source = if stdin_mode {
        let mut source = Vec::new();
        io::stdin().read_to_end(&mut source)?;
        Some(source)
    } else {
        None
    };
    let scope = Scope::resolve(&invocation, &cwd, all_selection)?;
    if let Some(status) = stdin_shortcut(&invocation, &scope, stdin_source.as_deref())? {
        return Ok(status);
    }

    let mut selection = select_paths(&invocation, &scope, &cwd, stdin_mode, all_selection)?;
    if invocation.show_files {
        for path in &selection.targets {
            println!("{}", display_path(path, scope.root.as_deref()).display());
        }
        return Ok(0);
    }
    profile.report(|| {
        format!(
            "discovery={:?} targets={} project={} loaded-set={}",
            profile.elapsed(),
            selection.targets.len(),
            selection.project.len(),
            selection.all().len()
        )
    });

    let loaded = Loaded::read(&selection, &invocation, all_selection || stdin_mode)?;
    selection.retain_loaded(&loaded);
    profile.report(|| {
        format!(
            "read={:?} sources={}",
            profile.elapsed(),
            loaded.sources.len()
        )
    });

    let target_indices = selection.free_form_indices(&selection.targets, &loaded);
    let project_indices = selection.free_form_indices(&selection.project, &loaded);
    let analysis_indices = if invocation.config.mode.normalizes() {
        deduplicate_indices(&target_indices, &project_indices)
    } else {
        Vec::new()
    };
    let facts = analyze_sources(&loaded.sources, &analysis_indices)?;
    let stdin_local = if stdin_mode && invocation.config.mode.normalizes() {
        let source = expect_stdin(stdin_source.as_deref());
        Some(match scope.stdin_path() {
            Some(path) => analyze_file_at(path, source)?,
            None => analyze_file(source)?,
        })
    } else {
        None
    };
    let context = build_context(
        &invocation,
        &scope,
        &loaded,
        &project_indices,
        &facts,
        stdin_local.as_ref(),
    );
    profile.report(|| format!("analysis={:?}", profile.elapsed()));

    if stdin_mode {
        return format_project_stdin(
            &invocation,
            &scope,
            expect_stdin(stdin_source.as_deref()),
            &context,
            stdin_local.as_ref(),
        );
    }
    let prepared = PreparedRun {
        invocation: &invocation,
        scope: &scope,
        selection: &selection,
        loaded: &loaded,
        facts: &facts,
        context: &context,
    };
    if invocation.stdout {
        return format_to_stdout(&prepared);
    }
    format_files(&prepared, &target_indices, &profile)
}

fn expect_stdin(source: Option<&[u8]>) -> &[u8] {
    source.expect("stdin mode must have read stdin")
}

/// `FORFORMAT_PROFILE_IO` timing, inert unless the variable is set.
struct Profile {
    enabled: bool,
    start: Instant,
}

impl Profile {
    fn new() -> Self {
        Self {
            enabled: env::var_os("FORFORMAT_PROFILE_IO").is_some(),
            start: Instant::now(),
        }
    }

    fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    /// The message is built only when profiling is on, so an ordinary run
    /// pays nothing for the counts a profile line reports.
    fn report(&self, fields: impl FnOnce() -> String) {
        if self.enabled {
            eprintln!("forformat profile: {}", fields());
        }
    }
}

/// stdin with nothing else to read: format the buffer and write it out.
fn format_bare_stdin(invocation: &Invocation) -> Result<i32, WorkflowError> {
    let mut source = Vec::new();
    io::stdin().read_to_end(&mut source)?;
    // stdin is the primary documented route, so it needs the same guard the
    // file routes get: free-form normalization of a fixed-form source
    // rewrites column-1 `*`/`C` comment markers as operators and destroys
    // the file.  There is no path to name in the diagnostic here.
    if skips_fixed_form(invocation, None, &source) {
        eprintln!("{}", fixed_message("<stdin>"));
        write_all_stdout(&source)?;
        return Ok(0);
    }
    let result = in_input(format_source(&source, &invocation.config), None, None)?;
    let mut declines = DeclineReporter::default();
    declines.report(&result.meta, None, None);
    declines.finish();
    write_all_stdout(&result.bytes)?;
    Ok(0)
}

/// The two stdin routes that answer before any project source is read: a
/// fixed-form buffer, which is passed through untouched, and a mode that does
/// not consult the project at all.
fn stdin_shortcut(
    invocation: &Invocation,
    scope: &Scope,
    stdin_source: Option<&[u8]>,
) -> Result<Option<i32>, WorkflowError> {
    let Some(source) = stdin_source else {
        return Ok(None);
    };
    let input_path = scope.stdin_path();
    if skips_fixed_form(invocation, input_path, source) {
        let input = input_path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<stdin>".to_string());
        eprintln!("{}", fixed_message(&input));
        write_all_stdout(source)?;
        return Ok(Some(0));
    }
    if !invocation.config.mode.normalizes() {
        let formatted = in_input(
            format_source(source, &invocation.config),
            None,
            scope.root.as_deref(),
        )?;
        let mut declines = DeclineReporter::default();
        declines.report(&formatted.meta, None, scope.root.as_deref());
        declines.finish();
        write_all_stdout(&formatted.bytes)?;
        return Ok(Some(0));
    }
    Ok(None)
}

fn build_context(
    invocation: &Invocation,
    scope: &Scope,
    loaded: &Loaded,
    project_indices: &[usize],
    facts: &[Option<crate::analysis::FileFacts>],
    stdin_local: Option<&crate::analysis::FileFacts>,
) -> crate::analysis::ProjectContext {
    if invocation.isolated || !invocation.config.mode.normalizes() {
        return isolated_context(&invocation.config);
    }
    let stdin_project_source = scope
        .stdin_path()
        .filter(|path| {
            scope.context_paths.is_empty()
                || scope
                    .context_paths
                    .iter()
                    .any(|context_path| path.starts_with(context_path))
        })
        .zip(stdin_local);
    project_context(
        &loaded.sources,
        project_indices,
        facts,
        stdin_project_source,
        &invocation.config,
    )
}

fn format_project_stdin(
    invocation: &Invocation,
    scope: &Scope,
    source: &[u8],
    context: &crate::analysis::ProjectContext,
    stdin_local: Option<&crate::analysis::FileFacts>,
) -> Result<i32, WorkflowError> {
    let formatted = if !invocation.config.mode.normalizes() {
        in_input(
            format_source(source, &invocation.config),
            None,
            scope.root.as_deref(),
        )?
    } else {
        in_input(
            crate::format::full::format_with_context_and_local(
                source,
                context,
                stdin_local.expect("full-mode stdin must have precomputed facts"),
                &invocation.config,
            ),
            None,
            scope.root.as_deref(),
        )?
    };
    let mut declines = DeclineReporter::default();
    declines.report(&formatted.meta, None, scope.root.as_deref());
    declines.finish();
    write_all_stdout(&formatted.bytes)?;
    Ok(0)
}

/// Correlated immutable inputs shared by the prepared file-output routes.
///
/// Target indices and profiling stay explicit at the call site because they
/// describe a particular route's execution rather than the prepared context.
struct PreparedRun<'a> {
    invocation: &'a Invocation,
    scope: &'a Scope,
    selection: &'a Selection,
    loaded: &'a Loaded,
    facts: &'a [Option<crate::analysis::FileFacts>],
    context: &'a crate::analysis::ProjectContext,
}

fn format_to_stdout(run: &PreparedRun<'_>) -> Result<i32, WorkflowError> {
    let path = &run.selection.targets[0];
    let source_index = run.loaded.index[path];
    let mut declines = DeclineReporter::default();
    if run.loaded.sources[source_index].form == SourceForm::Fixed {
        declines.report_fixed(path, run.scope.root.as_deref());
        write_all_stdout(&run.loaded.sources[source_index].bytes)?;
    } else {
        let formatted = in_input(
            format_one(
                &run.loaded.sources[source_index],
                run.facts[source_index].as_ref(),
                run.context,
                &run.invocation.config,
            ),
            Some(path),
            run.scope.root.as_deref(),
        )?;
        declines.report(&formatted.meta, Some(path), run.scope.root.as_deref());
        write_all_stdout(&formatted.bytes)?;
    }
    declines.finish();
    Ok(0)
}

/// Format every target, then deliver the results: a diff, an in-place
/// replacement, or nothing at all under `--check`.
///
/// Every target is formatted before anything is written, so a failure part-way
/// through cannot leave a half-rewritten tree.
fn format_files(
    run: &PreparedRun<'_>,
    target_indices: &[usize],
    profile: &Profile,
) -> Result<i32, WorkflowError> {
    let formatting_start = Instant::now();
    let formatted = format_targets(
        &run.loaded.sources,
        target_indices,
        run.facts,
        run.context,
        &run.invocation.config,
        run.scope.root.as_deref(),
    )?;
    let mut changed = Vec::new();
    let mut declines = DeclineReporter::default();
    let mut formatted = formatted.into_iter();
    for path in &run.selection.targets {
        let target = &run.loaded.sources[run.loaded.index[path]];
        if target.form == SourceForm::Fixed {
            declines.report_fixed(path, run.scope.root.as_deref());
            continue;
        }
        let (meta, output) = formatted
            .next()
            .expect("one formatting result per free-form target");
        declines.report(&meta, Some(path), run.scope.root.as_deref());
        let Some(formatted) = output else {
            continue;
        };
        changed.push(path.clone());
        if run.invocation.diff {
            write_all_stdout(&unified_diff(
                path,
                &target.bytes,
                &formatted,
                run.scope.root.as_deref(),
            ))?;
        } else if !run.invocation.check {
            atomic_replace(path, &formatted)?;
        }
    }
    declines.finish();
    if !run.invocation.diff {
        for path in &changed {
            println!(
                "{}",
                display_path(path, run.scope.root.as_deref()).display()
            );
        }
    }
    profile.report(|| {
        format!(
            "formatting={:?} total={:?} changed={}",
            formatting_start.elapsed(),
            profile.elapsed(),
            changed.len()
        )
    });
    Ok(i32::from(
        (run.invocation.check || run.invocation.diff) && !changed.is_empty(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{in_input, WorkflowError};
    use crate::error::FormatError;
    use std::path::Path;

    /// A bulk run formats every target before it writes any of them, so the
    /// failure that stops it is the only place the offending file is named.
    #[test]
    fn a_formatting_failure_names_the_file_it_came_from() {
        let failure: Result<(), _> = Err(FormatError::Unsupported(
            "wrapping entered a cycle".to_owned(),
        ));
        let named = in_input(
            failure,
            Some(Path::new("/checkout/src/module.f90")),
            Some(Path::new("/checkout")),
        )
        .unwrap_err();
        assert_eq!(
            named.to_string(),
            "src/module.f90: unsupported: wrapping entered a cycle"
        );
        assert_eq!(named.status(), 1);

        let stdin: Result<(), _> = Err(FormatError::Unsupported("wrapping entered a cycle".into()));
        assert_eq!(
            in_input(stdin, None, None).unwrap_err().to_string(),
            "<stdin>: unsupported: wrapping entered a cycle"
        );
    }

    /// Naming the input must not reclassify the failure: a closed pipe still
    /// has to exit 0 rather than report a formatting error.
    #[test]
    fn naming_the_input_preserves_the_failure_class() {
        let broken: Result<(), _> = Err(FormatError::Write(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed",
        )));
        assert!(in_input(broken, Some(Path::new("a.f90")), None)
            .unwrap_err()
            .is_broken_pipe());

        // A usage error belongs to the invocation and an I/O error already
        // names its own path, so neither is renamed.
        let usage: Result<(), _> = Err(WorkflowError::Usage("bad option".to_owned()));
        assert_eq!(
            in_input(usage, Some(Path::new("a.f90")), None)
                .unwrap_err()
                .to_string(),
            "bad option"
        );
    }
}