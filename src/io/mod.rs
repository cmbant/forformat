//! File and project workflow for the command-line formatter.
//!
//! This module deliberately contains no formatting rules. It selects sources,
//! builds one project context, delegates bytes to the library formatter, and
//! performs the requested output operation.

use crate::{
    analysis::{analyze_file, FileFacts, ProjectContext},
    cli::{ContextPath, Invocation},
    config::FormatMode,
    error::FormatError,
    format_source,
    source::SourceForm,
    FormatResult,
};
mod exclude;
use exclude::ExcludeMatcher;
use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Instant,
};

const GIT_HOOK_VARS: [&str; 4] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
];

/// A workflow failure carries the exit-status class required by the CLI.
#[derive(Debug)]
pub enum WorkflowError {
    Usage(String),
    Io(io::Error),
    Format(FormatError),
}

impl WorkflowError {
    pub fn status(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Io(_) | Self::Format(_) => 1,
        }
    }

    pub fn is_broken_pipe(&self) -> bool {
        match self {
            Self::Io(error) => error.kind() == io::ErrorKind::BrokenPipe,
            Self::Format(error) => error.is_broken_pipe(),
            Self::Usage(_) => false,
        }
    }
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Io(error) => error.fmt(f),
            Self::Format(error) => error.fmt(f),
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

/// Validate only the suffix. This is intentionally pure: existence and file
/// opening are separate operations (§9.1 of the port plan).
pub fn validate_extension(path: &Path) -> Result<(), String> {
    let valid = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let suffix = format!(".{extension}");
            crate::transform::vocab::SOURCE_EXTENSIONS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&suffix))
        })
        .unwrap_or(false);
    if valid {
        Ok(())
    } else {
        Err(format!(
            "expected a free-form Fortran source (suffix match is case-insensitive: .f, .f03, .f08, .f18, .f23, .f90, .f95): {}",
            path.display()
        ))
    }
}

/// Run a nested git command with all hook repository variables removed.
/// Keeping this as the only git entry point makes F2 difficult to regress.
fn git(args: &[&str], cwd: &Path) -> io::Result<std::process::Output> {
    let mut command = Command::new("git");
    command.args(args).current_dir(cwd).stdin(Stdio::null());
    for variable in GIT_HOOK_VARS {
        command.env_remove(variable);
    }
    command.output()
}

fn git_path(raw: &[u8]) -> io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(raw.to_vec())))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(raw.to_vec())
            .map(OsString::from)
            .map(PathBuf::from)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

/// Find the checkout containing start.
pub fn repository_root(start: &Path) -> io::Result<Option<PathBuf>> {
    let output = git(&["rev-parse", "--show-toplevel"], start)?;
    if !output.status.success() {
        return Ok(None);
    }
    let raw = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    if raw.is_empty() {
        Ok(None)
    } else {
        Ok(Some(fs::canonicalize(git_path(raw)?)?))
    }
}

/// Return tracked free-form sources, accepting upper-case suffix spellings.
pub fn tracked_sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    tracked_sources_with_submodules(root, true)
}

/// Return tracked free-form sources from the checkout itself, excluding
/// sources owned by initialized submodules.
pub fn tracked_sources_without_submodules(root: &Path) -> io::Result<Vec<PathBuf>> {
    tracked_sources_with_submodules(root, false)
}

fn tracked_sources_with_submodules(
    root: &Path,
    recurse_submodules: bool,
) -> io::Result<Vec<PathBuf>> {
    let mut args = vec!["ls-files", "-z", "--"];
    if recurse_submodules {
        args.insert(1, "--recurse-submodules");
    }
    let output = git(&args, root)?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let mut paths = Vec::new();
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
    {
        let relative = git_path(raw)?;
        if validate_extension(&relative).is_ok() {
            paths.push(root.join(relative));
        }
    }
    paths.sort();
    Ok(paths)
}

#[derive(Debug)]
struct Source {
    path: PathBuf,
    bytes: Vec<u8>,
    form: SourceForm,
}

