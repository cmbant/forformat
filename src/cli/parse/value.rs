use crate::{
    config::{FormatConfig, MacroDefine},
    error::FormatError,
};

pub(super) struct ArgCursor<I> {
    inner: I,
}

impl<I> ArgCursor<I>
where
    I: Iterator<Item = String>,
{
    pub(super) fn new(inner: I) -> Self {
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
