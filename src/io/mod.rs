//! File and project workflow for the command-line formatter.
//!
//! This module deliberately contains no formatting rules. It selects sources,
//! builds one project context, delegates bytes to the library formatter, and
//! performs the requested output operation.

use crate::{
    analysis::{analyze_project, ProjectContext},
    cli::Invocation,
    config::FormatMode,
    error::FormatError,
    format_source, format_source_with_context, FormatResult,
};
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
            "expected a free-form Fortran source (.f03, .f08, .f18, .f23, .f90, .f95): {}",
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

/// Find the checkout containing start.
pub fn repository_root(start: &Path) -> io::Result<Option<PathBuf>> {
    let output = git(&["rev-parse", "--show-toplevel"], start)?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let root = text.trim();
    if root.is_empty() {
        Ok(None)
    } else {
        Ok(Some(fs::canonicalize(root)?))
    }
}

/// Return tracked free-form sources, accepting upper-case suffix spellings.
pub fn tracked_sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    let output = git(&["ls-files", "--recurse-submodules", "-z", "--"], root)?;
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
        let relative = PathBuf::from(OsString::from(String::from_utf8_lossy(raw).into_owned()));
        if validate_extension(&relative).is_ok() {
            paths.push(root.join(relative));
        }
    }
    paths.sort();
    Ok(paths)
}

#[derive(Debug, Clone)]
struct Source {
    path: PathBuf,
    bytes: Vec<u8>,
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

fn read_source(path: &Path, root: Option<&Path>) -> Result<Source, WorkflowError> {
    validate_extension(path).map_err(WorkflowError::Usage)?;
    let input = resolve_input(path, root);
    let canonical = fs::canonicalize(&input).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            WorkflowError::Usage(format!(
                "Fortran source file does not exist: {}",
                input.display()
            ))
        } else {
            WorkflowError::Io(error)
        }
    })?;
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_file() {
        return Err(WorkflowError::Usage(format!(
            "Fortran source file is not a regular file: {}",
            input.display()
        )));
    }
    Ok(Source {
        bytes: fs::read(&canonical)?,
        path: canonical,
    })
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

fn project_context(
    sources: &[Source],
    config: &crate::config::FormatConfig,
) -> Result<ProjectContext, WorkflowError> {
    // The caller has already read every source. This is the one project
    // analysis pass for the invocation; formatting receives the resulting
    // context rather than rebuilding it per target.
    let mut context = analyze_project(
        sources
            .iter()
            .map(|source| (source.path.as_path(), source.bytes.as_slice())),
    )?;
    context.define(&config.defines);
    context.enable_target_local_component_resolution();
    Ok(context)
}

fn isolated_context(config: &crate::config::FormatConfig) -> ProjectContext {
    let mut context = ProjectContext::empty();
    context.define(&config.defines);
    context
}

