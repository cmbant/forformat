use super::{
    draft::DraftInvocation, options::single_dash_long_option_suggestion, settings::OptionLayer,
    Command,
};
use crate::{config::FormatConfig, error::FormatError};
use std::path::PathBuf;

mod config;
mod long;
mod short;
mod value;

use config::{config_start, ConfigSelection};
use value::ArgCursor;

pub(super) struct ParsedCommand {
    pub(super) command: Command,
    pub(super) config_selection: ConfigSelection,
    pub(super) cli_layer: OptionLayer,
}

pub fn parse<I>(args: I) -> Result<Command, FormatError>
where
    I: IntoIterator<Item = String>,
    I::IntoIter: Iterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let preliminary = parse_inner(args)?;
    if matches!(preliminary.command, Command::Help | Command::Version) {
        return Ok(preliminary.command);
    }

    let (no_config, explicit_config) = preliminary.config_selection.resolve()?;
    let config_arguments = if no_config {
        crate::config::ConfigArguments::default()
    } else {
        let cwd = std::env::current_dir().map_err(|error| {
            FormatError::InvalidOption(format!("cannot determine current directory: {error}"))
        })?;
        let start = config_start(&preliminary.command, &cwd);
        crate::config::config_args(&start, explicit_config.as_deref())?
    };

    // Both sources now produce typed option layers. TOML is materialized first
    // with schema-defined baseline/specific phases; argv is then applied in its
    // original order so command-line scalars win and legacy ordering semantics
    // remain intact without manufacturing a second argv.
    let mut format = FormatConfig::default();
    config_arguments.layer.apply_config(&mut format);
    preliminary.cli_layer.apply_cli(&mut format);
    validate_format_config(&format)?;

    let mut command = preliminary.command;
    if let Command::Run(invocation) = &mut command {
        // Query modes are CLI-only action state, not formatter configuration,
        // so retain those two engine-facing compatibility bits when replacing
        // the preliminary CLI-only FormatConfig with the merged one.
        format.last_indent = invocation.config.last_indent;
        format.last_usable = invocation.config.last_usable;
        invocation.config = format;

        invocation.no_submodules = preliminary
            .cli_layer
            .no_submodules
            .or(config_arguments.layer.no_submodules)
            .unwrap_or(false);
        invocation.force_free_input = preliminary
            .cli_layer
            .force_free_input
            .or(config_arguments.layer.force_free_input)
            .unwrap_or(false);
        invocation.exclude = preliminary
            .cli_layer
            .exclude
            .clone()
            .or(config_arguments.layer.exclude.clone());
        invocation.extend_exclude = config_arguments
            .layer
            .extend_exclude
            .iter()
            .chain(&preliminary.cli_layer.extend_exclude)
            .cloned()
            .collect();
        invocation.context_paths = if let Some(paths) = &preliminary.cli_layer.context_paths {
            paths.clone()
        } else if !invocation.isolated && !invocation.query_format && !invocation.show_files {
            config_arguments
                .layer
                .context_paths
                .clone()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
    }
    Ok(command)
}

pub(super) fn parse_inner<I>(args: I) -> Result<ParsedCommand, FormatError>
where
    I: IntoIterator<Item = String>,
    I::IntoIter: Iterator<Item = String>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    let mut cursor = ArgCursor::new(args);
    let mut draft = DraftInvocation::default();
    let mut config_selection = ConfigSelection::default();
    let mut help = false;
    let mut version = false;
    let mut options_ended = false;

    while let Some(arg) = cursor.next() {
        if arg == "--" {
            options_ended = true;
            continue;
        }
        if options_ended {
            draft.push_path(PathBuf::from(arg))?;
            continue;
        }
        if arg == "-h" || arg == "--help" {
            help = true;
            continue;
        }
        if arg == "-v" || arg == "--version" {
            version = true;
            continue;
        }
        if arg == "-lastindent" {
            draft.set_last_indent()?;
            continue;
        }
        if arg == "-lastusable" {
            draft.set_last_usable()?;
            continue;
        }
        if arg == "-ifree" || arg == "--input-format=free" {
            draft.options.force_free_input = Some(true);
            continue;
        }
        if arg == "-ofree" || arg == "-osame" || arg == "--output-format=free" {
            continue;
        }
        if arg == "-ifixed"
            || arg == "-ofixed"
            || arg == "--input-format=fixed"
            || arg == "--output-format=fixed"
        {
            return Err(FormatError::Unsupported(
                "fixed-form input/output is not supported".into(),
            ));
        }
        if let Some(suggestion) = single_dash_long_option_suggestion(&arg) {
            return Err(FormatError::InvalidOption(format!(
                "{arg} (did you mean {suggestion}? Long options use two dashes.)"
            )));
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = if let Some((name, value)) = long.split_once('=') {
                (
                    super::options::normalize_long(name),
                    Some(value.to_string()),
                )
            } else {
                (super::options::normalize_long(long), None)
            };
            long::parse_long(&name, value, &mut cursor, &mut draft, &mut config_selection)?;
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            short::parse_short(arg, &mut cursor, &mut draft)?;
            continue;
        }
        if arg.starts_with('-') {
            return Err(FormatError::InvalidOption(arg));
        }
        draft.push_path(PathBuf::from(arg))?;
    }

    let cli_layer = draft.options.clone();
    let mut format = FormatConfig::default();
    cli_layer.apply_cli(&mut format);
    validate_format_config(&format)?;

    // Keep validation before the help/version selection. Historically those
    // switches do not erase invalid combinations that were also supplied.
    let command = if help {
        draft.finish(format)?;
        Command::Help
    } else if version {
        draft.finish(format)?;
        Command::Version
    } else {
        Command::Run(Box::new(draft.finish(format)?))
    };
    Ok(ParsedCommand {
        command,
        config_selection,
        cli_layer,
    })
}

fn validate_format_config(config: &FormatConfig) -> Result<(), FormatError> {
    if config.rewrap && !config.mode.wraps() {
        return Err(FormatError::InvalidOption(
            "--rewrap requires full mode: --indent-only, --normalize-only, --canonicalize-only, and --canonicalize-and-indent do not run the wrapper".into(),
        ));
    }
    Ok(())
}
