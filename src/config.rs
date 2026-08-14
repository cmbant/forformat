use std::{fmt, fs, path::Path};

/// What the formatter is allowed to change.
///
/// `IndentOnly` is the findent 4.3.7 contract and stays byte-exact forever
/// (I6).  `Full` adds normalization and wrapping.  `NormalizeOnly` runs the
/// text passes without the structural layout, which is how a single
/// normalization rule is compared against the frozen Python reference while the
/// port is incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatMode {
    #[default]
    IndentOnly,
    NormalizeOnly,
    Full,
}

impl FormatMode {
    pub fn normalizes(self) -> bool {
        matches!(self, FormatMode::NormalizeOnly | FormatMode::Full)
    }

    pub fn lays_out(self) -> bool {
        matches!(self, FormatMode::IndentOnly | FormatMode::Full)
    }
}

/// Line-length policy for the reflow engine.
///
/// `line_length` is a budget, not a guarantee: a statement with no safe break
/// point is emitted long and reported by the corpus check rather than split
/// unsafely (I5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapConfig {
    pub enabled: bool,
    pub line_length: usize,
}

impl Default for WrapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            line_length: 120,
        }
    }
}

/// A `-D NAME[=VALUE]` definition.  Macro names outrank every other case rule
/// (I4), so this list is part of the case configuration, not just of any CPP
/// evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDefine {
    pub name: String,
    pub value: Option<String>,
}

/// Load formatter settings from the nearest project configuration.
///
/// The standalone format is a top-level TOML table in `.forformat.toml`.
/// `.findent.toml` is accepted as a compatibility spelling. Python projects
/// can keep the same settings in `[tool.forformat]` in `pyproject.toml`.
/// Values are returned as long-option arguments so the CLI parser remains the
/// single implementation of option names, aliases, and validation.
pub(crate) fn config_args(
    start: &Path,
    explicit: Option<&Path>,
) -> Result<Vec<String>, crate::error::FormatError> {
    if let Some(path) = explicit {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            start.join(path)
        };
        let table_path = (path.file_name() == Some(std::ffi::OsStr::new("pyproject.toml")))
            .then_some("tool.forformat");
        return read_config(&path, table_path);
    }

    let mut directory = start.to_path_buf();
    loop {
        for name in [".forformat.toml", ".findent.toml"] {
            let path = directory.join(name);
            if path.is_file() {
                return read_config(&path, None);
            }
        }
        let pyproject = directory.join("pyproject.toml");
        if pyproject.is_file() {
            let args = read_config(&pyproject, Some("tool.forformat"))?;
            if !args.is_empty() {
                return Ok(args);
            }
        }
        if !directory.pop() {
            break;
        }
    }
    Ok(Vec::new())
}

fn read_config(
    path: &Path,
    table_path: Option<&str>,
) -> Result<Vec<String>, crate::error::FormatError> {
    let text = fs::read_to_string(path).map_err(|error| config_error(path, error))?;
    let document = text
        .parse::<toml::Value>()
        .map_err(|error| config_error(path, error))?;
    let table = if let Some(table_path) = table_path {
        let mut value = &document;
        for component in table_path.split('.') {
            let Some(next) = value.get(component) else {
                return Ok(Vec::new());
            };
            value = next;
        }
        value
    } else {
        &document
    };
    let Some(table) = table.as_table() else {
        return Err(crate::error::FormatError::InvalidOption(format!(
            "configuration {} must contain a TOML table",
            path.display()
        )));
    };

    let mut args = Vec::new();
    for (key, value) in table {
        let key = key.replace('_', "-").to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "all"
                | "check"
                | "config"
                | "diff"
                | "isolated"
                | "last-indent"
                | "last-usable"
                | "no-config"
                | "stdin"
                | "stdout"
        ) {
            return Err(crate::error::FormatError::InvalidOption(format!(
                "configuration key `{key}` is a command-line workflow option"
            )));
        }
        if key == "mode" {
            let mode = value.as_str().ok_or_else(|| {
                crate::error::FormatError::InvalidOption(format!(
                    "configuration key `mode` in {} must be a string",
                    path.display()
                ))
            })?;
            let option = match mode {
                "full" => "--full",
                "indent-only" | "indent_only" => "--indent-only",
                "normalize-only" | "normalize_only" => "--normalize-only",
                other => {
                    return Err(crate::error::FormatError::InvalidOption(format!(
                        "configuration key `mode` in {} has unknown value `{other}`",
                        path.display()
                    )))
                }
            };
            args.push(option.to_string());
            continue;
        }
        if key == "define" || key == "defines" {
            match value {
                toml::Value::String(spec) => args.push(format!("--define={spec}")),
                toml::Value::Array(specs) => {
                    for spec in specs {
                        let spec = spec.as_str().ok_or_else(|| {
                            crate::error::FormatError::InvalidOption(format!(
                                "configuration key `{key}` in {} must contain strings",
                                path.display()
                            ))
                        })?;
                        args.push(format!("--define={spec}"));
                    }
                }
                _ => {
                    return Err(crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must be a string or array of strings",
                        path.display()
                    )))
                }
            }
            continue;
        }
        if matches!(
            key.as_str(),
            "uppercase-single-l" | "refactor-end" | "no-wrap"
        ) && value.as_bool() == Some(false)
        {
            // These are enabling switches in the CLI; false is already the
            // default and has no corresponding disabling option.
            continue;
        }
        let value = config_value(value, &key, path)?;
        args.push(format!("--{key}={value}"));
    }
    Ok(args)
}

