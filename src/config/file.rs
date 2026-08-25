use crate::cli::{
    options::{self, ConfigMapping, OptionId, OptionSpec, ValueKind},
    settings::{self, OptionLayer},
    ContextPath,
};
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ConfigArguments {
    pub(crate) layer: OptionLayer,
    /// Whether this configuration terminates upward `pyproject.toml` discovery.
    ///
    /// This deliberately tracks the old synthetic-argv notion of presence,
    /// independently of whether the typed layer changes runtime state. For
    /// example, `output_format = "same"` validates successfully without storing
    /// a setting, but historically still stopped discovery at that pyproject.
    pub(crate) discovery_match: bool,
}

/// Load formatter settings from the nearest project configuration.
///
/// The standalone format is a top-level TOML table in `.forformat.toml`.
/// `.findent.toml` is accepted as a compatibility spelling. Python projects
/// can keep the same settings in `[tool.forformat]` in `pyproject.toml`.
/// Both TOML and argv are decoded into the same typed option layer; callers
/// merge those layers explicitly instead of replaying configuration as argv.
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
            if config.discovery_match {
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
    let document =
        toml::from_str::<toml::Value>(&text).map_err(|error| config_error(path, error))?;
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

    let mut layer = OptionLayer::default();
    let mut discovery_match = false;
    for (raw_key, value) in table {
        let key = options::normalize_long(raw_key);
        if key == "mode" {
            parse_mode(value, path, &mut layer)?;
            discovery_match = true;
            continue;
        }

        let spec = config_spec(&key)?;
        match spec.id {
            OptionId::Define => {
                parse_defines(value, &key, path, spec.id, &mut layer)?;
                discovery_match |= match value {
                    toml::Value::String(_) => true,
                    toml::Value::Array(values) => !values.is_empty(),
                    _ => false,
                };
            }
            OptionId::Exclude | OptionId::ExtendExclude => {
                parse_exclusions(value, &key, path, spec.id, &mut layer)?;
                discovery_match |= value
                    .as_array()
                    .is_some_and(|patterns| !patterns.is_empty());
            }
            OptionId::ContextPath => {
                parse_context_paths(value, &key, path, &mut layer)?;
                discovery_match |= value.as_array().is_some_and(|paths| !paths.is_empty());
            }
            OptionId::NoSubmodules => {
                let enabled = value.as_bool().ok_or_else(|| {
                    crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must be a boolean",
                        path.display()
                    ))
                })?;
                if enabled {
                    layer.no_submodules = Some(true);
                    discovery_match = true;
                }
            }
            OptionId::InputFormat => {
                let value = config_scalar(value, &key, path, spec)?;
                layer.force_free_input = Some(settings::parse_input_format(&value)?);
                discovery_match = true;
            }
            OptionId::OutputFormat => {
                let value = config_scalar(value, &key, path, spec)?;
                settings::parse_output_format(&value)?;
                discovery_match = true;
            }
            _ => {
                let value = config_scalar(value, &key, path, spec)?;
                if let Some(setting) = settings::parse_format_setting(spec.id, Some(&value))? {
                    layer.push_format(spec.id, setting);
                }
                discovery_match = true;
            }
        }
    }
    Ok(ConfigArguments {
        layer,
        discovery_match,
    })
}

fn config_spec(key: &str) -> Result<&'static OptionSpec, crate::error::FormatError> {
    if let Some(spec) = options::lookup_config(key, None) {
        return Ok(spec);
    }
    if key == "context-path" {
        return Err(crate::error::FormatError::InvalidOption(
            "configuration key `context-path` is not supported; use `context-paths = [\"...\"]`"
                .into(),
        ));
    }
    if let Some(spec) = options::lookup_any_long(key) {
        if matches!(spec.config, ConfigMapping::None)
            && !matches!(spec.id, OptionId::Help | OptionId::Version)
        {
            return Err(crate::error::FormatError::InvalidOption(format!(
                "configuration key `{key}` is a command-line workflow option"
            )));
        }
    }
    // This is the same category and spelling the old synthetic-argv path
    // produced after handing an unknown key to the long-option parser.
    Err(crate::error::FormatError::InvalidOption(format!("--{key}")))
}

fn parse_mode(
    value: &toml::Value,
    path: &Path,
    layer: &mut OptionLayer,
) -> Result<(), crate::error::FormatError> {
    let mode = value.as_str().ok_or_else(|| {
        crate::error::FormatError::InvalidOption(format!(
            "configuration key `mode` in {} must be a string",
            path.display()
        ))
    })?;
    let normalized = mode.replace('_', "-");
    let Some(spec) = options::lookup_config("mode", Some(&normalized)) else {
        return Err(crate::error::FormatError::InvalidOption(format!(
            "configuration key `mode` in {} has unknown value `{mode}`",
            path.display()
        )));
    };
    if let Some(setting) = settings::parse_format_setting(spec.id, None)? {
        layer.push_format(spec.id, setting);
    }
    Ok(())
}