fn resolve_input(path: &Path, root: Option<&Path>) -> PathBuf {
    if path.is_absolute() || path.exists() {
        path.to_path_buf()
    } else if let Some(root) = root {
        let candidate = root.join(path);
        if candidate.exists() {
            candidate
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    }
}

fn read_source(path: &Path, force_free_input: bool) -> Result<Option<Source>, WorkflowError> {
    validate_extension(path).map_err(WorkflowError::Usage)?;
    let canonical = match fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkflowError::Io(error)),
    };
    let mut file = match File::open(&canonical) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkflowError::Io(error)),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(WorkflowError::Usage(format!(
            "Fortran source file is not a regular file: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let form = if force_free_input {
        SourceForm::Free
    } else {
        crate::source::detect_path(path, &bytes)
    };
    Ok(Some(Source {
        bytes,
        path: canonical,
        form,
    }))
}

/// Drop repeated paths, keeping the first occurrence and the original order.
///
/// The order is what makes diagnostics, diffs and the changed-file listing
/// reproducible, and the set is what keeps `--all` over a large checkout from
/// being quadratic in the number of tracked sources.
fn deduplicate(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            result.push(path);
        }
    }
    result
}

fn display_path(path: &Path, root: Option<&Path>) -> PathBuf {
    root.and_then(|root| path.strip_prefix(root).ok())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
}

fn resolve_context_paths(
    context_paths: &[ContextPath],
    root: Option<&Path>,
    cwd: &Path,
) -> Result<Vec<PathBuf>, WorkflowError> {
    context_paths
        .iter()
        .map(|context_path| {
            let path = &context_path.path;
            let candidate = if path.is_absolute() {
                path.clone()
            } else {
                context_path
                    .base
                    .as_deref()
                    .or(root)
                    .unwrap_or(cwd)
                    .join(path)
            };
            let resolved = fs::canonicalize(&candidate).map_err(|error| {
                WorkflowError::Usage(format!(
                    "--context-path does not exist: {} ({error})",
                    candidate.display()
                ))
            })?;
            if !fs::metadata(&resolved)?.is_dir() {
                return Err(WorkflowError::Usage(format!(
                    "--context-path requires a directory: {}",
                    candidate.display()
                )));
            }
            if let Some(root) = root {
                if !resolved.starts_with(root) {
                    return Err(WorkflowError::Usage(format!(
                        "--context-path must be inside the Git checkout: {}",
                        candidate.display()
                    )));
                }
            }
            Ok(resolved)
        })
        .collect()
}

fn filesystem_sources(
    context_paths: &[PathBuf],
    exclude_matcher: &ExcludeMatcher,
) -> io::Result<Vec<PathBuf>> {
    fn visit(directory: &Path, sources: &mut Vec<PathBuf>) -> io::Result<()> {
        let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(&path, sources)?;
            } else if file_type.is_file() && validate_extension(&path).is_ok() {
                sources.push(path);
            }
        }
        Ok(())
    }

    let mut sources = Vec::new();
    for context_path in context_paths {
        visit(context_path, &mut sources)?;
    }
    sources.retain(|path| {
        !context_paths
            .iter()
            .any(|root| exclude_matcher.is_excluded(root, path))
    });
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn context_sources(
    tracked: Option<&Vec<PathBuf>>,
    context_paths: &[PathBuf],
    exclude_matcher: &ExcludeMatcher,
    exclusion_root: &Path,
) -> Result<Option<Vec<PathBuf>>, WorkflowError> {
    let sources = if let Some(tracked) = tracked {
        tracked
            .iter()
            .filter(|path| {
                context_paths.is_empty()
                    || context_paths
                        .iter()
                        .any(|context_path| path.starts_with(context_path))
            })
            .cloned()
            .collect::<Vec<_>>()
    } else if !context_paths.is_empty() {
        filesystem_sources(context_paths, exclude_matcher)?
    } else {
        return Ok(None);
    };
    if tracked.is_some() {
        Ok(Some(
            sources
                .into_iter()
                .filter(|path| !exclude_matcher.is_excluded(exclusion_root, path))
                .collect(),
        ))
    } else {
        Ok(Some(sources))
    }
}

fn analyze_sources(
    sources: &[Source],
    indices: &[usize],
) -> Result<Vec<Option<FileFacts>>, WorkflowError> {
    let mut facts = (0..sources.len()).map(|_| None).collect::<Vec<_>>();
    for &index in indices {
        facts[index] = Some(analyze_file(&sources[index].bytes)?);
    }
    Ok(facts)
}

