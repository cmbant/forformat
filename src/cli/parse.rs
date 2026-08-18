use super::{draft::DraftInvocation, options::single_dash_long_option_suggestion, Command};
use crate::{
    config::{ConfigArguments, FormatConfig, MacroDefine},
    error::FormatError,
};
use std::path::{Path, PathBuf};

mod long;
mod short;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ConfigSelection {
    pub(super) no_config: bool,
    pub(super) explicit: Vec<String>,
}

impl ConfigSelection {
    pub(super) fn resolve(&self) -> Result<(bool, Option<PathBuf>), FormatError> {
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

pub(super) struct ParsedCommand {
    pub(super) command: Command,
    pub(super) config_selection: ConfigSelection,
}

pub(super) struct ArgCursor<I> {
    inner: I,
}

impl<I> ArgCursor<I>
where
    I: Iterator<Item = String>,
{
    fn new(inner: I) -> Self {
        Self { inner }
    }

    pub(super) fn next(&mut self) -> Option<String> {
        self.inner.next()
    }

    /// Required long-option values may be attached with `=` or consume the
    /// next argv element, including one that starts with `-`.
    pub(super) fn required_long(
        &mut self,
        inline: &mut Option<String>,
    ) -> Result<String, FormatError> {
        if let Some(value) = inline.take() {
            Ok(value)
        } else {
            self.next()
                .ok_or_else(|| FormatError::InvalidOption("missing option value".into()))
        }
    }

    /// Findent short options accept attached values (`-i4`) and separated
    /// values (`-i 4`). Optional long values deliberately do not use this.
    pub(super) fn required_short(
        &mut self,
        option: char,
        attached: &str,
    ) -> Result<String, FormatError> {
        if attached.is_empty() {
            self.next()
                .ok_or_else(|| FormatError::InvalidOption(format!("-{option} requires a value")))
        } else {
            Ok(attached.to_string())
        }
    }
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

fn config_start(command: &Command, cwd: &Path) -> PathBuf {
    if let Command::Run(invocation) = command {
        if let Some(path) = invocation.project_context.as_deref() {
            let candidate = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            return if candidate.is_dir() {
                candidate
            } else {
                candidate
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| cwd.to_path_buf())
            };
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

pub(super) fn reject_value(name: &str, value: &Option<String>) -> Result<(), FormatError> {
    if value.is_some() {
        Err(FormatError::InvalidOption(format!(
            "--{name} does not accept a value"
        )))
    } else {
        Ok(())
    }
}

/// Record a `-D NAME[=VALUE]` definition. Only the name affects casing, but
/// the value is kept for CPP evaluation.
pub(super) fn push_define(config: &mut FormatConfig, spec: &str) {
    let (name, value) = match spec.split_once('=') {
        Some((name, value)) => (name, Some(value.to_string())),
        None => (spec, None),
    };
    if !name.is_empty() {
        config.defines.push(MacroDefine {
            name: name.to_string(),
            value,
        });
    }
}

pub(super) fn parse_num(value: &str) -> Result<usize, FormatError> {
    value
        .parse::<isize>()
        .ok()
        .filter(|value| *value >= 0)
        .map(|value| value as usize)
        .ok_or_else(|| {
            FormatError::InvalidOption(format!("expected non-negative integer, got {value}"))
        })
}

pub(super) fn parse_bool(value: &str) -> Result<bool, FormatError> {
    match value {
        "0" | "false" | "no" => Ok(false),
        "1" | "true" | "yes" => Ok(true),
        _ => Err(FormatError::InvalidOption(format!(
            "expected 0 or 1, got {value}"
        ))),
    }
}

pub(super) fn parse_style_choice<T: Copy>(
    option: &str,
    value: &str,
    choices: &[(&str, T)],
) -> Result<T, FormatError> {
    choices
        .iter()
        .find(|(allowed, _)| *allowed == value)
        .map(|(_, parsed)| *parsed)
        .ok_or_else(|| {
            let allowed = choices
                .iter()
                .map(|(allowed, _)| *allowed)
                .collect::<Vec<_>>()
                .join(", ");
            FormatError::InvalidOption(format!(
                "--{option} has invalid value `{value}`; allowed values: {allowed}"
            ))
        })
}

pub(super) fn set_start(config: &mut FormatConfig, value: &str) -> Result<(), FormatError> {
    if value.eq_ignore_ascii_case("a") || value.eq_ignore_ascii_case("auto") {
        config.auto_start_indent = true
    } else {
        config.start_indent = parse_num(value)?;
        config.auto_start_indent = false
    }
    Ok(())
}

pub(super) fn set_short(
    config: &mut FormatConfig,
    option: char,
    value: usize,
) -> Result<(), FormatError> {
    match option {
        'a' => config.construct_indents.associate = value,
        'b' => config.construct_indents.block = value,
        'c' => config.case_indent = value,
        'd' => config.construct_indents.do_ = value,
        'e' => config.entry_indent = value,
        'E' => config.construct_indents.r#enum = value,
        'f' => config.construct_indents.if_ = value,
        'F' => config.construct_indents.forall = value,
        'j' => config.construct_indents.interface = value,
        'm' => config.construct_indents.module = value,
        'r' => config.construct_indents.procedure = value,
        's' => config.construct_indents.select = value,
        't' => config.construct_indents.r#type = value,
        'w' => config.construct_indents.where_ = value,
        'x' => config.construct_indents.critical = value,
        _ => return Err(FormatError::InvalidOption(format!("-{option}"))),
    }
    Ok(())
}

pub(super) fn set_construct(
    config: &mut FormatConfig,
    name: &str,
    value: usize,
) -> Result<(), FormatError> {
    match name {
        "associate" => config.construct_indents.associate = value,
        "block" => config.construct_indents.block = value,
        "case" => config.case_indent = value,
        "contains" => config.contains_indent = value,
        "do" => config.construct_indents.do_ = value,
        "entry" => config.entry_indent = value,
        "enum" => config.construct_indents.r#enum = value,
        "forall" => config.construct_indents.forall = value,
        "if" => config.construct_indents.if_ = value,
        "interface" => config.construct_indents.interface = value,
        "module" => config.construct_indents.module = value,
        "procedure" => config.construct_indents.procedure = value,
        "select" => config.construct_indents.select = value,
        "type" => config.construct_indents.r#type = value,
        "where" => config.construct_indents.where_ = value,
        "critical" => config.construct_indents.critical = value,
        _ => return Err(FormatError::InvalidOption(format!("--indent-{name}"))),
    }
    Ok(())
}
