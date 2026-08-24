use super::{draft::DraftInvocation, options::single_dash_long_option_suggestion, Command};
use crate::{config::ConfigArguments, error::FormatError};
use std::path::PathBuf;

mod config;
mod long;
mod short;
mod value;

use config::config_start;
use config::ConfigSelection;
use value::{
    parse_bool, parse_num, parse_style_choice, push_define, reject_value, set_construct, set_short,
    set_start, ArgCursor,
};

pub(super) struct ParsedCommand {
    pub(super) command: Command,
    pub(super) config_selection: ConfigSelection,
}

pub fn parse<I>(args: I) -> Result<Command, FormatError>
where
    I: IntoIterator<Item = String>,
    I::IntoIter: Iterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    let preliminary = parse_inner(args.clone())?;
    if matches!(preliminary.command, Command::Help | Command::Version) {
        return Ok(preliminary.command);
    }
    let (no_config, explicit_config) = preliminary.config_selection.resolve()?;
    let mut config = if no_config {
        ConfigArguments::default()
    } else {
        let cwd = std::env::current_dir().map_err(|error| {
            FormatError::InvalidOption(format!("cannot determine current directory: {error}"))
        })?;
        let start = config_start(&preliminary.command, &cwd);
        crate::config::config_args(&start, explicit_config.as_deref())?
    };
    // Config args are merged by prepending them to argv, which makes the
    // command line win for scalar options and accumulate for repeatable ones.
    // `--exclude` must not accumulate: it selects a set rather than adding to
    // one, so giving it on the command line discards the config file's
    // `exclude` the way it discards the built-in defaults. `--extend-exclude`
    // is the additive spelling and keeps accumulating. `preliminary` is a parse
    // of the command line alone, so it answers "was `--exclude` given there?"
    // using the real option grammar rather than a second-guessing rescan.
    if matches!(&preliminary.command, Command::Run(invocation) if invocation.exclude.is_some()) {
        config.args.retain(|arg| !arg.starts_with("--exclude="));
    }
    if matches!(&preliminary.command, Command::Run(invocation) if !invocation.context_paths.is_empty())
    {
        config.context_paths.clear();
    }
    let config_context_paths = config.context_paths;
    let mut combined = Vec::with_capacity(1 + config.args.len() + args.len());
    combined.push(
        args.first()
            .cloned()
            .unwrap_or_else(|| "forformat".to_string()),
    );
    combined.extend(config.args);
    combined.extend(args.into_iter().skip(1));
    let mut command = parse_inner(combined)?.command;
    if !config_context_paths.is_empty() {
        if let Command::Run(invocation) = &mut command {
            if invocation.context_paths.is_empty()
                && !invocation.isolated
                && !invocation.query_format
                && !invocation.show_files
            {
                invocation.context_paths = config_context_paths;
            }
        }
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
            draft.paths.push(PathBuf::from(arg));
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
            draft.config.last_indent = true;
            continue;
        }
        if arg == "-lastusable" {
            draft.config.last_usable = true;
            continue;
        }
        if arg == "-ifree" || arg == "--input-format=free" {
            draft.force_free_input = true;
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
        draft.paths.push(PathBuf::from(arg));
    }

    // Keep validation before the help/version selection. Historically those
    // switches do not erase invalid combinations that were also supplied.
    draft.validate()?;
    let command = if help {
        Command::Help
    } else if version {
        Command::Version
    } else {
        Command::Run(Box::new(draft.finish()))
    };
    Ok(ParsedCommand {
        command,
        config_selection,
    })
}