fn project_context(
    sources: &[Source],
    indices: &[usize],
    facts: &[Option<FileFacts>],
    stdin_source: Option<(&Path, &FileFacts)>,
    config: &crate::config::FormatConfig,
) -> ProjectContext {
    // Source facts are extracted once for the invocation, then reused both to
    // build project tables and as target-local precedence data during format.
    let mut context = ProjectContext::empty();
    for &index in indices {
        context.absorb(
            &sources[index].path,
            facts[index]
                .as_ref()
                .expect("every project source must have precomputed facts"),
        );
    }
    // A file-valued --project-context makes stdin the current version of that
    // tracked source. Its already-extracted facts replace the stale disk copy.
    if let Some((path, local)) = stdin_source {
        context.absorb(path, local);
    }
    context.define(&config.defines);
    context.enable_target_local_component_resolution();
    context
}

fn isolated_context(config: &crate::config::FormatConfig) -> ProjectContext {
    let mut context = ProjectContext::empty();
    context.define(&config.defines);
    context
}

fn format_one(
    source: &Source,
    local: Option<&FileFacts>,
    context: &ProjectContext,
    config: &crate::config::FormatConfig,
) -> Result<FormatResult, WorkflowError> {
    let result = if config.mode == FormatMode::IndentOnly {
        format_source(&source.bytes, config)?
    } else {
        crate::format::full::format_with_context_and_local(
            &source.bytes,
            context,
            local.expect("every full-mode target must have precomputed facts"),
            config,
        )?
    };
    Ok(result)
}

/// One formatted target: its metadata, and its bytes only if they differ from
/// what was read.
type FormattedTarget = (crate::FormatMeta, Option<Vec<u8>>);

/// Format every target, in parallel, and return one entry per target in target
/// order.
///
/// Formatting is pure once the single project-analysis pass has run, so the
/// targets are independent. Two things bound the cost of that independence:
///
/// * the worker count is `available_parallelism()`, not one thread per file —
///   `--all` over a large repository would otherwise ask the OS for thousands
///   of threads at once;
/// * a target whose output equals its input contributes only its metadata, so
///   an already-formatted tree does not hold a second copy of itself in memory.
///
/// Every target is formatted before the caller writes anything, which is what
/// keeps a failure part-way through from leaving a half-rewritten tree.
fn format_targets(
    sources: &[Source],
    target_indices: &[usize],
    facts: &[Option<FileFacts>],
    context: &ProjectContext,
    config: &crate::config::FormatConfig,
) -> Result<Vec<FormattedTarget>, WorkflowError> {
    let workers = std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1)
        .min(target_indices.len().max(1));
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let mut slots: Vec<Option<Result<FormattedTarget, WorkflowError>>> =
        (0..target_indices.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(&source_index) = target_indices.get(index) else {
                    return;
                };
                let target = &sources[source_index];
                let outcome = format_one(target, facts[source_index].as_ref(), context, config)
                    .map(|result| {
                        let changed = result.bytes != target.bytes;
                        (result.meta, changed.then_some(result.bytes))
                    });
                if sender.send((index, outcome)).is_err() {
                    return;
                }
            });
        }
        // The workers hold the only remaining senders, so the receiver ends
        // exactly when the last one finishes.
        drop(sender);
        for (index, outcome) in receiver {
            slots[index] = Some(outcome);
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.expect("format worker panicked"))
        .collect()
}

/// Keep routine formatting invocations useful when a generated source has
/// hundreds of equally-unwrappable statements.  Five concrete locations are
/// enough to identify the condition; the remainder are counted below.
const DECLINE_DIAGNOSTIC_LIMIT: usize = 5;

/// Bounded declined-wrap diagnostics for one CLI invocation.
///
/// The formatter deliberately keeps paths out of [`crate::FormatMeta`] so the
/// library API remains about a source buffer.  The CLI has the path, and adds
/// it here while combining diagnostics from all formatted targets.
#[derive(Default)]
struct DeclineReporter {
    reported: usize,
    suppressed: usize,
    suppressed_inputs: HashSet<String>,
    suppressed_stdin: bool,
}

impl DeclineReporter {
    fn report_fixed(&mut self, path: &Path, root: Option<&Path>) {
        let input = display_path(path, root).display().to_string();
        eprintln!("{}", fixed_message(&input));
    }

    fn report(&mut self, meta: &crate::FormatMeta, path: Option<&Path>, root: Option<&Path>) {
        let input = path
            .map(|path| display_path(path, root).display().to_string())
            .unwrap_or_else(|| "<stdin>".to_owned());
        for (line, reason) in &meta.declines {
            if self.reported < DECLINE_DIAGNOSTIC_LIMIT {
                eprintln!("{}", decline_message(&input, *line, *reason));
                self.reported += 1;
            } else {
                self.suppressed += 1;
                self.suppressed_stdin |= path.is_none();
                self.suppressed_inputs.insert(input.clone());
            }
        }
    }

