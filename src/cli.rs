use crate::{
    config::{ConfigArguments, FormatConfig, FormatMode, KeywordCase, MacroDefine},
    error::FormatError,
};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ConfigSelection {
    no_config: bool,
    explicit: Vec<String>,
}

impl ConfigSelection {
    fn resolve(&self) -> Result<(bool, Option<PathBuf>), FormatError> {
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

struct ParsedCommand {
    command: Command,
    config_selection: ConfigSelection,
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
        if !invocation.all && !invocation.all_files && invocation.paths.len() == 1 {
            let candidate = if invocation.paths[0].is_absolute() {
                invocation.paths[0].clone()
            } else {
                cwd.join(&invocation.paths[0])
            };
            if candidate.is_file() {
                return candidate
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| cwd.to_path_buf());
            }
        }
        if (invocation.all || invocation.all_files) && invocation.paths.len() == 1 {
            let candidate = if invocation.paths[0].is_absolute() {
                invocation.paths[0].clone()
            } else {
                cwd.join(&invocation.paths[0])
            };
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    cwd.to_path_buf()
}

fn parse_inner<I>(args: I) -> Result<ParsedCommand, FormatError>
where
    I: IntoIterator<Item = String>,
    I::IntoIter: Iterator<Item = String>,
{
    let mut a = args.into_iter();
    let _program = a.next();
    let mut c = FormatConfig::default();
    let mut config_selection = ConfigSelection::default();
    let mut help = false;
    let mut version = false;
    let mut options_ended = false;
    let mut paths = Vec::new();
    let mut project_context = None;
    let mut context_paths = Vec::new();
    let mut all = false;
    let mut all_files = false;
    let mut no_submodules = false;
    let mut stdin = false;
    let mut stdout = false;
    let mut force_free_input = false;
    let mut query_format = false;
    let mut isolated = false;
    let mut check = false;
    let mut diff = false;
    let mut show_files = false;
    let mut exclude: Option<Vec<String>> = None;
    let mut extend_exclude = Vec::new();
    while let Some(arg) = a.next() {
        if arg == "--" {
            options_ended = true;
            continue;
        }
        if options_ended {
            paths.push(PathBuf::from(arg));
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
            c.last_indent = true;
            continue;
        }
        if arg == "-lastusable" {
            c.last_usable = true;
            continue;
        }
        if arg == "-ifree" || arg == "--input-format=free" {
            force_free_input = true;
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
            let (name, val) = if let Some((n, v)) = long.split_once('=') {
                (
                    n.replace('_', "-").to_ascii_lowercase(),
                    Some(v.to_string()),
                )
            } else {
                (long.replace('_', "-").to_ascii_lowercase(), None)
            };
            let mut value = val;
            let need = |v: &mut Option<String>,
                        a: &mut I::IntoIter|
             -> Result<String, FormatError> {
                if let Some(x) = v.take() {
                    Ok(x)
                } else {
                    a.next()
                        .ok_or_else(|| FormatError::InvalidOption("missing option value".into()))
                }
            };
            match name.as_str() {
                "config" => {
                    let path = need(&mut value, &mut a)?;
                    config_selection.explicit.push(path);
                }
                "no-config" => {
                    reject_value(&name, &value)?;
                    config_selection.no_config = true;
                }
                "indent" => {
                    let v = need(&mut value, &mut a)?;
                    if v == "none" {
                        c.apply_indent = false
                    } else {
                        c.indent = parse_num(&v)?;
                        c.construct_indents.set_all(c.indent);
                        c.contains_indent = c.indent;
                        c.continuation_indent = c.indent;
                        c.case_indent = c.indent.saturating_sub(c.indent / 2);
                        c.entry_indent = c.case_indent
                    }
                }
                "start-indent" => set_start(&mut c, &need(&mut value, &mut a)?)?,
                "indent-contains" => {
                    let v = need(&mut value, &mut a)?;
                    if v == "restart" {
                        c.contains_restart = true
                    } else {
                        c.contains_indent = parse_num(&v)?
                    }
                }
                "include-left" => c.include_left = parse_bool(&need(&mut value, &mut a)?)?,
                "label-left" => c.label_left = parse_bool(&need(&mut value, &mut a)?)?,
                "max-indent" => c.max_indent = parse_num(&need(&mut value, &mut a)?)?,
                "openmp" => c.openmp = parse_bool(&need(&mut value, &mut a)?)?,
                "indent-ampersand" => {
                    c.indent_ampersand = value
                        .as_deref()
                        .map(parse_bool)
                        .transpose()?
                        .unwrap_or(true)
                }
                "indent-continuation" => {
                    let v = need(&mut value, &mut a)?;
                    if v == "none" || v == "-" {
                        c.indent_continuation = false;
                    } else if v == "default" || v == "d" {
                        c.indent_continuation = true;
                    } else {
                        c.continuation_indent = parse_num(&v)?;
                    }
                }
                "align-paren" => {
                    c.align_paren_value = value.as_deref().map(parse_num).transpose()?.unwrap_or(1);
                    c.align_paren = c.align_paren_value != 0;
                }
                "ws-remred" => {
                    c.ws_remred_value = value.as_deref().map(parse_num).transpose()?.unwrap_or(1);
                    c.ws_remred = c.ws_remred_value != 0;
                }
                "align-declarations" => {
                    c.align_declarations = parse_bool(&need(&mut value, &mut a)?)?
                }
                "align-comments" => c.align_comments = parse_bool(&need(&mut value, &mut a)?)?,
                "last-indent" => {
                    reject_value(&name, &value)?;
                    c.last_indent = true;
                }
                "last-usable" => {
                    reject_value(&name, &value)?;
                    c.last_usable = true;
                }
                "all" => {
                    reject_value(&name, &value)?;
                    all = true;
                }
                "all-files" => {
                    reject_value(&name, &value)?;
                    all_files = true;
                }
                "no-submodules" => {
                    no_submodules = value
                        .as_deref()
                        .map(parse_bool)
                        .transpose()?
                        .unwrap_or(true)
                }
                "context-path" => {
                    let path = need(&mut value, &mut a)?;
                    if path.is_empty() {
                        return Err(FormatError::InvalidOption(
                            "--context-path requires a path".into(),
                        ));
                    }
                    context_paths.push(ContextPath {
                        path: PathBuf::from(path),
                        base: None,
                    });
                }
                "project-context" => {
                    let path = need(&mut value, &mut a)?;
                    if path.is_empty() {
                        return Err(FormatError::InvalidOption(
                            "--project-context requires a path".into(),
                        ));
                    }
                    if project_context.is_some() {
                        return Err(FormatError::InvalidOption(
                            "--project-context may be specified only once".into(),
                        ));
                    }
                    project_context = Some(PathBuf::from(path));
                }
                "stdin" => {
                    reject_value(&name, &value)?;
                    stdin = true;
                }
                "stdout" => {
                    reject_value(&name, &value)?;
                    stdout = true;
                }
                "isolated" => {
                    reject_value(&name, &value)?;
                    isolated = true;
                }
                "check" => {
                    reject_value(&name, &value)?;
                    check = true;
                }
                "diff" => {
                    reject_value(&name, &value)?;
                    diff = true;
                }
                "show-files" => {
                    reject_value(&name, &value)?;
                    show_files = true;
                }
                "query-format" => {
                    reject_value(&name, &value)?;
                    query_format = true;
                }
                "exclude" | "extend-exclude" => {
                    let pattern = need(&mut value, &mut a)?;
                    if pattern.is_empty() {
                        return Err(FormatError::InvalidOption(format!(
                            "--{name} requires a non-empty glob"
                        )));
                    }
                    if name == "exclude" {
                        exclude.get_or_insert_with(Vec::new).push(pattern);
                    } else {
                        extend_exclude.push(pattern);
                    }
                }
                // Mode selection.  `indent-only` must be matched before the
                // generic `indent-*` construct arm below, which would otherwise
                // read it as a construct name.
                "indent-only" => {
                    reject_value(&name, &value)?;
                    c.mode = FormatMode::IndentOnly;
                }
                "full" => {
                    reject_value(&name, &value)?;
                    c.mode = FormatMode::Full;
                }
                "normalize-only" => {
                    reject_value(&name, &value)?;
                    c.mode = FormatMode::NormalizeOnly;
                }
                "wrap" => {
                    c.wrap.enabled = value
                        .as_deref()
                        .map(parse_bool)
                        .transpose()?
                        .unwrap_or(true)
                }
                "no-wrap" => {
                    let disabled = value
                        .as_deref()
                        .map(parse_bool)
                        .transpose()?
                        .unwrap_or(true);
                    c.wrap.enabled = !disabled;
                }
                "line-length" => c.wrap.line_length = parse_num(&need(&mut value, &mut a)?)?,
                "uppercase-single-l" => {
                    c.uppercase_single_l = value
                        .as_deref()
                        .map(parse_bool)
                        .transpose()?
                        .unwrap_or(true)
                }
                "define" => push_define(&mut c, &need(&mut value, &mut a)?),
                "keyword-case" => {
                    c.style.keyword_case = parse_style_choice(
                        &name,
                        &need(&mut value, &mut a)?,
                        &[
                            ("lower", KeywordCase::Lower),
                            ("upper", KeywordCase::Upper),
                            ("preserve", KeywordCase::Preserve),
                        ],
                    )?
                }
                "relational-symbols" => {
                    c.style.relational_symbols = parse_bool(&need(&mut value, &mut a)?)?
                }
                "array-brackets" => {
                    c.style.array_brackets = parse_bool(&need(&mut value, &mut a)?)?
                }
                "compact-multiplicative" => {
                    c.style.compact_multiplicative = parse_bool(&need(&mut value, &mut a)?)?
                }
                "join-goto" => c.style.join_goto = parse_bool(&need(&mut value, &mut a)?)?,
                "split-compound-keywords" => {
                    c.style.split_compound_keywords = parse_bool(&need(&mut value, &mut a)?)?
                }
                "strip-empty-args" => {
                    c.style.strip_empty_args = parse_bool(&need(&mut value, &mut a)?)?
                }
                "remove-redundant-parens" => {
                    c.style.remove_redundant_parens = parse_bool(&need(&mut value, &mut a)?)?
                }
                "remove-terminal-return" => {
                    c.style.remove_terminal_return = parse_bool(&need(&mut value, &mut a)?)?
                }
                "program-unit-spacing" => {
                    c.style.program_unit_spacing = parse_bool(&need(&mut value, &mut a)?)?
                }
                "max-blank-lines" => {
                    let v = need(&mut value, &mut a)?;
                    c.style.max_blank_lines = if v == "preserve" {
                        None
                    } else {
                        Some(parse_num(&v)?)
                    };
                }
                "delimiter-spacing" => {
                    c.style.delimiter_spacing = parse_bool(&need(&mut value, &mut a)?)?
                }
                "comment-spacing" => {
                    c.style.comment_spacing = parse_bool(&need(&mut value, &mut a)?)?
                }
                "continuation-markers" => {
                    c.style.continuation_markers = parse_bool(&need(&mut value, &mut a)?)?
                }
                "indent-changeteam" => {
                    c.construct_indents.changeteam = parse_num(&need(&mut value, &mut a)?)?
                }
                "refactor-end" | "refactor-procedures" => {
                    let (enabled, uppercase) = match value.as_deref() {
                        None => (true, false),
                        Some("upcase") => (true, true),
                        Some(value) => (parse_bool(value)?, false),
                    };
                    c.refactor_end = enabled;
                    c.uppercase_end = uppercase;
                }
                n if n.starts_with("indent-") => {
                    let v = parse_num(&need(&mut value, &mut a)?)?;
                    set_construct(&mut c, n.trim_start_matches("indent-"), v)?
                }
                "input-format" => match need(&mut value, &mut a)?.to_ascii_lowercase().as_str() {
                    "free" => force_free_input = true,
                    "auto" => force_free_input = false,
                    "fixed" => {
                        return Err(FormatError::Unsupported(
                            "fixed-form input/output is not supported".into(),
                        ));
                    }
                    other => {
                        return Err(FormatError::InvalidOption(format!(
                            "--input-format={other}"
                        )));
                    }
                },
                "output-format" => match need(&mut value, &mut a)?.to_ascii_lowercase().as_str() {
                    "free" | "same" => {}
                    "fixed" => {
                        return Err(FormatError::Unsupported(
                            "fixed-form input/output is not supported".into(),
                        ));
                    }
                    other => {
                        return Err(FormatError::InvalidOption(format!(
                            "--output-format={other}"
                        )));
                    }
                },
                _ => return Err(FormatError::InvalidOption(format!("--{name}"))),
            }
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            let b = arg.as_bytes();
            let ch = b[1] as char;
            let value = &arg[2..];
            match ch {
                'a' | 'b' | 'c' | 'd' | 'e' | 'E' | 'f' | 'F' | 'j' | 'm' | 'r' | 's' | 't'
                | 'w' | 'x' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption(format!("-{ch} requires a value"))
                        })?
                    } else {
                        value.to_string()
                    };
                    let n = parse_num(&v)?;
                    set_short(&mut c, ch, n)?
                }
                'C' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-C requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    if v == "-" {
                        c.contains_restart = true
                    } else {
                        c.contains_indent = parse_num(&v)?;
                        c.contains_restart = false
                    }
                }
                'i' => {
                    if value == "-" {
                        c.apply_indent = false
                    } else if value == "free" {
                        force_free_input = true;
                    } else if value == "auto" {
                        force_free_input = false;
                    } else if value == "fixed" {
                        return Err(FormatError::Unsupported(
                            "fixed-form input/output is not supported".into(),
                        ));
                    } else {
                        let v = if value.is_empty() {
                            a.next().ok_or_else(|| {
                                FormatError::InvalidOption("-i requires a value".into())
                            })?
                        } else {
                            value.to_string()
                        };
                        c.indent = parse_num(&v)?;
                        c.construct_indents.set_all(c.indent);
                        c.contains_indent = c.indent;
                        c.continuation_indent = c.indent;
                        c.case_indent = c.indent.saturating_sub(c.indent / 2);
                        c.entry_indent = c.case_indent
                    }
                }
                'I' => {
                    let start_value = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-I requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    set_start(&mut c, &start_value)?;
                }
                'k' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-k requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    if v == "-" || v == "none" {
                        c.indent_continuation = false
                    } else if v == "d" || v == "default" {
                        c.indent_continuation = true
                    } else {
                        c.continuation_indent = parse_num(&v)?
                    }
                }
                'D' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-D requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    push_define(&mut c, &v);
                }
                'K' => c.indent_ampersand = true,
                'l' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-l requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    c.label_left = parse_bool(&v)?;
                }
                'M' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-M requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    c.max_indent = parse_num(&v)?;
                }
                'R' => {
                    c.refactor_end = true;
                    c.uppercase_end = value == "R"
                }
                _ => return Err(FormatError::InvalidOption(arg)),
            }
            continue;
        }
        if arg.starts_with('-') {
            return Err(FormatError::InvalidOption(arg));
        }
        paths.push(PathBuf::from(arg));
    }
    if project_context.is_some()
        && (!paths.is_empty()
            || all
            || all_files
            || stdout
            || isolated
            || check
            || diff
            || show_files)
    {
        return Err(FormatError::InvalidOption(
            "--project-context cannot be combined with paths, --all, --all-files, --stdout, --isolated, --check, --diff, or --show-files".into(),
        ));
    }
    if stdin
        && (all
            || all_files
            || !paths.is_empty()
            || stdout
            || isolated
            || check
            || diff
            || show_files)
    {
        return Err(FormatError::InvalidOption(
            "--stdin cannot be combined with paths, --all, --all-files, --stdout, --check, --diff, --show-files, or --isolated".into(),
        ));
    }
    if all && all_files {
        return Err(FormatError::InvalidOption(
            "--all and --all-files cannot be combined".into(),
        ));
    }
    if stdout && (paths.len() != 1 || all || all_files || check || diff || show_files) {
        return Err(FormatError::InvalidOption(
            "--stdout requires exactly one path and cannot be combined with --all, --all-files, --check, --diff, or --show-files".into(),
        ));
    }
    if (all || all_files) && paths.len() > 1 {
        return Err(FormatError::InvalidOption(
            "--all and --all-files accept at most one directory path".into(),
        ));
    }
    if isolated && (all || all_files || paths.is_empty()) {
        return Err(FormatError::InvalidOption(
            "--isolated requires one or more explicit paths and cannot be combined with --all-files".into(),
        ));
    }
    if isolated && !context_paths.is_empty() {
        return Err(FormatError::InvalidOption(
            "--isolated cannot be combined with --context-path".into(),
        ));
    }
    if diff && paths.is_empty() && !all && !all_files {
        return Err(FormatError::InvalidOption(
            "--diff requires paths, --all, or --all-files".into(),
        ));
    }
    if check && paths.is_empty() && !all && !all_files {
        return Err(FormatError::InvalidOption(
            "--check requires paths, --all, or --all-files".into(),
        ));
    }
    if show_files && paths.is_empty() && !all && !all_files {
        return Err(FormatError::InvalidOption(
            "--show-files requires paths, --all, or --all-files".into(),
        ));
    }
    if show_files && (check || diff || c.last_indent || c.last_usable) {
        return Err(FormatError::InvalidOption(
            "--show-files cannot be combined with --check, --diff, or query modes".into(),
        ));
    }
    if query_format && (stdout || check || diff || show_files || c.last_indent || c.last_usable) {
        return Err(FormatError::InvalidOption(
            "--query-format cannot be combined with output, check, diff, or other query modes"
                .into(),
        ));
    }
    if query_format && (project_context.is_some() || !context_paths.is_empty() || isolated) {
        return Err(FormatError::InvalidOption(
            "--query-format cannot be combined with --project-context, --context-path, or --isolated".into(),
        ));
    }
    if project_context.is_some() {
        stdin = true;
    }
    if (c.last_indent || c.last_usable) && (all || all_files || !paths.is_empty() || check || diff)
    {
        return Err(FormatError::InvalidOption(
            "-lastindent/-lastusable cannot be combined with path-update, --check, or --diff"
                .into(),
        ));
    }
    if help {
        Ok(ParsedCommand {
            command: Command::Help,
            config_selection,
        })
    } else if version {
        Ok(ParsedCommand {
            command: Command::Version,
            config_selection,
        })
    } else {
        Ok(ParsedCommand {
            command: Command::Run(Box::new(Invocation {
                config: c,
                paths,
                project_context,
                context_paths,
                all,
                all_files,
                no_submodules,
                stdin,
                stdout,
                force_free_input,
                query_format,
                isolated,
                check,
                diff,
                show_files,
                exclude,
                extend_exclude,
            })),
            config_selection,
        })
    }
}

