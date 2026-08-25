use super::{ArgCursor, ConfigSelection};
use crate::{
    cli::{
        draft::DraftInvocation,
        options::{self, CliArity, OptionId},
        settings::{self, parse_bool},
        ContextPath,
    },
    error::FormatError,
};
use std::path::PathBuf;

pub(super) fn parse_long<I>(
    name: &str,
    value: Option<String>,
    cursor: &mut ArgCursor<I>,
    draft: &mut DraftInvocation,
    config_selection: &mut ConfigSelection,
) -> Result<(), FormatError>
where
    I: Iterator<Item = String>,
{
    let Some(spec) = options::lookup_long(name) else {
        if let Some(construct) = name.strip_prefix("indent-") {
            let mut value = value;
            let parsed = cursor.required_long(&mut value)?;
            settings::parse_num(&parsed)?;
            return Err(FormatError::InvalidOption(format!("--indent-{construct}")));
        }
        return Err(FormatError::InvalidOption(format!("--{name}")));
    };
    let value = consume_value(spec.cli_arity, name, value, cursor)?;

    match spec.id {
        OptionId::Config => {
            config_selection
                .explicit
                .push(value.expect("required option has a value"));
        }
        OptionId::NoConfig => config_selection.no_config = true,
        OptionId::LastIndent => draft.set_last_indent()?,
        OptionId::LastUsable => draft.set_last_usable()?,
        OptionId::All => draft.select_all(false)?,
        OptionId::AllFiles => draft.select_all(true)?,
        OptionId::NoSubmodules => {
            draft.options.no_submodules = Some(
                value
                    .as_deref()
                    .map(parse_bool)
                    .transpose()?
                    .unwrap_or(true),
            );
        }
        OptionId::ContextPath => {
            let path = value.expect("required option has a value");
            if path.is_empty() {
                return Err(FormatError::InvalidOption(
                    "--context-path requires a path".into(),
                ));
            }
            draft.push_context_path(ContextPath {
                path: PathBuf::from(path),
                base: None,
            })?;
        }
        OptionId::ProjectContext => {
            let path = value.expect("required option has a value");
            if path.is_empty() {
                return Err(FormatError::InvalidOption(
                    "--project-context requires a path".into(),
                ));
            }
            draft.select_project_context(PathBuf::from(path))?;
        }
        OptionId::Stdin => draft.select_stdin()?,
        OptionId::Stdout => draft.set_stdout()?,
        OptionId::Isolated => draft.set_isolated()?,
        OptionId::Check => draft.set_check()?,
        OptionId::Diff => draft.set_diff()?,
        OptionId::ShowFiles => draft.set_show_files()?,
        OptionId::QueryFormat => draft.set_query_format()?,
        OptionId::Exclude | OptionId::ExtendExclude => {
            let pattern = value.expect("required option has a value");
            if pattern.is_empty() {
                return Err(FormatError::InvalidOption(format!(
                    "--{name} requires a non-empty glob"
                )));
            }
            if spec.id == OptionId::Exclude {
                draft.options.push_exclude(pattern);
            } else {
                draft.options.extend_exclude.push(pattern);
            }
        }
        OptionId::InputFormat => {
            draft.options.force_free_input = Some(settings::parse_input_format(
                value.as_deref().expect("required option has a value"),
            )?);
        }
        OptionId::OutputFormat => {
            settings::parse_output_format(value.as_deref().expect("required option has a value"))?
        }
        OptionId::Help | OptionId::Version => {
            unreachable!("handled before long-option parsing")
        }
        _ => {
            if let Some(setting) = settings::parse_format_setting(spec.id, value.as_deref())? {
                draft.push_format(spec.id, setting);
            }
        }
    }
    Ok(())
}

fn consume_value<I>(
    arity: CliArity,
    name: &str,
    mut value: Option<String>,
    cursor: &mut ArgCursor<I>,
) -> Result<Option<String>, FormatError>
where
    I: Iterator<Item = String>,
{
    match arity {
        CliArity::None => {
            if value.is_some() {
                Err(FormatError::InvalidOption(format!(
                    "--{name} does not accept a value"
                )))
            } else {
                Ok(None)
            }
        }
        CliArity::Required => cursor.required_long(&mut value).map(Some),
        CliArity::Optional => Ok(value),
    }
}
