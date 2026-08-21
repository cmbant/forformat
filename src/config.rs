use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ConfigArguments {
    pub args: Vec<String>,
    pub context_paths: Vec<crate::cli::ContextPath>,
}

/// What the formatter is allowed to change.
///
/// `Full` is the product default and adds normalization and wrapping.
/// `IndentOnly` is the findent 4.3.7 contract and stays byte-exact forever
/// (I6).  `NormalizeOnly` runs the text passes without the structural layout,
/// which is how a single normalization rule can be tested independently of
/// structural layout.  `CanonicalizeOnly` is `NormalizeOnly` minus presentation
/// whitespace: token and spelling canonicalization without reflowing the
/// author's spacing.
///
/// The four modes are one field on purpose.  Canonicalization used to be
/// `NormalizeOnly` plus a separate `style.normalize_whitespace = false`, which
/// made `--canonicalize-only --normalize-only` depend on argument order — the
/// second option reset the whitespace half of the first.  Whether whitespace is
/// presentation the formatter owns is a property of the mode, so it is answered
/// by [`FormatMode::normalizes_whitespace`] and cannot disagree with `mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    IndentOnly,
    NormalizeOnly,
    CanonicalizeOnly,
    Full,
}

impl FormatMode {
    pub fn normalizes(self) -> bool {
        matches!(
            self,
            FormatMode::NormalizeOnly | FormatMode::CanonicalizeOnly | FormatMode::Full
        )
    }

    pub fn lays_out(self) -> bool {
        matches!(self, FormatMode::IndentOnly | FormatMode::Full)
    }

    /// Whether presentation whitespace belongs to the formatter in this mode.
    ///
    /// Only canonicalization says no: it keeps the authored spacing and line
    /// structure while still canonicalizing tokens and spellings.  This governs
    /// interior whitespace only — whitespace at end of line is invisible rather
    /// than a formatting choice, and every mode removes it.
    pub fn normalizes_whitespace(self) -> bool {
        !matches!(self, FormatMode::CanonicalizeOnly)
    }

    /// Whether the reflow wrapper runs, and therefore whether `rewrap` can mean
    /// anything.
    pub fn wraps(self) -> bool {
        matches!(self, FormatMode::Full)
    }
}

/// Line-length policy for the reflow engine.
///
/// `line_length` is a budget, not a guarantee: a statement with no safe break
/// point is emitted long and reported by a decline diagnostic rather than split
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

/// Case policy for recognized Fortran keywords and intrinsic spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordCase {
    Lower,
    Upper,
    Preserve,
}

/// Opinionated full-mode normalization choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleConfig {
    pub keyword_case: KeywordCase,
    /// Uppercase reserved OpenMP sentinels (`!$OMP`, `!$OMPX`) and the
    /// directive keywords that follow them, independently of `keyword_case`.
    ///
    /// `!$OMP PARALLEL DO` in otherwise lowercase source is the near-universal
    /// convention, so this defaults on and a directive does not follow
    /// `keyword_case` unless it is turned off.  It governs reserved directives
    /// only: a conditional-compilation `!$ ` line is ordinary Fortran wearing a
    /// sentinel, so its body follows `keyword_case` like any other statement.
    pub openmp_case: bool,
    pub relational_symbols: bool,
    pub array_brackets: bool,
    pub compact_multiplicative: bool,
    pub join_goto: bool,
    pub split_compound_keywords: bool,
    pub strip_empty_args: bool,
    pub remove_redundant_parens: bool,
    /// Drop semicolons that separate no pair of non-empty statements.
    pub normalize_semicolons: bool,
    pub remove_terminal_return: bool,
    pub program_unit_spacing: bool,
    pub max_blank_lines: Option<usize>,
    pub delimiter_spacing: bool,
    pub comment_spacing: bool,
    pub continuation_markers: bool,
}

impl StyleConfig {
    /// Case policy for a reserved OpenMP sentinel and its directive keywords.
    ///
    /// `openmp_case` is a switch rather than its own [`KeywordCase`] because
    /// there is only one convention worth naming separately: uppercase
    /// directives over lowercase Fortran.  Turning it off hands directives back
    /// to `keyword_case`, which is also how `--keyword-case=preserve` reaches
    /// them.
    pub fn openmp_keyword_case(&self) -> KeywordCase {
        if self.openmp_case {
            KeywordCase::Upper
        } else {
            self.keyword_case
        }
    }
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Lower,
            openmp_case: true,
            relational_symbols: true,
            array_brackets: true,
            compact_multiplicative: true,
            join_goto: true,
            split_compound_keywords: true,
            strip_empty_args: true,
            remove_redundant_parens: true,
            normalize_semicolons: true,
            remove_terminal_return: true,
            program_unit_spacing: true,
            max_blank_lines: Some(2),
            delimiter_spacing: true,
            comment_spacing: true,
            continuation_markers: true,
        }
    }
}