fn reject_value(name: &str, value: &Option<String>) -> Result<(), FormatError> {
    if value.is_some() {
        Err(FormatError::InvalidOption(format!(
            "--{name} does not accept a value"
        )))
    } else {
        Ok(())
    }
}

/// Record a `-D NAME[=VALUE]` definition.  Only the name affects casing, but
/// the value is kept for the CPP evaluation the port will need.
fn push_define(c: &mut FormatConfig, spec: &str) {
    let (name, value) = match spec.split_once('=') {
        Some((name, value)) => (name, Some(value.to_string())),
        None => (spec, None),
    };
    if !name.is_empty() {
        c.defines.push(MacroDefine {
            name: name.to_string(),
            value,
        });
    }
}

fn parse_num(s: &str) -> Result<usize, FormatError> {
    s.parse::<isize>()
        .ok()
        .filter(|x| *x >= 0)
        .map(|x| x as usize)
        .ok_or_else(|| {
            FormatError::InvalidOption(format!("expected non-negative integer, got {s}"))
        })
}

fn parse_style_choice<T: Copy>(
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

/// Point out the common `-all` typo without interfering with findent-style
/// short options such as `-i4` and `-ifree`.
fn single_dash_long_option_suggestion(arg: &str) -> Option<String> {
    let spelling = arg.strip_prefix('-')?;
    if spelling.is_empty() || spelling.starts_with('-') {
        return None;
    }
    let name = spelling
        .split_once('=')
        .map_or(spelling, |(name, _)| name)
        .replace('_', "-")
        .to_ascii_lowercase();
    let known = matches!(
        name.as_str(),
        "align-comments"
            | "align-declarations"
            | "align-paren"
            | "all"
            | "all-files"
            | "array-brackets"
            | "check"
            | "compact-multiplicative"
            | "comment-spacing"
            | "config"
            | "continuation-markers"
            | "delimiter-spacing"
            | "diff"
            | "exclude"
            | "extend-exclude"
            | "full"
            | "include-left"
            | "indent-ampersand"
            | "indent-changeteam"
            | "indent-contains"
            | "indent-continuation"
            | "indent-only"
            | "isolated"
            | "label-left"
            | "last-indent"
            | "last-usable"
            | "line-length"
            | "join-goto"
            | "keyword-case"
            | "max-blank-lines"
            | "max-indent"
            | "no-wrap"
            | "normalize-only"
            | "no-config"
            | "no-submodules"
            | "context-path"
            | "openmp"
            | "project-context"
            | "program-unit-spacing"
            | "refactor-end"
            | "refactor-procedures"
            | "relational-symbols"
            | "remove-redundant-parens"
            | "remove-terminal-return"
            | "split-compound-keywords"
            | "start-indent"
            | "stdin"
            | "stdout"
            | "show-files"
            | "query-format"
            | "strip-empty-args"
            | "uppercase-single-l"
            | "wrap"
            | "ws-remred"
    ) || name == "define"
        || name == "input-format"
        || name == "output-format"
        || name.starts_with("indent-");
    known.then(|| format!("--{spelling}"))
}

fn parse_bool(s: &str) -> Result<bool, FormatError> {
    match s {
        "0" | "false" | "no" => Ok(false),
        "1" | "true" | "yes" => Ok(true),
        _ => Err(FormatError::InvalidOption(format!(
            "expected boolean (true/false, yes/no, or 1/0), got {s}"
        ))),
    }
}
fn set_start(c: &mut FormatConfig, s: &str) -> Result<(), FormatError> {
    if s.eq_ignore_ascii_case("a") || s.eq_ignore_ascii_case("auto") {
        c.auto_start_indent = true
    } else {
        c.start_indent = parse_num(s)?;
        c.auto_start_indent = false
    }
    Ok(())
}
fn set_short(c: &mut FormatConfig, ch: char, n: usize) -> Result<(), FormatError> {
    match ch {
        'a' => c.construct_indents.associate = n,
        'b' => c.construct_indents.block = n,
        'c' => c.case_indent = n,
        'd' => c.construct_indents.do_ = n,
        'e' => c.entry_indent = n,
        'E' => c.construct_indents.r#enum = n,
        'f' => c.construct_indents.if_ = n,
        'F' => c.construct_indents.forall = n,
        'j' => c.construct_indents.interface = n,
        'm' => c.construct_indents.module = n,
        'r' => c.construct_indents.procedure = n,
        's' => c.construct_indents.select = n,
        't' => c.construct_indents.r#type = n,
        'w' => c.construct_indents.where_ = n,
        'x' => c.construct_indents.critical = n,
        _ => return Err(FormatError::InvalidOption(format!("-{ch}"))),
    }
    Ok(())
}
fn set_construct(c: &mut FormatConfig, n: &str, v: usize) -> Result<(), FormatError> {
    match n {
        "associate" => c.construct_indents.associate = v,
        "block" => c.construct_indents.block = v,
        "case" => c.case_indent = v,
        "contains" => c.contains_indent = v,
        "do" => c.construct_indents.do_ = v,
        "entry" => c.entry_indent = v,
        "enum" => c.construct_indents.r#enum = v,
        "forall" => c.construct_indents.forall = v,
        "if" => c.construct_indents.if_ = v,
        "interface" => c.construct_indents.interface = v,
        "module" => c.construct_indents.module = v,
        "procedure" => c.construct_indents.procedure = v,
        "select" => c.construct_indents.select = v,
        "type" => c.construct_indents.r#type = v,
        "where" => c.construct_indents.where_ = v,
        "critical" => c.construct_indents.critical = v,
        _ => return Err(FormatError::InvalidOption(format!("--indent-{n}"))),
    }
    Ok(())
}

pub fn usage() -> &'static str {
    "Usage: forformat [OPTIONS] < input > output\n\n\
Free-form Fortran formatter.\n\
\x20\x20-i<n>, --indent=<n>                 global indentation (default 3)\n\
\x20\x20-i-, --indent=none                  leave indentation unchanged\n\
\x20\x20-I<n>, --start-indent=<n>           starting indentation\n\
\x20\x20-Ia, --start-indent=a               infer starting indentation\n\
\x20\x20-M<n>, --max-indent=<n>             maximum indentation (0 = unlimited)\n\
\x20\x20-k<n>, --indent-continuation=<n>    continuation indentation\n\
\x20\x20-K, --indent-ampersand[=<BOOL>]     indent leading continuation ampersands\n\
\x20\x20--align-paren[=<n>]                align continuation lines at parentheses\n\
\x20\x20--include-left=<BOOL>              put INCLUDE at the starting indent\n\
\x20\x20-Rr, -RR, --refactor-end[=<BOOL>|upcase]  complete END definition statements\n\
\x20\x20--ws-remred[=<n>]                  reduce redundant whitespace\n\
\x20\x20--align-declarations=<BOOL>        shrink space to align `::` blocks (default 1)\n\
\x20\x20--align-comments=<BOOL>            shrink space to align trailing comment blocks (default 0)\n\
\x20\x20--lastindent, -lastusable           print query result instead of source\n\
\x20\x20--query-format                     print free/fixed for each input and exit\n\
\x20\x20--all-files [directory]             format this checkout's tracked sources; submodules are context only\n\
\x20\x20<paths>, --all [directory]          format explicit files or all tracked sources recursively\n\
\x20\x20--no-submodules[=<BOOL>]            omit submodule sources from targets and project context\n\
\x20\x20--context-path=<directory>           limit project context to sources beneath DIRECTORY; repeatable\n\
\x20\x20--stdin                             read source from stdin (default without paths)\n\
\x20\x20--project-context=<path>            treat stdin as belonging to the Git project containing PATH; a source-file PATH identifies stdin as that file and shadows its on-disk contents\n\
\x20\x20--stdout                            write one file's result to stdout\n\
\x20\x20--isolated                          do not scan repository sources for case resolution\n\
\x20\x20--check                             exit 1 if selected files would change\n\
\x20\x20--diff                              print unified diffs and exit 1 if changed\n\
\x20\x20--show-files                        print selected files without formatting\n\
\x20\x20--exclude=<glob>                    exclude tracked sources from --all-files, --all, and project scanning (repeatable)\n\
\x20\x20--extend-exclude=<glob>             add to the exclusions instead of replacing them (repeatable)\n\
  Query modes cannot be combined with path-update, --check, or --diff.\n\
\x20\x20--indent-only                      findent-compatible indentation only\n\
\x20\x20--full                             full formatting: normalization and wrapping (default)\n\
\x20\x20--normalize-only                   normalization without structural layout\n\
\x20\x20--wrap[=<BOOL>], --no-wrap[=<BOOL>] reflow over-long statements (full mode)\n\
\x20\x20--line-length=<n>                  wrapping budget (default 120)\n\
\x20\x20--keyword-case=<lower|upper|preserve>\n\
                                      recognized keyword case (default lower)\n\
\x20\x20--relational-symbols=<BOOL>        rewrite `.eq.` and friends as `==` (default true)\n\
\x20\x20--array-brackets=<BOOL>            rewrite `(/ ... /)` as `[ ... ]` (default true)\n\
\x20\x20--compact-multiplicative=<BOOL>    no spaces around binary `*`, `/`, `**` (default true)\n\
\x20\x20--split-compound-keywords=<BOOL>   write `endif` as `end if` (default true)\n\
\x20\x20--join-goto=<BOOL>                 write `go to` as `goto` (default true)\n\
\x20\x20--strip-empty-args=<BOOL>          strip empty SUBROUTINE definition arg lists (default true)\n\
\x20\x20--remove-redundant-parens=<BOOL>   remove redundant parentheses (default true)\n\
\x20\x20--remove-terminal-return=<BOOL>    remove terminal procedure RETURN (default true)\n\
\x20\x20--program-unit-spacing=<BOOL>      canonical blank lines around program units (default true)\n\
\x20\x20--max-blank-lines=<n|preserve>     blank-line cap (default 2)\n\
\x20\x20--delimiter-spacing=<BOOL>         normalize spaces after delimiters (default true)\n\
\x20\x20--comment-spacing=<BOOL>           normalize the gap before a trailing `!` (default true)\n\
\x20\x20--continuation-markers=<BOOL>      normalize continuation markers and OpenMP sentinels (default true)\n\
\x20\x20-D NAME[=VALUE], --define=...      define a macro name (repeatable)\n\
\x20\x20--uppercase-single-l[=<BOOL>]      uppercase a lone `l` used as a name\n\
\x20\x20--config=<path>                    use a project TOML configuration explicitly\n\
\x20\x20--no-config                        ignore project TOML configuration\n\
\x20\x20-h, --help                         show this help\n\
\x20\x20-v, --version                      show version\n\
Automatic fixed/free input detection is enabled by default; use -ifree or\n\
\x20\x20--input-format=free to force free-form input. Fixed-form output remains unsupported."
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_inner, Command, DEFAULT_EXCLUDES};
    use std::path::PathBuf;

    fn run(args: &[&str]) -> crate::config::FormatConfig {
        let mut argv = vec!["forformat".to_string()];
        argv.extend(args.iter().map(|arg| (*arg).to_string()));
        match parse(argv).unwrap() {
            Command::Run(invocation) => invocation.config,
            _ => panic!("expected a formatting command"),
        }
    }

    fn selection(args: &[&str]) -> Result<(bool, Option<PathBuf>), crate::error::FormatError> {
        let mut argv = vec!["forformat".to_string()];
        argv.extend(args.iter().map(|arg| (*arg).to_string()));
        parse_inner(argv).and_then(|parsed| parsed.config_selection.resolve())
    }

    #[test]
    fn config_selection_uses_the_parser_value_grammar() {
        let consumed = parse_inner([
            "forformat".to_string(),
            "-D".to_string(),
            "--config".to_string(),
            "foo.toml".to_string(),
        ])
        .unwrap();
        assert_eq!(consumed.config_selection.resolve().unwrap(), (false, None));
        match consumed.command {
            Command::Run(invocation) => {
                assert_eq!(invocation.config.defines[0].name, "--config");
                assert_eq!(invocation.paths, [PathBuf::from("foo.toml")]);
            }
            _ => panic!("expected run"),
        }

        assert_eq!(
            selection(&["-D", "VALUE", "--config", "foo.toml"]).unwrap(),
            (false, Some(PathBuf::from("foo.toml")))
        );
        assert_eq!(selection(&["--define=--config"]).unwrap(), (false, None));
        assert_eq!(selection(&["-D", "--no-config"]).unwrap(), (false, None));
    }

    #[test]
    fn config_selection_preserves_spellings_conflicts_and_termination() {
        assert_eq!(
            selection(&["--config=path.toml"]).unwrap(),
            (false, Some(PathBuf::from("path.toml")))
        );
        assert_eq!(
            selection(&["--config", "path.toml"]).unwrap(),
            (false, Some(PathBuf::from("path.toml")))
        );
        assert_eq!(selection(&["--no-config"]).unwrap(), (true, None));

        assert!(matches!(
            selection(&["--config=one.toml", "--config=two.toml"]),
            Err(crate::error::FormatError::InvalidOption(message))
                if message == "--config may be specified only once"
        ));
        assert!(matches!(
            selection(&["--no-config", "--config=path.toml"]),
            Err(crate::error::FormatError::InvalidOption(message))
                if message == "--config cannot be combined with --no-config"
        ));

        let terminated = parse_inner([
            "forformat".to_string(),
            "--".to_string(),
            "--config".to_string(),
            "foo.toml".to_string(),
        ])
        .unwrap();
        assert_eq!(
            terminated.config_selection.resolve().unwrap(),
            (false, None)
        );
        match terminated.command {
            Command::Run(invocation) => {
                assert_eq!(
                    invocation.paths,
                    [PathBuf::from("--config"), PathBuf::from("foo.toml")]
                );
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn overloaded_short_options_accept_attached_and_separated_values() {
        let attached = run(&["-i4", "-C-", "-k5", "-M9"]);
        let separated = run(&["-i", "4", "-C", "-", "-k", "5", "-M", "9"]);
        assert_eq!(attached, separated);
        assert!(!run(&["-i-"]).apply_indent);
        assert!(run(&["-Ia"]).auto_start_indent);
        assert!(parse(["forformat".to_string(), "-iauto".to_string()].into_iter()).is_ok());
    }

    #[test]
    fn optional_values_are_not_taken_from_the_next_argument() {
        let bare = run(&["--align_paren"]);
        assert!(bare.align_paren);
        assert_eq!(bare.align_paren_value, 1);
        assert!(!run(&["--align-paren=0"]).align_paren);
        assert_eq!(run(&["--align_paren=4"]).align_paren_value, 4);
        assert!(run(&["--ws-remred"]).ws_remred);
        assert_eq!(run(&["--ws_remred=0"]).ws_remred_value, 0);
    }

    #[test]
    fn no_submodules_accepts_explicit_boolean_values() {
        let parse_no_submodules = |args: &[&str]| {
            let argv = std::iter::once("forformat")
                .chain(args.iter().copied())
                .map(str::to_owned);
            let Command::Run(invocation) = parse(argv).unwrap() else {
                panic!("expected a formatting command");
            };
            invocation.no_submodules
        };

        assert!(parse_no_submodules(&["--no-config", "--no-submodules"]));
        assert!(parse_no_submodules(&[
            "--no-config",
            "--no-submodules=true"
        ]));
        assert!(!parse_no_submodules(&[
            "--no-config",
            "--no-submodules=false"
        ]));
    }

    #[test]
    fn optional_boolean_switches_accept_bare_and_explicit_values() {
        let parse_config = |args: &[&str]| {
            let argv = std::iter::once("forformat")
                .chain(args.iter().copied())
                .map(str::to_owned);
            let Command::Run(invocation) = parse(argv).unwrap() else {
                panic!("expected a formatting command");
            };
            invocation.config
        };

        assert!(parse_config(&["--no-config", "--wrap"]).wrap.enabled);
        assert!(!parse_config(&["--no-config", "--wrap=false"]).wrap.enabled);
        assert!(!parse_config(&["--no-config", "--no-wrap"]).wrap.enabled);
        assert!(
            parse_config(&["--no-config", "--no-wrap=false"])
                .wrap
                .enabled
        );
        assert!(
            !parse_config(&["--no-config", "--no-wrap=true"])
                .wrap
                .enabled
        );
        assert!(parse_config(&["--no-config", "--indent-ampersand"]).indent_ampersand);
        assert!(!parse_config(&["--no-config", "--indent-ampersand=false"]).indent_ampersand);
    }

    #[test]
    fn legacy_boolean_and_refactor_options_honor_explicit_values() {
        let parse_invocation = |args: &[&str]| {
            let argv = std::iter::once("forformat")
                .chain(args.iter().copied())
                .map(str::to_owned);
            let Command::Run(invocation) = parse(argv).unwrap() else {
                panic!("expected a formatting command");
            };
            invocation
        };

        let invocation = parse_invocation(&[
            "--no-config",
            "--last-indent",
            "--last-usable",
            "--uppercase-single-l=false",
            "--refactor-end=false",
        ]);
        assert!(invocation.config.last_indent);
        assert!(invocation.config.last_usable);
        assert!(!invocation.config.uppercase_single_l);
        assert!(!invocation.config.refactor_end);

        let invocation = parse_invocation(&[
            "--no-config",
            "--uppercase-single-l=true",
            "--refactor-end=true",
        ]);
        assert!(invocation.config.uppercase_single_l);
        assert!(invocation.config.refactor_end);
        assert!(!invocation.config.uppercase_end);

        let invocation = parse_invocation(&["--no-config", "--refactor-end=upcase"]);
        assert!(invocation.config.refactor_end);
        assert!(invocation.config.uppercase_end);

        let invocation = parse_invocation(&["--no-config", "--refactor-procedures=false"]);
        assert!(!invocation.config.refactor_end);

        for option in ["last-indent", "last-usable"] {
            let argument = format!("--{option}=false");
            assert!(parse(["forformat".to_string(), argument].into_iter()).is_err());
        }
        assert!(parse(
            [
                "forformat".to_string(),
                "--refactor-end=unexpected".to_string()
            ]
            .into_iter()
        )
        .is_err());
    }

    #[test]
    fn valueless_workflow_and_mode_switches_reject_attached_values() {
        for option in [
            "all",
            "all-files",
            "stdin",
            "stdout",
            "isolated",
            "check",
            "diff",
            "show-files",
            "query-format",
            "full",
            "indent-only",
            "normalize-only",
            "no-config",
        ] {
            let argument = format!("--{option}=false");
            let argv = ["forformat".to_string(), argument.clone()];
            assert!(parse(argv.into_iter()).is_err(), "{argument} was accepted");
        }
    }

    #[test]
    fn format_aliases_and_option_termination_are_explicit() {
        assert!(matches!(
            parse(["forformat".to_string(), "--input_format=free".to_string()].into_iter())
                .unwrap(),
            Command::Run(_)
        ));
        assert!(matches!(
            parse(["forformat".to_string(), "--output-format=same".to_string()].into_iter())
                .unwrap(),
            Command::Run(_)
        ));
        let terminated =
            parse(["forformat".to_string(), "--".to_string(), "-i4".to_string()]).unwrap();
        match terminated {
            Command::Run(invocation) => assert_eq!(invocation.paths, [PathBuf::from("-i4")]),
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn project_context_implies_stdin_and_rejects_file_workflows() {
        for args in [
            &["--project-context", ".", "source.f90"][..],
            &["--project-context", ".", "--check"][..],
            &["--query-format", "--project-context", "."][..],
            &["--project-context", ".", "--project-context", "."][..],
        ] {
            assert!(
                parse(
                    std::iter::once("forformat".to_string())
                        .chain(args.iter().map(|arg| (*arg).to_string())),
                )
                .is_err(),
                "{args:?}"
            );
        }
        let Command::Run(invocation) = parse(
            ["forformat", "--stdin", "--project-context", "."]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected run")
        };
        assert!(invocation.stdin);
        assert!(parse(
            ["forformat", "--project-context", "."]
                .into_iter()
                .map(str::to_owned),
        )
        .is_ok());
        assert!(parse(
            [
                "forformat",
                "--isolated",
                "source.f90",
                "--context-path",
                "src"
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .is_err());
    }

    #[test]
    fn long_and_short_construct_aliases_produce_the_same_config() {
        assert_eq!(
            run(&[
                "-a5", "-b6", "-c2", "-d7", "-e4", "-E8", "-F9", "-f5", "-j6", "-m7", "-r8", "-s9",
                "-t4", "-w5", "-x6"
            ]),
            run(&[
                "--indent-associate=5",
                "--indent-block=6",
                "--indent-case=2",
                "--indent-do=7",
                "--indent-entry=4",
                "--indent-enum=8",
                "--indent-forall=9",
                "--indent-if=5",
                "--indent-interface=6",
                "--indent-module=7",
                "--indent-procedure=8",
                "--indent-select=9",
                "--indent-type=4",
                "--indent-where=5",
                "--indent-critical=6",
            ])
        );
        assert_eq!(run(&["-C-"]), run(&["--indent-contains=restart"]));
        assert_eq!(run(&["-K"]), run(&["--indent-ampersand"]));
        assert_eq!(run(&["-Rr"]), run(&["--refactor-end"]));
        assert_eq!(run(&["-RR"]), run(&["--refactor-end=upcase"]));
    }

    #[test]
    fn long_alias_spellings_and_optional_values_have_a_matrix() {
        let aliases = [
            ("--indent_associate=5", "--indent-associate=5"),
            ("--indent_contains=restart", "--indent-contains=restart"),
            ("--include_left=1", "--include-left=1"),
            ("--label_left=0", "--label-left=0"),
            ("--input_format=free", "--input-format=free"),
            ("--output_format=same", "--output-format=same"),
        ];
        for (underscore, hyphen) in aliases {
            assert_eq!(
                run(&[underscore]),
                run(&[hyphen]),
                "{underscore} != {hyphen}"
            );
        }

        assert_eq!(run(&["--align_paren"]).align_paren_value, 1);
        assert!(!run(&["--align-paren=0"]).align_paren);
        assert!(run(&["--align_paren=1"]).align_paren);
        assert_eq!(run(&["--align-paren=7"]).align_paren_value, 7);
        assert_eq!(run(&["--ws_remred"]).ws_remred_value, 1);
        assert!(!run(&["--ws-remred=0"]).ws_remred);
        assert!(run(&["--ws_remred=1"]).ws_remred);
    }

    #[test]
    fn every_documented_long_option_family_parses_with_a_value() {
        let options = [
            "--indent=4",
            "--start-indent=2",
            "--indent-contains=4",
            "--include-left=1",
            "--label-left=0",
            "--max-indent=12",
            "--openmp=0",
            "--indent-ampersand",
            "--indent-continuation=7",
            "--align-paren=3",
            "--ws-remred=1",
            "--align-declarations=0",
            "--align-comments=1",
            "--indent-changeteam=4",
            "--indent-associate=4",
            "--indent-block=4",
            "--indent-case=4",
            "--indent-contains=4",
            "--indent-do=4",
            "--indent-entry=4",
            "--indent-enum=4",
            "--indent-forall=4",
            "--indent-if=4",
            "--indent-interface=4",
            "--indent-module=4",
            "--indent-procedure=4",
            "--indent-select=4",
            "--indent-type=4",
            "--indent-where=4",
            "--refactor-end",
            "--refactor-end=upcase",
            "--last-indent",
            "--last-usable",
            "--input-format=free",
            "--output-format=free",
            "--output-format=same",
        ];
        for option in options {
            assert!(
                parse(["forformat".to_string(), option.to_string()].into_iter()).is_ok(),
                "{option}"
            );
        }
    }

    #[test]
    fn missing_and_invalid_long_values_have_stable_diagnostics() {
        for option in [
            "--indent",
            "--start-indent",
            "--indent-contains",
            "--include-left",
            "--label-left",
            "--max-indent",
            "--openmp",
            "--indent-continuation",
            "--indent-changeteam",
            "--indent-if",
        ] {
            match parse(["forformat".to_string(), option.to_string()]) {
                Err(crate::error::FormatError::InvalidOption(message)) => {
                    assert_eq!(message, "missing option value", "{option}")
                }
                _ => panic!("unexpected result for {option}"),
            }
        }
        for option in ["--include-left=2", "--label-left=maybe", "--openmp=maybe"] {
            assert!(matches!(
                parse(["forformat".to_string(), option.to_string()].into_iter()),
                Err(crate::error::FormatError::InvalidOption(_))
            ));
        }
    }

    #[test]
    fn rejected_long_values_have_stable_diagnostics() {
        for (args, expected) in [
            (&["--input-format=unknown"][..], "--input-format=unknown"),
            (&["--output_format=unknown"][..], "--output-format=unknown"),
            (
                &["--align_paren=-1"][..],
                "expected non-negative integer, got -1",
            ),
            (
                &["--ws_remred=no"][..],
                "expected non-negative integer, got no",
            ),
        ] {
            match parse(
                std::iter::once("forformat".to_string())
                    .chain(args.iter().map(|arg| (*arg).to_string())),
            ) {
                Err(crate::error::FormatError::InvalidOption(value)) => {
                    assert_eq!(value, expected)
                }
                _ => panic!("unexpected result for {args:?}"),
            }
        }
    }

    #[test]
    fn mode_and_full_format_options_parse_and_do_not_collide_with_construct_names() {
        use crate::config::FormatMode;
        assert_eq!(run(&[]).mode, FormatMode::Full);
        assert_eq!(run(&["--full"]).mode, FormatMode::Full);
        assert_eq!(run(&["--normalize-only"]).mode, FormatMode::NormalizeOnly);
        // `--indent-only` must not be read as `--indent-<construct>`.
        assert_eq!(
            run(&["--full", "--indent-only"]).mode,
            FormatMode::IndentOnly
        );
        assert_eq!(run(&["--indent_only"]).mode, FormatMode::IndentOnly);

        assert!(run(&[]).wrap.enabled);
        assert!(!run(&["--no-wrap"]).wrap.enabled);
        assert!(!run(&["--wrap=0"]).wrap.enabled);
        assert_eq!(run(&["--line-length=100"]).wrap.line_length, 100);
        assert!(run(&["--uppercase-single-l"]).uppercase_single_l);
    }

    #[test]
    fn style_options_parse_all_values_and_underscore_spellings() {
        use crate::config::KeywordCase;

        assert!(run(&[]).style.join_goto);
        assert!(run(&[]).style.split_compound_keywords);

        let config = run(&[
            "--keyword_case",
            "upper",
            "--relational-symbols=0",
            "--array_brackets",
            "0",
            "--compact-multiplicative=0",
            "--join_goto=0",
            "--split-compound-keywords",
            "0",
            "--strip_empty_args=0",
            "--remove-redundant-parens",
            "0",
            "--remove_terminal_return=0",
            "--program-unit-spacing",
            "0",
            "--max_blank_lines",
            "preserve",
            "--delimiter-spacing=0",
            "--comment_spacing",
            "0",
            "--continuation-markers=0",
        ]);
        assert_eq!(config.style.keyword_case, KeywordCase::Upper);
        assert!(!config.style.relational_symbols);
        assert!(!config.style.array_brackets);
        assert!(!config.style.compact_multiplicative);
        assert!(!config.style.join_goto);
        assert!(!config.style.split_compound_keywords);
        assert!(!config.style.strip_empty_args);
        assert!(!config.style.remove_redundant_parens);
        assert!(!config.style.remove_terminal_return);
        assert!(!config.style.program_unit_spacing);
        assert_eq!(config.style.max_blank_lines, None);
        assert!(!config.style.delimiter_spacing);
        assert!(!config.style.comment_spacing);
        assert!(!config.style.continuation_markers);

        fn style_bool(config: &crate::config::FormatConfig, option: &str) -> bool {
            match option {
                "relational-symbols" => config.style.relational_symbols,
                "array-brackets" => config.style.array_brackets,
                "compact-multiplicative" => config.style.compact_multiplicative,
                "join-goto" => config.style.join_goto,
                "split-compound-keywords" => config.style.split_compound_keywords,
                "strip-empty-args" => config.style.strip_empty_args,
                "remove-redundant-parens" => config.style.remove_redundant_parens,
                "remove-terminal-return" => config.style.remove_terminal_return,
                "program-unit-spacing" => config.style.program_unit_spacing,
                "delimiter-spacing" => config.style.delimiter_spacing,
                "comment-spacing" => config.style.comment_spacing,
                "continuation-markers" => config.style.continuation_markers,
                _ => unreachable!(),
            }
        }
        let bools = [
            "relational-symbols",
            "array-brackets",
            "compact-multiplicative",
            "join-goto",
            "split-compound-keywords",
            "strip-empty-args",
            "remove-redundant-parens",
            "remove-terminal-return",
            "program-unit-spacing",
            "delimiter-spacing",
            "comment-spacing",
            "continuation-markers",
        ];
        for option in bools {
            for spelling in [option.to_string(), option.replace('-', "_")] {
                let zero = format!("--{spelling}=0");
                let one = format!("--{spelling}=1");
                assert!(!style_bool(&run(&[zero.as_str()]), option));
                assert!(style_bool(&run(&[one.as_str()]), option));
            }
        }

        assert_eq!(run(&["--max-blank-lines=0"]).style.max_blank_lines, Some(0));
        assert_eq!(
            run(&["--max-blank-lines", "7"]).style.max_blank_lines,
            Some(7)
        );
    }

    #[test]
    fn style_options_report_the_option_bad_value_and_allowed_values() {
        let result = parse([
            "forformat".to_string(),
            "--strip-empty-args=maybe".to_string(),
        ]);
        assert!(matches!(
            result,
            Err(crate::error::FormatError::InvalidOption(message))
                if message == "expected boolean (true/false, yes/no, or 1/0), got maybe"
        ));
        let result = parse([
            "forformat".to_string(),
            "--keyword-case=invalid".to_string(),
        ]);
        assert!(matches!(
            result,
            Err(crate::error::FormatError::InvalidOption(message))
                if message.contains("keyword-case")
                    && message.contains("invalid")
                    && message.contains("allowed values")
        ));
        assert!(matches!(
            parse(["forformat", "--max-blank-lines=bad"].into_iter().map(str::to_owned)),
            Err(crate::error::FormatError::InvalidOption(message))
                if message.contains("bad")
        ));
        for option in [
            "--keyword-case",
            "--array-brackets",
            "--compact-multiplicative",
            "--join-goto",
            "--split-compound-keywords",
            "--strip-empty-args",
            "--relational-symbols",
            "--remove-redundant-parens",
            "--remove-terminal-return",
            "--program-unit-spacing",
            "--delimiter-spacing",
            "--comment-spacing",
            "--continuation-markers",
            "--max-blank-lines",
        ] {
            assert!(matches!(
                parse(["forformat".to_string(), option.to_string()].into_iter()),
                Err(crate::error::FormatError::InvalidOption(message))
                    if message == "missing option value"
            ));
        }
    }

    #[test]
    fn alignment_reduction_toggles_default_on_for_declarations_and_off_for_comments() {
        let default = run(&[]);
        assert!(default.align_declarations);
        assert!(!default.align_comments);
        assert!(!run(&["--align-declarations=0"]).align_declarations);
        assert!(run(&["--align-comments=1"]).align_comments);
    }

    #[test]
    fn macro_definitions_accumulate_in_order_from_both_spellings() {
        let config = run(&["-DFIRST", "-D", "Second=2", "--define=Third"]);
        let names: Vec<&str> = config
            .defines
            .iter()
            .map(|define| define.name.as_str())
            .collect();
        assert_eq!(names, ["FIRST", "Second", "Third"]);
        assert_eq!(config.defines[1].value.as_deref(), Some("2"));
        assert_eq!(config.defines[0].value, None);
    }

    #[test]
    fn exclude_accepts_repeatable_separated_and_normalized_spellings() {
        let mut argv = vec!["forformat".to_string()];
        argv.extend(
            [
                "--no-config",
                "--exclude=vendor/**",
                "--exclude",
                "generated/",
            ]
            .iter()
            .map(|arg| (*arg).to_string()),
        );
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected a formatting command");
        };
        assert_eq!(invocation.exclude_patterns(), ["vendor/**", "generated/"]);

        let Command::Run(invocation) = parse(
            ["forformat", "--no_config", "--EXCLUDE=vendor/**"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap() else {
            panic!("expected a formatting command");
        };
        assert_eq!(invocation.exclude_patterns(), ["vendor/**"]);
    }

    #[test]
    fn extend_exclude_adds_to_the_set_exclude_selects() {
        let run = |args: &[&str]| {
            let argv = std::iter::once("forformat")
                .chain(args.iter().copied())
                .map(str::to_owned);
            let Command::Run(invocation) = parse(argv).unwrap() else {
                panic!("expected a formatting command");
            };
            invocation
        };

        // With no `--exclude`, the defaults stand and `--extend-exclude` adds
        // to them.
        let invocation = run(&["--no-config", "--extend-exclude=generated/"]);
        assert!(invocation.exclude.is_none());
        assert_eq!(invocation.extend_exclude, ["generated/"]);
        assert_eq!(
            invocation.exclude_patterns(),
            DEFAULT_EXCLUDES
                .iter()
                .map(|s| (*s).to_string())
                .chain(["generated/".to_string()])
                .collect::<Vec<_>>()
        );

        // `--exclude` replaces the defaults; the two options compose.
        let invocation = run(&[
            "--no-config",
            "--exclude=vendor/",
            "--extend_exclude=generated/",
        ]);
        assert_eq!(invocation.exclude_patterns(), ["vendor/", "generated/"]);
    }

    #[test]
    fn unsupported_and_invalid_cli_paths_have_stable_categories() {
        assert!(matches!(
            parse(["forformat".to_string(), "-ifixed".to_string()].into_iter()),
            Err(crate::error::FormatError::Unsupported(_))
        ));
        assert!(matches!(
            parse(["forformat".to_string(), "--not-an-option".to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
        assert!(matches!(
            parse(["forformat".to_string(), "-i".to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
        assert!(matches!(
            parse(["forformat".to_string(), "--include-left=maybe".to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
    }

    #[test]
    fn single_dash_long_option_typos_explain_the_required_spelling() {
        for (typo, expected) in [
            (
                "-all",
                "-all (did you mean --all? Long options use two dashes.)",
            ),
            (
                "-indent_module=0",
                "-indent_module=0 (did you mean --indent_module=0? Long options use two dashes.)",
            ),
        ] {
            match parse(["forformat".to_string(), typo.to_string()]) {
                Err(crate::error::FormatError::InvalidOption(message)) => {
                    assert_eq!(message, expected)
                }
                _ => panic!("unexpected result for {typo}"),
            }
        }

        // These are valid legacy spellings and must not be mistaken for
        // single-dash long options.
        assert!(parse(["forformat".to_string(), "-i4".to_string()].into_iter()).is_ok());
        assert!(parse(["forformat".to_string(), "-ifree".to_string()].into_iter()).is_ok());
    }

    #[test]
    fn file_workflow_flags_and_query_mode_validation_are_explicit() {
        use crate::config::FormatMode;
        let parsed = parse(
            ["forformat", "--full", "--all"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        match parsed {
            Command::Run(invocation) => {
                assert!(invocation.all);
                assert_eq!(invocation.config.mode, FormatMode::Full);
            }
            _ => panic!("expected run"),
        }
        assert!(parse(
            ["forformat", "-lastindent", "--check", "x.f90"]
                .into_iter()
                .map(str::to_owned),
        )
        .is_err());
        assert!(parse(
            ["forformat", "--stdout", "x.f90", "y.f90"]
                .into_iter()
                .map(str::to_owned),
        )
        .is_err());
    }
}