    fn finish(&self) {
        if let Some(message) = self.summary() {
            eprintln!("{message}");
        }
    }

    fn summary(&self) -> Option<String> {
        (self.suppressed > 0).then(|| {
            let inputs = self.suppressed_inputs.len();
            let input_word = if self.suppressed_stdin {
                if inputs == 1 {
                    "input"
                } else {
                    "inputs"
                }
            } else if inputs == 1 {
                "file"
            } else {
                "files"
            };
            format!(
                "forformat: + {} additional declined-wrap diagnostics in {inputs} {input_word}",
                self.suppressed
            )
        })
    }
}

fn decline_message(input: &str, line: usize, reason: crate::format::wrapping::Decline) -> String {
    format!("forformat: {input}:{}: declined wrap: {reason:?}", line + 1)
}

fn fixed_message(input: &str) -> String {
    format!("forformat: {input}: fixed-form source, skipped")
}

/// Should this unnamed buffer be declined as fixed form?
///
/// Two carve-outs beyond the `-ifree` override. A buffer with no non-blank byte
/// has nothing to protect, and findent's detector answers FIXED at EOF, so
/// without this every content-free invocation — `forformat </dev/null` among
/// them — would report a skip. And `-lastindent`/`-lastusable` only report on
/// the source rather than rewriting it, so there is nothing to decline.
fn skips_fixed_form(invocation: &Invocation, input_path: Option<&Path>, source: &[u8]) -> bool {
    !invocation.force_free_input
        && !invocation.config.last_indent
        && !invocation.config.last_usable
        && source.iter().any(|byte| !byte.is_ascii_whitespace())
        && input_path.map_or_else(
            || crate::source::detect(source),
            |path| crate::source::detect_path(path, source),
        ) == SourceForm::Fixed
}

fn write_all_stdout(bytes: &[u8]) -> Result<(), WorkflowError> {
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    stdout.write_all(bytes)?;
    stdout.flush()?;
    Ok(())
}

/// Replace a file atomically, preserving its mode bits. Symlink resolution is
/// done by the caller, so a symlink argument leaves the link intact.
pub fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), WorkflowError> {
    let target = fs::canonicalize(path)?;
    let metadata = fs::metadata(&target)?;
    let mode = mode_bits(&metadata);
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        ".{}.forformat-{}-{}",
        target.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        number
    );
    let temporary = target.parent().unwrap_or_else(|| Path::new(".")).join(name);
    let result = (|| -> Result<(), WorkflowError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        set_mode(&file, mode)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn mode_bits(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn mode_bits(_metadata: &fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn set_mode(file: &File, mode: u32) -> Result<(), WorkflowError> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: u32) -> Result<(), WorkflowError> {
    Ok(())
}

fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&bytes[start..=index]);
            start = index + 1;
        }
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

const DIFF_CONTEXT_LINES: usize = 3;

fn append_diff_line(output: &mut Vec<u8>, marker: u8, line: &[u8]) {
    output.push(marker);
    output.extend_from_slice(line);
    if !line.ends_with(b"\n") {
        output.extend_from_slice(b"\n\\ No newline at end of file\n");
    }
}

fn hunk_line_number(start: usize, count: usize) -> usize {
    if count == 0 {
        start
    } else {
        start + 1
    }
}

