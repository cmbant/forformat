use super::Command;
use crate::error::FormatError;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(in crate::cli) struct ConfigSelection {
    pub(in crate::cli) no_config: bool,
    pub(in crate::cli) explicit: Vec<String>,
}

impl ConfigSelection {
    pub(in crate::cli) fn resolve(&self) -> Result<(bool, Option<PathBuf>), FormatError> {
        let mut explicit = None;
        for path in &self.explicit {
            if path.is_empty() || path.starts_with('-') {
                return Err(FormatError::InvalidOption(
                    "--config requires a path".to_string(),
                ));
            }
            if explicit.replace(PathBuf::from(path)).is_some() {
                return Err(FormatError::InvalidOption(
                    "--config may be specified only once".to_string(),
                ));
            }
        }
        if self.no_config && explicit.is_some() {
            return Err(FormatError::InvalidOption(
                "--config cannot be combined with --no-config".to_string(),
            ));
        }
        Ok((self.no_config, explicit))
    }
}

pub(super) fn config_start(command: &Command, cwd: &Path) -> PathBuf {
    if let Command::Run(invocation) = command {
        // A virtual stdin filename is the input's identity, so configuration
        // follows that file even when project analysis is explicitly rooted
        // somewhere else with --project-context.
        if let Some(path) = invocation.stdin_filename.as_deref() {
            let candidate = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            return candidate
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| cwd.to_path_buf());
        }
        if invocation.paths.len() == 1 {
            let candidate = if invocation.paths[0].is_absolute() {
                invocation.paths[0].clone()
            } else {
                cwd.join(&invocation.paths[0])
            };
            // A lone directory argument selects that directory's tracked
            // sources (see `promote_directory_argument` in io/mod.rs), so its
            // config discovery matches explicit `--all`/`--all-files DIR`.
            if candidate.is_dir() {
                return candidate;
            }
            if !invocation.all && !invocation.all_files && candidate.is_file() {
                return candidate
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| cwd.to_path_buf());
            }
        }
    }
    cwd.to_path_buf()
}