fn config_value(
    value: &toml::Value,
    key: &str,
    path: &Path,
) -> Result<String, crate::error::FormatError> {
    let value = match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) if *value >= 0 => value.to_string(),
        toml::Value::Boolean(value) => {
            if matches!(key, "align-paren" | "ws-remred") {
                if *value { "1" } else { "0" }.to_string()
            } else if key == "indent-continuation" {
                if *value { "default" } else { "none" }.to_string()
            } else {
                value.to_string()
            }
        }
        toml::Value::Integer(_) => {
            return Err(crate::error::FormatError::InvalidOption(format!(
                "configuration key `{key}` in {} must be non-negative",
                path.display()
            )))
        }
        _ => {
            return Err(crate::error::FormatError::InvalidOption(format!(
                "configuration key `{key}` in {} must be a string, integer, or boolean",
                path.display()
            )))
        }
    };
    Ok(value)
}

fn config_error(path: &Path, error: impl fmt::Display) -> crate::error::FormatError {
    crate::error::FormatError::InvalidOption(format!("configuration {}: {error}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatConfig {
    /// Selects which stages of the pipeline run.  Everything below this field
    /// is either shared or specific to one stage.
    pub mode: FormatMode,
    /// Full-mode reflow policy.
    pub wrap: WrapConfig,
    /// Command-line macro definitions, in the order given.
    pub defines: Vec<MacroDefine>,
    /// Uppercase a lone `l` used as a name, a Python-side option retained for
    /// compatibility with the reference formatter.
    pub uppercase_single_l: bool,
    pub indent: usize,
    pub apply_indent: bool,
    pub start_indent: usize,
    pub auto_start_indent: bool,
    pub max_indent: usize,
    pub label_left: bool,
    pub include_left: bool,
    pub indent_continuation: bool,
    pub continuation_indent: usize,
    pub indent_ampersand: bool,
    /// Whether parenthesis alignment is enabled.  `align_paren_value` keeps
    /// the optional numeric CLI value without breaking boolean API callers.
    pub align_paren: bool,
    pub align_paren_value: usize,
    pub openmp: bool,
    pub contains_restart: bool,
    pub contains_indent: usize,
    pub case_indent: usize,
    pub entry_indent: usize,
    pub refactor_end: bool,
    pub uppercase_end: bool,
    /// Whether redundant-whitespace reduction is enabled.  The numeric mode
    /// is retained in `ws_remred_value` for the optional CLI contract.
    pub ws_remred: bool,
    pub ws_remred_value: usize,
    pub last_indent: bool,
    pub last_usable: bool,
    pub construct_indents: ConstructIndents,
    /// Whether step 17 may shrink the whitespace before a declaration's `::`
    /// to fit a shared block column. Declarations are hand-aligned often
    /// enough that this defaults on.
    pub align_declarations: bool,
    /// Whether step 17b may shrink the whitespace before a trailing comment
    /// to fit a shared run column. Off by default: unlike a declaration's
    /// `::`, a comment's gap is not a separator with an owed minimum, so
    /// there is no default width to fall back to if the author's is not
    /// kept — shrinking it is a layout opinion this formatter does not
    /// impose unless asked.
    pub align_comments: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructIndents {
    pub associate: usize,
    pub block: usize,
    pub changeteam: usize,
    pub critical: usize,
    pub do_: usize,
    pub r#enum: usize,
    pub forall: usize,
    pub if_: usize,
    pub interface: usize,
    pub module: usize,
    pub procedure: usize,
    pub select: usize,
    pub r#type: usize,
    pub where_: usize,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            mode: FormatMode::Full,
            wrap: WrapConfig::default(),
            defines: Vec::new(),
            uppercase_single_l: false,
            indent: 3,
            apply_indent: true,
            start_indent: 0,
            auto_start_indent: false,
            max_indent: 100,
            label_left: true,
            include_left: false,
            indent_continuation: true,
            continuation_indent: 3,
            indent_ampersand: false,
            align_paren: false,
            align_paren_value: 0,
            openmp: true,
            contains_restart: false,
            contains_indent: 3,
            case_indent: 2,
            entry_indent: 2,
            refactor_end: false,
            uppercase_end: false,
            ws_remred: false,
            ws_remred_value: 0,
            last_indent: false,
            last_usable: false,
            construct_indents: ConstructIndents::with_indent(3),
            align_declarations: true,
            align_comments: false,
        }
    }
}

impl ConstructIndents {
    pub const fn with_indent(n: usize) -> Self {
        Self {
            associate: n,
            block: n,
            changeteam: n,
            critical: n,
            do_: n,
            r#enum: n,
            forall: n,
            if_: n,
            interface: n,
            module: n,
            procedure: n,
            select: n,
            r#type: n,
            where_: n,
        }
    }
    pub fn set_all(&mut self, n: usize) {
        *self = Self::with_indent(n);
    }
}