/// Load formatter settings from the nearest project configuration.
///
/// The standalone format is a top-level TOML table in `.forformat.toml`.
/// `.findent.toml` is accepted as a compatibility spelling. Python projects
/// can keep the same settings in `[tool.forformat]` in `pyproject.toml`.
/// Ordinary values are returned as long-option arguments so the CLI parser
/// remains the single implementation of option names, aliases, and validation.
/// Context paths retain their configuration-directory origin separately.
pub(crate) fn config_args(
    start: &Path,
    explicit: Option<&Path>,
) -> Result<ConfigArguments, crate::error::FormatError> {
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
            let config = read_config(&pyproject, Some("tool.forformat"))?;
            if !config.args.is_empty() || !config.context_paths.is_empty() {
                return Ok(config);
            }
        }
        if !directory.pop() {
            break;
        }
    }
    Ok(ConfigArguments::default())
}

fn read_config(
    path: &Path,
    table_path: Option<&str>,
) -> Result<ConfigArguments, crate::error::FormatError> {
    let text = fs::read_to_string(path).map_err(|error| config_error(path, error))?;
    let document = text
        .parse::<toml::Value>()
        .map_err(|error| config_error(path, error))?;
    let table = if let Some(table_path) = table_path {
        let mut value = &document;
        for component in table_path.split('.') {
            let Some(next) = value.get(component) else {
                return Ok(ConfigArguments::default());
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

    let mut entries: Vec<_> = table.iter().collect();
    entries.sort_by_key(|(key, _)| {
        let normalized = normalize_config_key(key);
        (config_key_priority(&normalized), normalized)
    });

    let mut args = Vec::new();
    let mut context_paths = Vec::new();
    for (key, value) in entries {
        let key = normalize_config_key(key);
        if matches!(
            key.as_str(),
            "all"
                | "all-files"
                | "check"
                | "config"
                | "diff"
                | "isolated"
                | "last-indent"
                | "last-usable"
                | "no-config"
                | "project-context"
                | "query-format"
                | "stdin"
                | "stdout"
                | "show-files"
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
                "canonicalize-only" | "canonicalize_only" => "--canonicalize-only",
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
        if key == "exclude" || key == "extend-exclude" {
            let specs = value.as_array().ok_or_else(|| {
                crate::error::FormatError::InvalidOption(format!(
                    "configuration key `{key}` in {} must be an array of strings",
                    path.display()
                ))
            })?;
            for spec in specs {
                let spec = spec.as_str().ok_or_else(|| {
                    crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must contain strings",
                        path.display()
                    ))
                })?;
                if spec.is_empty() {
                    return Err(crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must not contain empty patterns",
                        path.display()
                    )));
                }
                // The two keys are not synonyms: `exclude` selects the set and
                // `extend-exclude` adds to it, so they must survive as distinct
                // options for the command line to layer over them correctly.
                args.push(format!("--{key}={spec}"));
            }
            continue;
        }
        if key == "context-paths" {
            let paths = value.as_array().ok_or_else(|| {
                crate::error::FormatError::InvalidOption(format!(
                    "configuration key `{key}` in {} must be an array of strings",
                    path.display()
                ))
            })?;
            for context_path in paths {
                let context_path = context_path.as_str().ok_or_else(|| {
                    crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must contain strings",
                        path.display()
                    ))
                })?;
                if context_path.is_empty() {
                    return Err(crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must not contain empty paths",
                        path.display()
                    )));
                }
                context_paths.push(crate::cli::ContextPath {
                    path: PathBuf::from(context_path),
                    base: Some(config_directory(path)),
                });
            }
            continue;
        }
        if key == "no-submodules" {
            let enabled = value.as_bool().ok_or_else(|| {
                crate::error::FormatError::InvalidOption(format!(
                    "configuration key `{key}` in {} must be a boolean",
                    path.display()
                ))
            })?;
            if enabled {
                args.push("--no-submodules".to_string());
            }
            continue;
        }
        let value = config_value(value, &key, path)?;
        args.push(format!("--{key}={value}"));
    }
    Ok(ConfigArguments {
        args,
        context_paths,
    })
}

fn config_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

// TOML tables are BTreeMaps, but CLI option order is meaningful: --indent
// resets the values later supplied by the construct-specific --indent-*
// options. Keep every broad/resetting config key in this list so adding a new
// one requires an explicit precedence decision instead of silently inheriting
// alphabetical ordering.
const RESETTING_CONFIG_KEYS: &[&str] = &["indent"];

