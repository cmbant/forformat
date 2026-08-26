//! Selecting the sources one invocation will read.
//!
//! Everything here answers "which files", never "what do they say": the Git
//! queries, the filesystem walk, the path resolution that turns a command-line
//! argument into something openable, and the one read that produces a
//! [`Source`]. Keeping it apart from the workflow in [`super`] is what makes
//! the single `git` entry point below auditable.

use super::{exclude::ExcludeMatcher, WorkflowError};
use crate::{cli::ContextPath, source::SourceForm};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const GIT_HOOK_VARS: [&str; 4] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
];

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
            "expected a free-form Fortran source (suffix match is case-insensitive: .f, .f03, .f08, .f18, .f23, .f90, .f95, .fpp, .pf): {}",
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

pub(super) fn git_path(raw: &[u8]) -> io::Result<PathBuf> {
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

pub(super) fn tracked_sources_with_submodules(
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
pub(super) struct Source {
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) form: SourceForm,
}

pub(super) fn resolve_input(path: &Path, root: Option<&Path>) -> PathBuf {
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

pub(super) fn read_source(
    path: &Path,
    force_free_input: bool,
) -> Result<Option<Source>, WorkflowError> {
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
pub(super) fn deduplicate(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            result.push(path);
        }
    }
    result
}

pub(super) fn display_path(path: &Path, root: Option<&Path>) -> PathBuf {
    root.and_then(|root| path.strip_prefix(root).ok())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
}

pub(super) fn resolve_context_paths(
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

pub(super) fn filesystem_sources(
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

pub(super) fn context_sources(
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

#[cfg(test)]
mod tests {
    use super::validate_extension;
    use std::path::Path;

    #[test]
    fn section_9_1_valid_extension_is_pure_and_accepts_missing_path() {
        assert!(validate_extension(Path::new("does-not-exist.F90")).is_ok());
        assert!(validate_extension(Path::new("does-not-exist.txt")).is_err());
    }
}
