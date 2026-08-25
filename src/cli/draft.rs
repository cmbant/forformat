use super::{
    options::OptionId,
    settings::{FormatSetting, OptionLayer},
    ContextPath, Invocation,
};
use crate::{config::FormatConfig, error::FormatError};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum InputSelection {
    #[default]
    Implicit,
    Stdin { project_context: Option<PathBuf> },
    ExplicitPaths { paths: Vec<PathBuf>, isolated: bool },
    All { directory: Option<PathBuf> },
    AllFiles { directory: Option<PathBuf> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndentQuery {
    LastIndent,
    LastUsable,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionRequest {
    Stdout,
    Check,
    Diff,
    ShowFiles,
    QueryFormat,
    LastIndent,
    LastUsable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Rewrite,
    Stdout,
    Check,
    Diff { check: bool },
    ShowFiles,
    QueryFormat,
    IndentQuery(IndentQuery),
}

/// Mutable command state while argv is being consumed.
///
/// Input selection and output action are enums rather than a matrix of
/// independent booleans. Parsing can therefore construct only one selection
/// and one action at a time; the remaining validation is about compatibility
/// between those typed choices and project context rather than rejecting
/// impossible boolean combinations after the fact.
#[derive(Debug, Default)]
pub(super) struct DraftInvocation {
    selection: InputSelection,
    actions: Vec<ActionRequest>,
    pub(super) options: OptionLayer,
}

impl DraftInvocation {
    pub(super) fn push_path(&mut self, path: PathBuf) -> Result<(), FormatError> {
        match &mut self.selection {
            InputSelection::Implicit => {
                self.selection = InputSelection::ExplicitPaths {
                    paths: vec![path],
                    isolated: false,
                };
            }
            InputSelection::Stdin { project_context } => {
                return Err(if project_context.is_some() {
                    project_context_conflict()
                } else {
                    stdin_conflict()
                });
            }
            InputSelection::ExplicitPaths { paths, .. } => paths.push(path),
            InputSelection::All { directory } | InputSelection::AllFiles { directory } => {
                if directory.replace(path).is_some() {
                    return Err(bulk_directory_conflict());
                }
            }
        }
        Ok(())
    }

    pub(super) fn select_stdin(&mut self) -> Result<(), FormatError> {
        match &self.selection {
            InputSelection::Implicit => {
                self.selection = InputSelection::Stdin {
                    project_context: None,
                };
                Ok(())
            }
            InputSelection::Stdin { .. } => Ok(()),
            InputSelection::ExplicitPaths { .. }
            | InputSelection::All { .. }
            | InputSelection::AllFiles { .. } => Err(stdin_conflict()),
        }
    }

    pub(super) fn select_project_context(&mut self, path: PathBuf) -> Result<(), FormatError> {
        match &mut self.selection {
            InputSelection::Implicit => {
                self.selection = InputSelection::Stdin {
                    project_context: Some(path),
                };
                Ok(())
            }
            InputSelection::Stdin { project_context } => {
                if project_context.replace(path).is_some() {
                    Err(FormatError::InvalidOption(
                        "--project-context may be specified only once".into(),
                    ))
                } else {
                    Ok(())
                }
            }
            InputSelection::ExplicitPaths { .. }
            | InputSelection::All { .. }
            | InputSelection::AllFiles { .. } => Err(project_context_conflict()),
        }
    }

    pub(super) fn select_all(&mut self, all_files: bool) -> Result<(), FormatError> {
        let selection = std::mem::take(&mut self.selection);
        self.selection = match selection {
            InputSelection::Implicit => bulk_selection(all_files, None),
            InputSelection::Stdin { project_context } => {
                self.selection = InputSelection::Stdin { project_context };
                return Err(
                    if matches!(
                        self.selection,
                        InputSelection::Stdin {
                            project_context: Some(_),
                            ..
                        }
                    ) {
                        project_context_conflict()
                    } else {
                        stdin_conflict()
                    },
                );
            }
            InputSelection::ExplicitPaths { paths, isolated } => {
                if isolated {
                    self.selection = InputSelection::ExplicitPaths { paths, isolated };
                    return Err(isolated_selection_conflict());
                }
                if paths.len() > 1 {
                    self.selection = InputSelection::ExplicitPaths { paths, isolated };
                    return Err(bulk_directory_conflict());
                }
                bulk_selection(all_files, paths.into_iter().next())
            }
            InputSelection::All { directory } if !all_files => InputSelection::All { directory },
            InputSelection::AllFiles { directory } if all_files => {
                InputSelection::AllFiles { directory }
            }
            InputSelection::All { directory } => {
                self.selection = InputSelection::All { directory };
                return Err(all_kind_conflict());
            }
            InputSelection::AllFiles { directory } => {
                self.selection = InputSelection::AllFiles { directory };
                return Err(all_kind_conflict());
            }
        };
        Ok(())
    }

    pub(super) fn set_isolated(&mut self) -> Result<(), FormatError> {
        if self
            .options
            .context_paths
            .as_ref()
            .is_some_and(|paths| !paths.is_empty())
        {
            return Err(FormatError::InvalidOption(
                "--isolated cannot be combined with --context-path".into(),
            ));
        }
        match &mut self.selection {
            InputSelection::Implicit => {
                self.selection = InputSelection::ExplicitPaths {
                    paths: Vec::new(),
                    isolated: true,
                };
                Ok(())
            }
            InputSelection::Stdin { project_context } => Err(if project_context.is_some() {
                project_context_conflict()
            } else {
                stdin_conflict()
            }),
            InputSelection::ExplicitPaths { isolated, .. } => {
                *isolated = true;
                Ok(())
            }
            InputSelection::All { .. } | InputSelection::AllFiles { .. } => {
                Err(isolated_selection_conflict())
            }
        }
    }

    pub(super) fn push_context_path(&mut self, path: ContextPath) -> Result<(), FormatError> {
        if matches!(
            &self.selection,
            InputSelection::ExplicitPaths { isolated: true, .. }
        ) {
            return Err(FormatError::InvalidOption(
                "--isolated cannot be combined with --context-path".into(),
            ));
        }
        self.options.push_context_path(path);
        Ok(())
    }

    pub(super) fn set_stdout(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::Stdout);
        Ok(())
    }

    pub(super) fn set_check(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::Check);
        Ok(())
    }

    pub(super) fn set_diff(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::Diff);
        Ok(())
    }

    pub(super) fn set_show_files(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::ShowFiles);
        Ok(())
    }

    pub(super) fn set_query_format(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::QueryFormat);
        Ok(())
    }

    pub(super) fn set_last_indent(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::LastIndent);
        Ok(())
    }

    pub(super) fn set_last_usable(&mut self) -> Result<(), FormatError> {
        self.request_action(ActionRequest::LastUsable);
        Ok(())
    }

    fn request_action(&mut self, request: ActionRequest) {
        if !self.actions.contains(&request) {
            self.actions.push(request);
        }
    }

    pub(super) fn push_format(&mut self, id: OptionId, setting: FormatSetting) {
        self.options.push_format(id, setting);
    }

    pub(super) fn finish(mut self, mut config: FormatConfig) -> Result<Invocation, FormatError> {
        self.validate()?;
        let action = self.resolve_action();

        let (paths, project_context, all, all_files, stdin, isolated) = match self.selection {
            InputSelection::Implicit => (Vec::new(), None, false, false, false, false),
            InputSelection::Stdin { project_context } => {
                (Vec::new(), project_context, false, false, true, false)
            }
            InputSelection::ExplicitPaths { paths, isolated } => {
                (paths, None, false, false, false, isolated)
            }
            InputSelection::All { directory } => (
                directory.into_iter().collect(),
                None,
                true,
                false,
                false,
                false,
            ),
            InputSelection::AllFiles { directory } => (
                directory.into_iter().collect(),
                None,
                false,
                true,
                false,
                false,
            ),
        };

        let (stdout, check, diff, show_files, query_format, last_indent, last_usable) = match action
        {
            Action::Rewrite => (false, false, false, false, false, false, false),
            Action::Stdout => (true, false, false, false, false, false, false),
            Action::Check => (false, true, false, false, false, false, false),
            Action::Diff { check } => (false, check, true, false, false, false, false),
            Action::ShowFiles => (false, false, false, true, false, false, false),
            Action::QueryFormat => (false, false, false, false, true, false, false),
            Action::IndentQuery(IndentQuery::LastIndent) => {
                (false, false, false, false, false, true, false)
            }
            Action::IndentQuery(IndentQuery::LastUsable) => {
                (false, false, false, false, false, false, true)
            }
            Action::IndentQuery(IndentQuery::Both) => {
                (false, false, false, false, false, true, true)
            }
        };
        config.last_indent = last_indent;
        config.last_usable = last_usable;

        Ok(Invocation {
            config,
            paths,
            project_context,
            context_paths: self.options.context_paths.take().unwrap_or_default(),
            all,
            all_files,
            no_submodules: self.options.no_submodules.unwrap_or(false),
            stdin,
            stdout,
            force_free_input: self.options.force_free_input.unwrap_or(false),
            query_format,
            isolated,
            check,
            diff,
            show_files,
            exclude: self.options.exclude,
            extend_exclude: self.options.extend_exclude,
        })
    }

    fn validate(&self) -> Result<(), FormatError> {
        let stdout = self.has_action(ActionRequest::Stdout);
        let check = self.has_action(ActionRequest::Check);
        let diff = self.has_action(ActionRequest::Diff);
        let show_files = self.has_action(ActionRequest::ShowFiles);
        let query_format = self.has_action(ActionRequest::QueryFormat);
        let indent_query = self.has_action(ActionRequest::LastIndent)
            || self.has_action(ActionRequest::LastUsable);

        // Keep the historical validation priority while resolving into typed
        // selection/action states only after every argv token has been seen.
        // This avoids argument-order-dependent diagnostics without restoring
        // the old boolean matrix as DraftInvocation state.
        if let InputSelection::Stdin { project_context } = &self.selection {
            if project_context.is_some() && (stdout || check || diff || show_files) {
                return Err(project_context_conflict());
            }
            if project_context.is_none() && (stdout || check || diff || show_files) {
                return Err(stdin_conflict());
            }
        }

        if stdout
            && (!matches!(
                &self.selection,
                InputSelection::ExplicitPaths { ref paths, .. } if paths.len() == 1
            ) || check
                || diff
                || show_files)
        {
            return Err(stdout_conflict());
        }

        if matches!(
            &self.selection,
            InputSelection::ExplicitPaths {
                isolated: true,
                ref paths
            } if paths.is_empty()
        ) {
            return Err(isolated_selection_conflict());
        }
        if matches!(
            &self.selection,
            InputSelection::ExplicitPaths { isolated: true, .. }
        ) && self
            .options
            .context_paths
            .as_ref()
            .is_some_and(|paths| !paths.is_empty())
        {
            return Err(FormatError::InvalidOption(
                "--isolated cannot be combined with --context-path".into(),
            ));
        }

        if diff && !self.has_file_selection() {
            return Err(FormatError::InvalidOption(
                "--diff requires paths, --all, or --all-files".into(),
            ));
        }
        if check && !self.has_file_selection() {
            return Err(FormatError::InvalidOption(
                "--check requires paths, --all, or --all-files".into(),
            ));
        }
        if show_files && !self.has_file_selection() {
            return Err(FormatError::InvalidOption(
                "--show-files requires paths, --all, or --all-files".into(),
            ));
        }
        if show_files && (check || diff || indent_query) {
            return Err(show_files_action_conflict());
        }
        if query_format && (stdout || check || diff || show_files || indent_query) {
            return Err(query_format_action_conflict());
        }
        if query_format
            && (matches!(
                &self.selection,
                InputSelection::Stdin {
                    project_context: Some(_)
                } | InputSelection::ExplicitPaths { isolated: true, .. }
            ) || self
                .options
                .context_paths
                .as_ref()
                .is_some_and(|paths| !paths.is_empty()))
        {
            return Err(FormatError::InvalidOption(
                "--query-format cannot be combined with --project-context, --context-path, or --isolated".into(),
            ));
        }
        if indent_query
            && (matches!(
                &self.selection,
                InputSelection::ExplicitPaths { .. }
                    | InputSelection::All { .. }
                    | InputSelection::AllFiles { .. }
            ) || check
                || diff)
        {
            return Err(indent_query_action_conflict());
        }
        Ok(())
    }

    fn resolve_action(&self) -> Action {
        if self.has_action(ActionRequest::QueryFormat) {
            Action::QueryFormat
        } else if self.has_action(ActionRequest::ShowFiles) {
            Action::ShowFiles
        } else if self.has_action(ActionRequest::Stdout) {
            Action::Stdout
        } else if self.has_action(ActionRequest::Diff) {
            Action::Diff {
                check: self.has_action(ActionRequest::Check),
            }
        } else if self.has_action(ActionRequest::Check) {
            Action::Check
        } else {
            match (
                self.has_action(ActionRequest::LastIndent),
                self.has_action(ActionRequest::LastUsable),
            ) {
                (true, true) => Action::IndentQuery(IndentQuery::Both),
                (true, false) => Action::IndentQuery(IndentQuery::LastIndent),
                (false, true) => Action::IndentQuery(IndentQuery::LastUsable),
                (false, false) => Action::Rewrite,
            }
        }
    }

    fn has_action(&self, request: ActionRequest) -> bool {
        self.actions.contains(&request)
    }

    fn has_file_selection(&self) -> bool {
        match &self.selection {
            InputSelection::ExplicitPaths { paths, .. } => !paths.is_empty(),
            InputSelection::All { .. } | InputSelection::AllFiles { .. } => true,
            InputSelection::Implicit | InputSelection::Stdin { .. } => false,
        }
    }
}