fn parse_defines(
    value: &toml::Value,
    key: &str,
    path: &Path,
    id: OptionId,
    layer: &mut OptionLayer,
) -> Result<(), crate::error::FormatError> {
    let mut push = |define: &str| -> Result<(), crate::error::FormatError> {
        if let Some(setting) = settings::parse_format_setting(id, Some(define))? {
            layer.push_format(id, setting);
        }
        Ok(())
    };
    match value {
        toml::Value::String(define) => push(define),
        toml::Value::Array(defines) => {
            for define in defines {
                let define = define.as_str().ok_or_else(|| {
                    crate::error::FormatError::InvalidOption(format!(
                        "configuration key `{key}` in {} must contain strings",
                        path.display()
                    ))
                })?;
                push(define)?;
            }
            Ok(())
        }
        _ => Err(crate::error::FormatError::InvalidOption(format!(
            "configuration key `{key}` in {} must be a string or array of strings",
            path.display()
        ))),
    }
}

fn parse_exclusions(
    value: &toml::Value,
    key: &str,
    path: &Path,
    id: OptionId,
    layer: &mut OptionLayer,
) -> Result<(), crate::error::FormatError> {
    let specs = value.as_array().ok_or_else(|| {
        crate::error::FormatError::InvalidOption(format!(
            "configuration key `{key}` in {} must be an array of strings",
            path.display()
        ))
    })?;
    for pattern in specs {
        let pattern = pattern.as_str().ok_or_else(|| {
            crate::error::FormatError::InvalidOption(format!(
                "configuration key `{key}` in {} must contain strings",
                path.display()
            ))
        })?;
        if pattern.is_empty() {
            return Err(crate::error::FormatError::InvalidOption(format!(
                "configuration key `{key}` in {} must not contain empty patterns",
                path.display()
            )));
        }
        if id == OptionId::Exclude {
            layer.push_exclude(pattern.to_string());
        } else {
            layer.extend_exclude.push(pattern.to_string());
        }
    }
    Ok(())
}

fn parse_context_paths(
    value: &toml::Value,
    key: &str,
    path: &Path,
    layer: &mut OptionLayer,
) -> Result<(), crate::error::FormatError> {
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
        layer.push_context_path(ContextPath {
            path: PathBuf::from(context_path),
            base: Some(config_directory(path)),
        });
    }
    Ok(())
}

fn config_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn config_scalar(
    value: &toml::Value,
    key: &str,
    path: &Path,
    spec: &OptionSpec,
) -> Result<String, crate::error::FormatError> {
    let value = match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) if *value >= 0 => value.to_string(),
        toml::Value::Boolean(value) => match spec.value_kind {
            ValueKind::OptionalNonNegative | ValueKind::WhitespaceReduction => {
                if *value { "1" } else { "0" }.to_string()
            }
            ValueKind::ContinuationIndent => if *value { "default" } else { "none" }.to_string(),
            _ => value.to_string(),
        },
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
mod discovery_tests {
    use super::config_args;
    use std::fs;

    #[test]
    fn noop_scalar_pyproject_stops_parent_discovery() {
        let root = std::env::temp_dir().join(format!(
            "forformat-pyproject-discovery-{}",
            std::process::id()
        ));
        let child = root.join("child");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&child).unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[tool.forformat]\nkeyword_case = 'upper'\n",
        )
        .unwrap();
        fs::write(
            child.join("pyproject.toml"),
            "[tool.forformat]\noutput_format = 'same'\n",
        )
        .unwrap();

        let config = config_args(&child, None).unwrap();
        assert!(config.discovery_match);
        assert!(
            config.layer.is_empty(),
            "the child pyproject should stop discovery without applying the parent"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_repeatable_pyproject_still_falls_through_to_parent() {
        let root = std::env::temp_dir().join(format!(
            "forformat-empty-pyproject-discovery-{}",
            std::process::id()
        ));
        let child = root.join("child");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&child).unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[tool.forformat]\nkeyword_case = 'upper'\n",
        )
        .unwrap();
        fs::write(
            child.join("pyproject.toml"),
            "[tool.forformat]\ndefines = []\n",
        )
        .unwrap();

        let config = config_args(&child, None).unwrap();
        assert!(config.discovery_match);
        assert!(
            !config.layer.is_empty(),
            "an empty repeatable setting should preserve the old parent fallthrough"
        );

        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod tests;