fn format_one(
    source: &Source,
    context: &ProjectContext,
    config: &crate::config::FormatConfig,
) -> Result<FormatResult, WorkflowError> {
    let result = if config.mode == FormatMode::IndentOnly {
        format_source(&source.bytes, config)?
    } else {
        format_source_with_context(&source.bytes, context, config)?
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
    targets: &[Source],
    context: &ProjectContext,
    config: &crate::config::FormatConfig,
) -> Result<Vec<FormattedTarget>, WorkflowError> {
    let workers = std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1)
        .min(targets.len().max(1));
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let mut slots: Vec<Option<Result<FormattedTarget, WorkflowError>>> =
        (0..targets.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(target) = targets.get(index) else {
                    return;
                };
                let outcome = format_one(target, context, config).map(|result| {
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

fn report_declines(meta: &crate::FormatMeta) {
    for (line, reason) in &meta.declines {
        eprintln!("forformat: declined wrap at line {}: {reason:?}", line + 1);
    }
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

fn unified_diff(path: &Path, old: &[u8], new: &[u8], root: Option<&Path>) -> Vec<u8> {
    let relative = display_path(path, root).display().to_string();
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let mut output = Vec::new();
    output.extend_from_slice(format!("--- a/{relative}\n+++ b/{relative}\n").as_bytes());
    output.extend_from_slice(
        format!("@@ -1,{} +1,{} @@\n", old_lines.len(), new_lines.len()).as_bytes(),
    );
    for line in old_lines {
        output.push(b'-');
        output.extend_from_slice(line);
        if !line.ends_with(b"\n") {
            output.push(b'\n');
        }
    }
    for line in new_lines {
        output.push(b'+');
        output.extend_from_slice(line);
        if !line.ends_with(b"\n") {
            output.push(b'\n');
        }
    }
    output
}

/// Execute one parsed invocation. Return value is the process status for a
/// successful operation: 0 clean/success, 1 differences found.
pub fn execute(invocation: Invocation) -> Result<i32, WorkflowError> {
    if invocation.stdin || (invocation.paths.is_empty() && !invocation.all) {
        let mut source = Vec::new();
        io::stdin().read_to_end(&mut source)?;
        let config = invocation.config;
        let result = format_source(&source, &config)?;
        report_declines(&result.meta);
        write_all_stdout(&result.bytes)?;
        return Ok(0);
    }

    let profile = env::var_os("FORFORMAT_PROFILE_IO").is_some();
    let profile_start = Instant::now();
    let cwd = env::current_dir()?;
    let all_scope = if invocation.all {
        invocation
            .paths
            .first()
            .map(|path| {
                let candidate = resolve_input(path, None);
                let canonical = fs::canonicalize(&candidate).map_err(|error| {
                    WorkflowError::Usage(format!(
                        "--all directory does not exist: {} ({error})",
                        candidate.display()
                    ))
                })?;
                if !fs::metadata(&canonical)?.is_dir() {
                    return Err(WorkflowError::Usage(format!(
                        "--all requires a directory: {}",
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
    let tracked = if invocation.all || (!invocation.isolated && root.is_some()) {
        root.as_deref().map(tracked_sources).transpose()?
    } else {
        None
    };
    let target_paths = if invocation.all {
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
    } else {
        deduplicate(
            invocation
                .paths
                .iter()
                .map(|path| resolve_input(path, root.as_deref()))
                .collect::<Vec<_>>(),
        )
    };
    let project_paths = if invocation.isolated {
        // Isolated means no project tables at all. The target is still read
        // and formatted, but its declarations remain local to the formatter,
        // exactly as they are for stdin.
        Vec::new()
    } else if let Some(tracked) = tracked.as_ref() {
        deduplicate(
            tracked
                .iter()
                .cloned()
                .chain(target_paths.iter().cloned())
                .collect::<Vec<_>>(),
        )
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
    let loaded: Vec<Source> = all_paths
        .iter()
        .map(|path| read_source(path, root.as_deref()))
        .collect::<Result<_, _>>()?;
    if profile {
        eprintln!(
            "forformat profile: read={:?} sources={}",
            profile_start.elapsed(),
            loaded.len()
        );
    }
    // `all_paths` is the deduplicated concatenation of the two path lists, so
    // every target and project path occurs in it verbatim and `loaded[i]` is
    // the source read for `all_paths[i]`.  One index over that correspondence
    // replaces a linear search — and a `canonicalize` syscall — per lookup.
    let loaded_index: HashMap<&Path, usize> = all_paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_path(), index))
        .collect();
    let source_for = |path: &Path| {
        let index = *loaded_index.get(path).expect("selected path was loaded");
        loaded[index].clone()
    };
    let targets: Vec<Source> = target_paths.iter().map(|path| source_for(path)).collect();
    let project_sources: Vec<Source> = project_paths.iter().map(|path| source_for(path)).collect();
    let context = if invocation.isolated {
        isolated_context(&invocation.config)
    } else {
        project_context(&project_sources, &invocation.config)?
    };
    if profile {
        eprintln!(
            "forformat profile: project-analysis={:?}",
            profile_start.elapsed(),
        );
    }

    if invocation.stdout {
        let formatted = format_one(&targets[0], &context, &invocation.config)?;
        report_declines(&formatted.meta);
        write_all_stdout(&formatted.bytes)?;
        return Ok(0);
    }

    let formatting_start = Instant::now();
    let formatted = format_targets(&targets, &context, &invocation.config)?;
    let mut changed = Vec::new();
    for ((target, path), (meta, output)) in targets.iter().zip(&target_paths).zip(formatted) {
        report_declines(&meta);
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
    use super::validate_extension;
    #[cfg(unix)]
    use std::fs;
    use std::path::Path;

    #[test]
    fn section_9_1_valid_extension_is_pure_and_accepts_missing_path() {
        assert!(validate_extension(Path::new("does-not-exist.F90")).is_ok());
        assert!(validate_extension(Path::new("does-not-exist.txt")).is_err());
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
        let _ = fs::remove_dir_all(&directory);
    }
}