fn bulk_selection(all_files: bool, directory: Option<PathBuf>) -> InputSelection {
    if all_files {
        InputSelection::AllFiles { directory }
    } else {
        InputSelection::All { directory }
    }
}

fn project_context_conflict() -> FormatError {
    FormatError::InvalidOption(
        "--project-context cannot be combined with paths, --all, --all-files, --stdout, --isolated, --check, --diff, or --show-files".into(),
    )
}

fn stdin_conflict() -> FormatError {
    FormatError::InvalidOption(
        "--stdin cannot be combined with paths, --all, --all-files, --stdout, --check, --diff, --show-files, or --isolated".into(),
    )
}

fn all_kind_conflict() -> FormatError {
    FormatError::InvalidOption("--all and --all-files cannot be combined".into())
}

fn bulk_directory_conflict() -> FormatError {
    FormatError::InvalidOption("--all and --all-files accept at most one directory path".into())
}

fn isolated_selection_conflict() -> FormatError {
    FormatError::InvalidOption(
        "--isolated requires one or more explicit paths and cannot be combined with --all or --all-files"
            .into(),
    )
}

fn stdout_conflict() -> FormatError {
    FormatError::InvalidOption(
        "--stdout requires exactly one path and cannot be combined with --all, --all-files, --check, --diff, or --show-files".into(),
    )
}

fn show_files_action_conflict() -> FormatError {
    FormatError::InvalidOption(
        "--show-files cannot be combined with --check, --diff, or query modes".into(),
    )
}

fn query_format_action_conflict() -> FormatError {
    FormatError::InvalidOption(
        "--query-format cannot be combined with output, check, diff, or other query modes".into(),
    )
}

fn indent_query_action_conflict() -> FormatError {
    FormatError::InvalidOption(
        "-lastindent/-lastusable cannot be combined with path-update, --check, or --diff".into(),
    )
}

#[cfg(test)]
mod tests;
