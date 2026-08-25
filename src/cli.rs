use crate::config::FormatConfig;
use std::path::PathBuf;

mod draft;
mod help;
pub(crate) mod options;
mod parse;
pub(crate) mod settings;

pub use help::usage;
pub use parse::parse;

#[cfg(test)]
use parse::parse_inner;

/// The `--version` line, taken from the package manifest so a version bump is
/// a one-line change in `Cargo.toml`.
pub const VERSION: &str = concat!("forformat ", env!("CARGO_PKG_VERSION"));

pub enum Command {
    Run(Box<Invocation>),
    Help,
    Version,
}

/// A context directory and the directory against which a relative path is
/// interpreted. `None` means the path came directly from the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPath {
    pub path: PathBuf,
    pub base: Option<PathBuf>,
}

/// Parsed command-line state. Formatting remains configured by
/// [`FormatConfig`]; file/project policy lives here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub config: FormatConfig,
    pub paths: Vec<PathBuf>,
    pub project_context: Option<PathBuf>,
    pub context_paths: Vec<ContextPath>,
    pub all: bool,
    pub all_files: bool,
    pub no_submodules: bool,
    pub stdin: bool,
    pub stdout: bool,
    pub force_free_input: bool,
    pub query_format: bool,
    pub isolated: bool,
    pub check: bool,
    pub diff: bool,
    pub show_files: bool,
    /// Patterns from `--exclude`, which *replaces* [`DEFAULT_EXCLUDES`] rather
    /// than adding to it. `None` means the option was never given.
    pub exclude: Option<Vec<String>>,
    /// Patterns from `--extend-exclude`, added to whichever set `exclude`
    /// selected.
    pub extend_exclude: Vec<String>,
}

/// Sources excluded when no `--exclude` is given.
///
/// This is empty on purpose. Ruff and black need opinionated defaults because
/// they walk the filesystem and would otherwise descend into `.venv` and
/// friends; forformat selects files with `git ls-files`, so a file only reaches
/// the formatter because someone chose to track it. Skipping a tracked source
/// by default would contradict what `--all` says it does.
///
/// The layering is still modelled, so a default added here would behave the way
/// the two options are documented: `--exclude` drops it, `--extend-exclude`
/// keeps it.
pub const DEFAULT_EXCLUDES: &[&str] = &[];

impl Invocation {
    /// The exclusion patterns actually in force.
    ///
    /// `--exclude` replaces the defaults and `--extend-exclude` adds to
    /// whichever set survived that, matching how ruff and black layer the same
    /// pair of options.
    pub fn exclude_patterns(&self) -> Vec<String> {
        let base = match self.exclude.as_deref() {
            Some(patterns) => patterns.to_vec(),
            None => DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect(),
        };
        base.into_iter()
            .chain(self.extend_exclude.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests;