fn normalize_config_key(key: &str) -> String {
    key.replace('_', "-").to_ascii_lowercase()
}

fn config_key_priority(key: &str) -> usize {
    RESETTING_CONFIG_KEYS
        .iter()
        .position(|resetting| *resetting == key)
        .unwrap_or(RESETTING_CONFIG_KEYS.len())
}

#[cfg(test)]
mod tests {
    use super::{config_args, config_key_priority, FormatConfig, FormatMode, KeywordCase};
    use crate::cli::{parse, Command};
    use std::fs;

    fn config_from_text(name: &str, text: &str) -> FormatConfig {
        let path = std::env::temp_dir().join(format!(
            "forformat-config-{name}-{}.toml",
            std::process::id()
        ));
        fs::write(&path, text).unwrap();
        let config_args = config_args(&path, Some(&path)).unwrap();
        let _ = fs::remove_file(&path);

        let mut args = vec!["forformat".to_string(), "--no-config".to_string()];
        args.extend(config_args.args);
        match parse(args).unwrap() {
            Command::Run(invocation) => invocation.config,
            _ => panic!("expected a formatting command"),
        }
    }

    #[test]
    fn specific_indent_config_overrides_the_global_reset() {
        let config = config_from_text("specific-indent", "indent = 4\nindent-select = 2\n");

        assert_eq!(config.indent, 4);
        assert_eq!(config.construct_indents.select, 2);
    }

