use super::{ContextPath, Invocation};
use crate::{config::FormatConfig, error::FormatError};
use std::path::PathBuf;

/// Mutable command state while argv is being consumed.
///
/// Parsing deliberately records first and validates afterwards: several of the
/// findent-compatible spellings are overloaded, and cross-option validity is a
/// property of the completed invocation rather than of any individual token.
#[derive(Debug, Default)]
pub(super) struct DraftInvocation {
    pub(super) config: FormatConfig,
    pub(super) paths: Vec<PathBuf>,
    pub(super) project_context: Option<PathBuf>,
    pub(super) context_paths: Vec<ContextPath>,
    pub(super) all: bool,
    pub(super) all_files: bool,
    pub(super) no_submodules: bool,
    pub(super) stdin: bool,
    pub(super) stdout: bool,
    pub(super) force_free_input: bool,
    pub(super) query_format: bool,
    pub(super) isolated: bool,
    pub(super) check: bool,
    pub(super) diff: bool,
    pub(super) show_files: bool,
    pub(super) exclude: Option<Vec<String>>,
    pub(super) extend_exclude: Vec<String>,
}

impl DraftInvocation {
    pub(super) fn validate(&self) -> Result<(), FormatError> {
        if self.project_context.is_some()
            && (!self.paths.is_empty()
                || self.all
                || self.all_files
                || self.stdout
                || self.isolated
                || self.check
                || self.diff
                || self.show_files)
        {
            return Err(FormatError::InvalidOption(
                "--project-context cannot be combined with paths, --all, --all-files, --stdout, --isolated, --check, --diff, or --show-files".into(),
            ));
        }
        if self.stdin
            && (self.all
                || self.all_files
                || !self.paths.is_empty()
                || self.stdout
                || self.isolated
                || self.check
                || self.diff
                || self.show_files)
        {
            return Err(FormatError::InvalidOption(
                "--stdin cannot be combined with paths, --all, --all-files, --stdout, --check, --diff, --show-files, or --isolated".into(),
            ));
        }
        if self.all && self.all_files {
            return Err(FormatError::InvalidOption(
                "--all and --all-files cannot be combined".into(),
            ));
        }
        if self.stdout
            && (self.paths.len() != 1
                || self.all
                || self.all_files
                || self.check
                || self.diff
                || self.show_files)
        {
            return Err(FormatError::InvalidOption(
                "--stdout requires exactly one path and cannot be combined with --all, --all-files, --check, --diff, or --show-files".into(),
            ));
        }
        if (self.all || self.all_files) && self.paths.len() > 1 {
            return Err(FormatError::InvalidOption(
                "--all and --all-files accept at most one directory path".into(),
            ));
        }
        if self.isolated && (self.all || self.all_files || self.paths.is_empty()) {
            return Err(FormatError::InvalidOption(
                "--isolated requires one or more explicit paths and cannot be combined with --all-files".into(),
            ));
        }
        if self.isolated && !self.context_paths.is_empty() {
            return Err(FormatError::InvalidOption(
                "--isolated cannot be combined with --context-path".into(),
            ));
        }
        if self.diff && self.paths.is_empty() && !self.all && !self.all_files {
            return Err(FormatError::InvalidOption(
                "--diff requires paths, --all, or --all-files".into(),
            ));
        }
        if self.check && self.paths.is_empty() && !self.all && !self.all_files {
            return Err(FormatError::InvalidOption(
                "--check requires paths, --all, or --all-files".into(),
            ));
        }
        if self.show_files && self.paths.is_empty() && !self.all && !self.all_files {
            return Err(FormatError::InvalidOption(
                "--show-files requires paths, --all, or --all-files".into(),
            ));
        }
        if self.show_files
            && (self.check || self.diff || self.config.last_indent || self.config.last_usable)
        {
            return Err(FormatError::InvalidOption(
                "--show-files cannot be combined with --check, --diff, or query modes".into(),
            ));
        }
        if self.query_format
            && (self.stdout
                || self.check
                || self.diff
                || self.show_files
                || self.config.last_indent
                || self.config.last_usable)
        {
            return Err(FormatError::InvalidOption(
                "--query-format cannot be combined with output, check, diff, or other query modes"
                    .into(),
            ));
        }
        if self.query_format
            && (self.project_context.is_some() || !self.context_paths.is_empty() || self.isolated)
        {
            return Err(FormatError::InvalidOption(
                "--query-format cannot be combined with --project-context, --context-path, or --isolated".into(),
            ));
        }
        // Rewrap repacks continuations through the reflow wrapper, and only
        // full mode runs it. Staying silent would make the flag a no-op that
        // looks like it worked; `--no-wrap` is different and stays inert,
        // because turning wrapping off inside full mode is a coherent policy
        // rather than a request the mode cannot answer.
        if self.config.rewrap && !self.config.mode.wraps() {
            return Err(FormatError::InvalidOption(
                "--rewrap requires full mode: --indent-only, --normalize-only, --canonicalize-only, and --canonicalize-and-indent do not run the wrapper".into(),
            ));
        }
        if (self.config.last_indent || self.config.last_usable)
            && (self.all || self.all_files || !self.paths.is_empty() || self.check || self.diff)
        {
            return Err(FormatError::InvalidOption(
                "-lastindent/-lastusable cannot be combined with path-update, --check, or --diff"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Invocation {
        if self.project_context.is_some() {
            self.stdin = true;
        }
        Invocation {
            config: self.config,
            paths: self.paths,
            project_context: self.project_context,
            context_paths: self.context_paths,
            all: self.all,
            all_files: self.all_files,
            no_submodules: self.no_submodules,
            stdin: self.stdin,
            stdout: self.stdout,
            force_free_input: self.force_free_input,
            query_format: self.query_format,
            isolated: self.isolated,
            check: self.check,
            diff: self.diff,
            show_files: self.show_files,
            exclude: self.exclude,
            extend_exclude: self.extend_exclude,
        }
    }
}
