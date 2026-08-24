use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

const CANONICALIZE_INDENT: &str = "--canonicalize-and-indent";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ConfigArguments {
    pub args: Vec<String>,
    pub context_paths: Vec<crate::cli::ContextPath>,
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
                "canonicalize-and-indent" | "canonicalize_and_indent" => CANONICALIZE_INDENT,
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

#[cfg(test)]
mod tests {
    use super::{config_args, config_key_priority};
    use crate::{
        cli::{parse, Command},
        config::{FormatConfig, FormatMode, KeywordCase},
    };
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
            "keyword_case = 'upper'\nopenmp-case = false\nrelational_symbols = false\ncompact_multiplicative = false\narray-brackets = false\njoin-goto = false\nsplit-compound-keywords = false\nstrip_empty_args = false\nremove-redundant-parens = false\nnormalize-semicolons = false\nremove_terminal_return = false\nprogram_unit_spacing = false\nmax_blank_lines = 'preserve'\ndelimiter-spacing = false\ncomment_spacing = false\ncontinuation-markers = false\n",
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
