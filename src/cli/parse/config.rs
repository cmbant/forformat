use super::Command;
use crate::error::FormatError;
use std::{
    fs,
    path::{Path, PathBuf},
};

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

fn stdin_filename_identity(path: &Path, cwd: &Path) -> PathBuf {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };

    // Match the successful identity rules used by `resolve_stdin_filename` in
    // the workflow layer. Existing files resolve as a whole path so symlink
    // aliases follow their target. New editor buffers keep their requested leaf
    // name under a canonical parent (which also resolves a symlinked parent).
    if let Ok(canonical) = fs::canonicalize(&candidate) {
        return canonical;
    }
    let Some(file_name) = candidate.file_name() else {
        return candidate;
    };
    let Some(parent) = candidate.parent() else {
        return candidate;
    };
    fs::canonicalize(parent)
        .map(|canonical_parent| canonical_parent.join(file_name))
        .unwrap_or(candidate)
}

pub(super) fn config_start(command: &Command, cwd: &Path) -> PathBuf {
    if let Command::Run(invocation) = command {
        // A virtual stdin filename is the input's identity, so configuration
        // follows that same resolved file identity even when project analysis
        // is explicitly rooted somewhere else with --project-context.
        if let Some(path) = invocation.stdin_filename.as_deref() {
            let candidate = stdin_filename_identity(path, cwd);
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

#[cfg(all(test, unix))]
mod tests {
    use super::stdin_filename_identity;
    use std::{
        fs,
        os::unix::fs::symlink,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn stdin_filename_config_identity_follows_a_symlink_target() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "forformat-stdin-config-symlink-{}-{unique}",
            std::process::id()
        ));
        let alias_dir = root.join("alias");
        let target_dir = root.join("other-repo/src");
        fs::create_dir_all(&alias_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("real.f90");
        fs::write(&target, b"program p\nend program p\n").unwrap();
        symlink(&target, alias_dir.join("buffer.f90")).unwrap();

        let identity = stdin_filename_identity(Path::new("alias/buffer.f90"), &root);
        assert_eq!(identity, fs::canonicalize(target).unwrap());

        let _ = fs::remove_dir_all(root);
    }
}