    #[test]
    fn global_indent_config_resets_every_per_construct_indent() {
        let config = config_from_text("global-indent", "indent = 4\n");

        assert_eq!(config.construct_indents.associate, 4);
        assert_eq!(config.construct_indents.block, 4);
        assert_eq!(config.construct_indents.changeteam, 4);
        assert_eq!(config.construct_indents.critical, 4);
        assert_eq!(config.construct_indents.do_, 4);
        assert_eq!(config.construct_indents.r#enum, 4);
        assert_eq!(config.construct_indents.forall, 4);
        assert_eq!(config.construct_indents.if_, 4);
        assert_eq!(config.construct_indents.interface, 4);
        assert_eq!(config.construct_indents.module, 4);
        assert_eq!(config.construct_indents.procedure, 4);
        assert_eq!(config.construct_indents.select, 4);
        assert_eq!(config.construct_indents.r#type, 4);
        assert_eq!(config.construct_indents.where_, 4);
        assert_eq!(config.contains_indent, 4);
        assert_eq!(config.continuation_indent, 4);
        assert_eq!(config.case_indent, 2);
        assert_eq!(config.entry_indent, 2);
    }

    #[test]
    fn resetting_config_keys_have_explicit_precedence() {
        let config = config_from_text("precedence", "indent-select = 2\nindent = 4\n");
        assert!(config_key_priority("indent") < config_key_priority("indent-select"));
        assert_eq!(config.construct_indents.select, 2);
    }

    #[test]
    fn project_context_is_not_a_configuration_key() {
        let path = std::env::temp_dir().join(format!(
            "forformat-project-context-config-{}.toml",
            std::process::id()
        ));
        fs::write(&path, "project-context = '.'\n").unwrap();
        let error = config_args(&path, Some(&path)).unwrap_err();
        let _ = fs::remove_file(&path);
        assert!(error
            .to_string()
            .contains("configuration key `project-context` is a command-line workflow option"));
    }

    #[test]
    fn query_format_is_not_a_configuration_key() {
        let path = std::env::temp_dir().join(format!(
            "forformat-query-format-config-{}.toml",
            std::process::id()
        ));
        fs::write(&path, "query_format = true\n").unwrap();
        let error = config_args(&path, Some(&path)).unwrap_err();
        let _ = fs::remove_file(&path);
        assert!(error
            .to_string()
            .contains("configuration key `query-format` is a command-line workflow option"));
    }

    #[test]
    fn canonicalize_mode_loads_from_toml() {
        let config = config_from_text("canonicalize", "mode = 'canonicalize-only'\n");
        assert_eq!(config.mode, FormatMode::CanonicalizeOnly);
        assert!(!config.mode.normalizes_whitespace());
        assert!(config.mode.normalizes());
        assert!(!config.mode.lays_out());
    }

    #[test]
    fn rewrap_loads_from_toml_alongside_full_mode() {
        let config = config_from_text("full-rewrap", "mode = 'full'\nrewrap = true\n");
        assert_eq!(config.mode, FormatMode::Full);
        assert!(config.rewrap);
        assert!(config.wrap.enabled);
    }

    #[test]
    fn rewrap_is_rejected_by_a_mode_that_cannot_wrap() {
        // The wrapper is full-mode only, so a configuration that asks a
        // no-layout mode to repack continuations is contradictory rather than
        // silently inert.
        let path = std::env::temp_dir().join(format!(
            "forformat-canonicalize-rewrap-{}.toml",
            std::process::id()
        ));
        fs::write(&path, "mode = 'canonicalize-only'\nrewrap = true\n").unwrap();
        let config_args = config_args(&path, Some(&path)).unwrap();
        let _ = fs::remove_file(&path);

        let mut args = vec!["forformat".to_string(), "--no-config".to_string()];
        args.extend(config_args.args);
        let Err(error) = parse(args) else {
            panic!("rewrap in a no-layout mode should be rejected");
        };
        assert!(
            error.to_string().contains("--rewrap requires full mode"),
            "{error}"
        );
    }

    #[test]
    fn style_keys_load_from_the_standalone_toml_shape() {
        let config = config_from_text(
            "style-options",
            "keyword_case = 'upper'\nopenmp-case = false\nrelational_symbols = false\ncompact_multiplicative = false\narray-brackets = false\njoin-goto = false\nsplit-compound-keywords = false\nstrip_empty_args = false\nremove-redundant-parens = false\nnormalize-semicolons = false\nremove_terminal_return = false\nprogram-unit-spacing = false\nmax_blank_lines = 'preserve'\ndelimiter-spacing = false\ncomment_spacing = false\ncontinuation-markers = false\n",
        );
        assert_eq!(config.style.keyword_case, KeywordCase::Upper);
        assert!(!config.style.openmp_case);
        assert!(!config.style.relational_symbols);
        assert!(!config.style.compact_multiplicative);
        assert!(!config.style.array_brackets);
        assert!(!config.style.join_goto);
        assert!(!config.style.split_compound_keywords);
        assert!(!config.style.strip_empty_args);
        assert!(!config.style.remove_redundant_parens);
        assert!(!config.style.normalize_semicolons);
        assert!(!config.style.remove_terminal_return);
        assert!(!config.style.program_unit_spacing);
        assert_eq!(config.style.max_blank_lines, None);
        assert!(!config.style.delimiter_spacing);
        assert!(!config.style.comment_spacing);
        assert!(!config.style.continuation_markers);
    }

    #[test]
    fn style_keys_load_from_pyproject_and_cli_scalars_override_them() {
        let directory =
            std::env::temp_dir().join(format!("forformat-pyproject-dir-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let pyproject = directory.join("pyproject.toml");
        fs::write(
            &pyproject,
            "[tool.forformat]\nkeyword-case = 'upper'\ncompact_multiplicative = false\nstrip_empty_args = false\nmax-blank-lines = 1\n",
        )
        .unwrap();
        let from_pyproject = config_args(&pyproject, Some(&pyproject)).unwrap();
        let _ = fs::remove_file(&pyproject);
        let _ = fs::remove_dir(&directory);
        let mut argv = vec!["forformat".to_string(), "--no-config".to_string()];
        argv.extend(from_pyproject.args);
        argv.extend([
            "--keyword-case=preserve".to_string(),
            "--strip-empty-args=1".to_string(),
            "--max-blank-lines=0".to_string(),
        ]);
        let crate::cli::Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected run")
        };
        assert_eq!(invocation.config.style.keyword_case, KeywordCase::Preserve);
        assert!(!invocation.config.style.compact_multiplicative);
        assert!(invocation.config.style.strip_empty_args);
        assert_eq!(invocation.config.style.max_blank_lines, Some(0));
    }

    #[test]
    fn boolean_switches_keep_false_values_when_loaded_from_toml() {
        let config = config_from_text(
            "boolean-switches",
            "uppercase_single_l = false\nrefactor_end = false\nno_wrap = false\nrewrap = false\n",
        );

        assert!(!config.uppercase_single_l);
        assert!(!config.refactor_end);
        assert!(!config.rewrap);
        assert!(config.wrap.enabled);
    }
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
    /// Repack already-continued eligible statements through the normal wrapper.
    pub rewrap: bool,
    /// Command-line macro definitions, in the order given.
    pub defines: Vec<MacroDefine>,
    /// Full-mode lexical and structural style choices.
    pub style: StyleConfig,
    /// Uppercase a lone `l` used as a name, a Python-side option retained for
    /// compatibility with established command-line profiles.
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
            rewrap: false,
            defines: Vec::new(),
            style: StyleConfig::default(),
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