fn unified_diff(path: &Path, old: &[u8], new: &[u8], root: Option<&Path>) -> Vec<u8> {
    if old == new {
        return Vec::new();
    }

    let relative = display_path(path, root).display().to_string();
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let common_limit = old_lines.len().min(new_lines.len());

    let mut common_prefix = 0usize;
    while common_prefix < common_limit && old_lines[common_prefix] == new_lines[common_prefix] {
        common_prefix += 1;
    }

    let mut common_suffix = 0usize;
    while common_suffix < common_limit - common_prefix
        && old_lines[old_lines.len() - 1 - common_suffix]
            == new_lines[new_lines.len() - 1 - common_suffix]
    {
        common_suffix += 1;
    }

    let context_before = common_prefix.min(DIFF_CONTEXT_LINES);
    let context_after = common_suffix.min(DIFF_CONTEXT_LINES);
    let old_change_end = old_lines.len() - common_suffix;
    let new_change_end = new_lines.len() - common_suffix;
    let old_start = common_prefix - context_before;
    let new_start = common_prefix - context_before;
    let old_end = old_change_end + context_after;
    let new_end = new_change_end + context_after;
    let old_count = old_end - old_start;
    let new_count = new_end - new_start;

    let mut output = Vec::new();
    output.extend_from_slice(format!("--- a/{relative}\n+++ b/{relative}\n").as_bytes());
    output.extend_from_slice(
        format!(
            "@@ -{},{} +{},{} @@\n",
            hunk_line_number(old_start, old_count),
            old_count,
            hunk_line_number(new_start, new_count),
            new_count,
        )
        .as_bytes(),
    );

    for line in &old_lines[old_start..common_prefix] {
        append_diff_line(&mut output, b' ', line);
    }
    for line in &old_lines[common_prefix..old_change_end] {
        append_diff_line(&mut output, b'-', line);
    }
    for line in &new_lines[common_prefix..new_change_end] {
        append_diff_line(&mut output, b'+', line);
    }
    for line in &old_lines[old_change_end..old_end] {
        append_diff_line(&mut output, b' ', line);
    }
    output
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
pub fn execute(invocation: Invocation) -> Result<i32, WorkflowError> {
    let invocation = promote_directory_argument(invocation);
    if invocation.query_format {
        return execute_query_format(invocation);
    }
    let all_selection = invocation.all || invocation.all_files;
    let stdin_mode = invocation.stdin || (invocation.paths.is_empty() && !all_selection);
    if stdin_mode && invocation.project_context.is_none() && invocation.context_paths.is_empty() {
        let mut source = Vec::new();
        io::stdin().read_to_end(&mut source)?;
        // stdin is the primary documented route, so it needs the same guard the
        // file routes get: free-form normalization of a fixed-form source
        // rewrites column-1 `*`/`C` comment markers as operators and destroys
        // the file.  There is no path to name in the diagnostic here.
        if skips_fixed_form(&invocation, None, &source) {
            eprintln!("{}", fixed_message("<stdin>"));
            write_all_stdout(&source)?;
            return Ok(0);
        }
        let config = invocation.config;
        let result = format_source(&source, &config)?;
        let mut declines = DeclineReporter::default();
        declines.report(&result.meta, None, None);
        declines.finish();
        write_all_stdout(&result.bytes)?;
        return Ok(0);
    }

    let profile = env::var_os("FORFORMAT_PROFILE_IO").is_some();
    let profile_start = Instant::now();
    let cwd = env::current_dir()?;
    let stdin_source = if stdin_mode {
        let mut source = Vec::new();
        io::stdin().read_to_end(&mut source)?;
        Some(source)
    } else {
        None
    };
    // A directory-valued project context describes an anonymous stdin buffer.
    // A file-valued context additionally identifies the tracked file whose
    // in-memory contents stdin replaces, so its stale on-disk bytes must not
    // contribute to project analysis.
    let project_scope = invocation
        .project_context
        .as_deref()
        .map(|path| {
            let candidate = resolve_input(path, None);
            let canonical = fs::canonicalize(&candidate).map_err(|error| {
                WorkflowError::Usage(format!(
                    "--project-context path does not exist: {} ({error})",
                    candidate.display()
                ))
            })?;
            let metadata = fs::metadata(&canonical)?;
            if metadata.is_dir() {
                return Ok((canonical, None));
            }
            if !metadata.is_file() {
                return Err(WorkflowError::Usage(format!(
                    "--project-context requires a directory or regular source file: {}",
                    candidate.display()
                )));
            }
            validate_extension(&candidate).map_err(WorkflowError::Usage)?;
            let parent = candidate
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let directory = fs::canonicalize(parent)?;
            let stdin_path = directory.join(
                candidate
                    .file_name()
                    .expect("a regular file must have a file name"),
            );
            Ok((directory, Some(stdin_path)))
        })
        .transpose()?;
    let all_scope = if all_selection {
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
    } else if let Some((scope, _)) = project_scope.as_ref() {
        repository_root(scope)?
    } else {
        repository_root(&cwd)?
    };
    if invocation.project_context.is_some() && root.is_none() {
        return Err(WorkflowError::Usage(
            "--project-context requires a valid Git checkout".into(),
        ));
    }
    let context_paths = if invocation.context_paths.is_empty() {
        Vec::new()
    } else {
        resolve_context_paths(&invocation.context_paths, root.as_deref(), &cwd)?
    };
    if stdin_mode {
        let source = stdin_source
            .as_deref()
            .expect("stdin mode must have read stdin");
        let input_path = project_scope.as_ref().and_then(|(_, path)| path.as_deref());
        if skips_fixed_form(&invocation, input_path, source) {
            let input = input_path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<stdin>".to_string());
            eprintln!("{}", fixed_message(&input));
            write_all_stdout(source)?;
            return Ok(0);
        }
        if invocation.config.mode == FormatMode::IndentOnly {
            let formatted = format_source(source, &invocation.config)?;
            let mut declines = DeclineReporter::default();
            declines.report(&formatted.meta, None, root.as_deref());
            declines.finish();
            write_all_stdout(&formatted.bytes)?;
            return Ok(0);
        }
    }
    let exclude_matcher = ExcludeMatcher::new(&invocation.exclude_patterns());
    let tracked_source_reader = if invocation.no_submodules {
        tracked_sources_without_submodules
    } else {
        tracked_sources
    };
    let tracked = if all_selection
        || invocation.project_context.is_some()
        || (!invocation.isolated && root.is_some())
    {
        root.as_deref().map(tracked_source_reader).transpose()?
    } else {
        None
    };
    let exclusion_root = root.as_deref().unwrap_or(cwd.as_path());
    let context_tracked = context_sources(
        tracked.as_ref(),
        &context_paths,
        &exclude_matcher,
        exclusion_root,
    )?;
    let tracked = tracked.map(|paths| {
        paths
            .into_iter()
            .filter(|path| !exclude_matcher.is_excluded(exclusion_root, path))
            .collect::<Vec<_>>()
    });
    let mut target_paths = if invocation.all {
        let tracked = tracked
            .as_ref()
            .ok_or_else(|| WorkflowError::Usage("--all requires a valid Git checkout".into()))?;
        match all_scope.as_deref() {
            Some(scope) => tracked
                .iter()
                .filter(|path| path.starts_with(scope))
                .cloned()
                .collect(),
            None => tracked.clone(),
        }
    } else if invocation.all_files {
        let tracked = root
            .as_deref()
            .map(tracked_sources_without_submodules)
            .transpose()?
            .ok_or_else(|| {
                WorkflowError::Usage("--all-files requires a valid Git checkout".into())
            })?;
        let tracked = tracked
            .into_iter()
            .filter(|path| !exclude_matcher.is_excluded(root.as_deref().unwrap(), path));
        match all_scope.as_deref() {
            Some(scope) => tracked.filter(|path| path.starts_with(scope)).collect(),
            None => tracked.collect(),
        }
    } else if stdin_mode {
        Vec::new()
    } else {
        deduplicate(
            invocation
                .paths
                .iter()
                .map(|path| resolve_input(path, root.as_deref()))
                .collect::<Vec<_>>(),
        )
    };
    if invocation.show_files {
        for path in &target_paths {
            println!("{}", display_path(path, root.as_deref()).display());
        }
        return Ok(0);
    }
    let mut project_paths = if invocation.isolated {
        // Isolated means no project tables at all. The target is still read
        // and formatted, but its declarations remain local to the formatter,
        // exactly as they are for stdin.
        Vec::new()
    } else if let Some((_, stdin_path)) = project_scope.as_ref() {
        context_tracked
            .as_ref()
            .expect("project-context requires tracked sources")
            .iter()
            .filter(|path| stdin_path.as_ref() != Some(*path))
            .cloned()
            .collect()
    } else if let Some(context_tracked) = context_tracked.as_ref() {
        if context_paths.is_empty() {
            deduplicate(
                context_tracked
                    .iter()
                    .cloned()
                    .chain(target_paths.iter().cloned())
                    .collect::<Vec<_>>(),
            )
        } else {
            context_tracked.clone()
        }
    } else {
        target_paths.clone()
    };
    let all_paths = deduplicate(
        target_paths
            .iter()
            .cloned()
            .chain(project_paths.iter().cloned())
            .collect::<Vec<_>>(),
    );
    if profile {
        eprintln!(
            "forformat profile: discovery={:?} targets={} project={} loaded-set={}",
            profile_start.elapsed(),
            target_paths.len(),
            project_paths.len(),
            all_paths.len()
        );
    }
    // Read each selected source once. The same in-memory bytes serve both the
    // target formatter and the single project-analysis pass.
    let explicit_targets: HashSet<&Path> = if all_selection || stdin_mode {
        HashSet::new()
    } else {
        target_paths.iter().map(PathBuf::as_path).collect()
    };
    let mut loaded = Vec::with_capacity(all_paths.len());
    let mut loaded_index = HashMap::with_capacity(all_paths.len());
    for path in &all_paths {
        match read_source(path, invocation.force_free_input)? {
            Some(source) => {
                loaded_index.insert(path.clone(), loaded.len());
                loaded.push(source);
            }
            None if explicit_targets.contains(path.as_path()) => {
                return Err(WorkflowError::Usage(format!(
                    "Fortran source file does not exist: {}",
                    path.display()
                )));
            }
            None => {}
        }
    }
    target_paths.retain(|path| loaded_index.contains_key(path));
    project_paths.retain(|path| {
        loaded_index.contains_key(path) && loaded[loaded_index[path]].form == SourceForm::Free
    });
    if profile {
        eprintln!(
            "forformat profile: read={:?} sources={}",
            profile_start.elapsed(),
            loaded.len()
        );
    }
    let target_indices: Vec<usize> = target_paths
        .iter()
        .filter_map(|path| {
            let index = loaded_index[path];
            (loaded[index].form == SourceForm::Free).then_some(index)
        })
        .collect();
    let project_indices: Vec<usize> = project_paths
        .iter()
        .map(|path| loaded_index[path])
        .collect();

    let mut analysis_needed = vec![false; loaded.len()];
    if invocation.config.mode != FormatMode::IndentOnly {
        for &index in target_indices.iter().chain(project_indices.iter()) {
            analysis_needed[index] = true;
        }
    }
    let analysis_indices = analysis_needed
        .iter()
        .enumerate()
        .filter_map(|(index, needed)| (*needed).then_some(index))
        .collect::<Vec<_>>();
    let facts = analyze_sources(&loaded, &analysis_indices)?;
    let stdin_local = if stdin_mode && invocation.config.mode != FormatMode::IndentOnly {
        Some(analyze_file(
            stdin_source
                .as_deref()
                .expect("stdin mode must have read stdin"),
        )?)
    } else {
        None
    };

    let context = if invocation.isolated || invocation.config.mode == FormatMode::IndentOnly {
        isolated_context(&invocation.config)
    } else {
        let stdin_project_source = project_scope
            .as_ref()
            .and_then(|(_, path)| path.as_deref())
            .filter(|path| {
                context_paths.is_empty()
                    || context_paths
                        .iter()
                        .any(|context_path| path.starts_with(context_path))
            })
            .zip(stdin_local.as_ref());
        project_context(
            &loaded,
            &project_indices,
            &facts,
            stdin_project_source,
            &invocation.config,
        )
    };
    if profile {
        eprintln!("forformat profile: analysis={:?}", profile_start.elapsed(),);
    }

    if stdin_mode {
        let source = stdin_source
            .as_deref()
            .expect("stdin mode must have read stdin");
        let formatted = if invocation.config.mode == FormatMode::IndentOnly {
            format_source(source, &invocation.config)?
        } else {
            crate::format::full::format_with_context_and_local(
                source,
                &context,
                stdin_local
                    .as_ref()
                    .expect("full-mode stdin must have precomputed facts"),
                &invocation.config,
            )?
        };
        let mut declines = DeclineReporter::default();
        declines.report(&formatted.meta, None, root.as_deref());
        declines.finish();
        write_all_stdout(&formatted.bytes)?;
        return Ok(0);
    }

    if invocation.stdout {
        let mut declines = DeclineReporter::default();
        let source_index = loaded_index[&target_paths[0]];
        if loaded[source_index].form == SourceForm::Fixed {
            declines.report_fixed(&target_paths[0], root.as_deref());
            write_all_stdout(&loaded[source_index].bytes)?;
        } else {
            let formatted = format_one(
                &loaded[source_index],
                facts[source_index].as_ref(),
                &context,
                &invocation.config,
            )?;
            declines.report(&formatted.meta, Some(&target_paths[0]), root.as_deref());
            write_all_stdout(&formatted.bytes)?;
        }
        declines.finish();
        return Ok(0);
    }

    let formatting_start = Instant::now();
    let formatted = format_targets(
        &loaded,
        &target_indices,
        &facts,
        &context,
        &invocation.config,
    )?;
    let mut changed = Vec::new();
    let mut declines = DeclineReporter::default();
    let mut formatted = formatted.into_iter();
    for path in &target_paths {
        let source_index = loaded_index[path];
        let target = &loaded[source_index];
        if target.form == SourceForm::Fixed {
            declines.report_fixed(path, root.as_deref());
            continue;
        }
        let (meta, output) = formatted
            .next()
            .expect("one formatting result per free-form target");
        declines.report(&meta, Some(path), root.as_deref());
        let Some(formatted) = output else {
            continue;
        };
        changed.push(path.clone());
        if invocation.diff {
            write_all_stdout(&unified_diff(
                path,
                &target.bytes,
                &formatted,
                root.as_deref(),
            ))?;
        } else if !invocation.check {
            atomic_replace(path, &formatted)?;
        }
    }
    declines.finish();
    if !invocation.diff {
        for path in &changed {
            println!("{}", display_path(path, root.as_deref()).display());
        }
    }
    if profile {
        eprintln!(
            "forformat profile: formatting={:?} total={:?} changed={}",
            formatting_start.elapsed(),
            profile_start.elapsed(),
            changed.len()
        );
    }
    Ok(i32::from(
        (invocation.check || invocation.diff) && !changed.is_empty(),
    ))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::atomic_replace;
    use super::{
        decline_message, project_context, unified_diff, validate_extension, DeclineReporter,
    };
    use crate::{analysis::names::NameSpace, config::FormatConfig, format::wrapping::Decline};
    #[cfg(unix)]
    use std::fs;
    use std::path::Path;

    #[test]
    fn section_9_1_valid_extension_is_pure_and_accepts_missing_path() {
        assert!(validate_extension(Path::new("does-not-exist.F90")).is_ok());
        assert!(validate_extension(Path::new("does-not-exist.txt")).is_err());
    }

    #[test]
    fn unified_diff_marks_missing_final_newlines() {
        let diff = unified_diff(Path::new("source.f90"), b"a\nold", b"a\nnew", None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -1,2 +1,2 @@\n a\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn unified_diff_reports_a_newline_only_change() {
        let diff = unified_diff(Path::new("source.f90"), b"same", b"same\n", None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -1,1 +1,1 @@\n-same\n\\ No newline at end of file\n+same\n"
        );
    }

    #[test]
    fn unified_diff_trims_unchanged_file_ends() {
        let old = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        let new = b"1\n2\n3\n4\n5\n6\n7\nchanged\n9\n10\n11\n12\n13\n14\n15\n";
        let diff = unified_diff(Path::new("source.f90"), old, new, None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -5,7 +5,7 @@\n 5\n 6\n 7\n-8\n+changed\n 9\n 10\n 11\n"
        );
    }

    #[test]
    fn declined_wrap_diagnostics_include_the_input_and_bound_the_summary() {
        assert_eq!(
            decline_message("src/example.f90", 41, Decline::NoSafeBreak),
            "forformat: src/example.f90:42: declined wrap: NoSafeBreak"
        );

        let mut reporter = DeclineReporter {
            suppressed: 7,
            ..Default::default()
        };
        reporter
            .suppressed_inputs
            .insert("src/example.f90".to_owned());
        reporter
            .suppressed_inputs
            .insert("src/another.f90".to_owned());
        assert_eq!(
            reporter.summary().as_deref(),
            Some("forformat: + 7 additional declined-wrap diagnostics in 2 files")
        );

        let mut stdin_reporter = DeclineReporter {
            suppressed: 1,
            suppressed_stdin: true,
            ..Default::default()
        };
        stdin_reporter
            .suppressed_inputs
            .insert("<stdin>".to_owned());
        assert_eq!(
            stdin_reporter.summary().as_deref(),
            Some("forformat: + 1 additional declined-wrap diagnostics in 1 input")
        );
    }

    #[test]
    fn stdin_replacement_is_present_in_the_project_tables() {
        let replacement = b"module CurrentName\nend module CurrentName\n";
        let replacement_facts = crate::analysis::analyze_file(replacement).unwrap();
        let context = project_context(
            &[],
            &[],
            &[],
            Some((Path::new("target.f90"), &replacement_facts)),
            &FormatConfig::default(),
        );
        let local = crate::analysis::analyze_file(b"program p\nend program p\n").unwrap();
        assert_eq!(
            context
                .resolver(&local)
                .spelling(NameSpace::Module, b"currentname"),
            Some(b"CurrentName".as_slice())
        );
        assert_eq!(context.sources, vec![Path::new("target.f90")]);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replace_preserves_mode_and_cleans_failed_temporary_write() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("forformat-io-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let path = directory.join("source.f90");
        fs::write(&path, b"old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        atomic_replace(&path, b"new\n").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read(&path).unwrap(), b"new\n");
        let failure = atomic_replace(&directory, b"nope\n");
        assert!(failure.is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        let _ = fs::remove_dir_all(directory);
    }
}
