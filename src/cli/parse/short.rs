use super::{parse_bool, parse_num, push_define, set_short, set_start, ArgCursor};
use crate::{cli::draft::DraftInvocation, error::FormatError};

pub(super) fn parse_short<I>(
    arg: String,
    cursor: &mut ArgCursor<I>,
    draft: &mut DraftInvocation,
) -> Result<(), FormatError>
where
    I: Iterator<Item = String>,
{
    let bytes = arg.as_bytes();
    let option = bytes[1] as char;
    let attached = &arg[2..];
    match option {
        'a' | 'b' | 'c' | 'd' | 'e' | 'E' | 'f' | 'F' | 'j' | 'm' | 'r' | 's' | 't'
        | 'w' | 'x' => {
            let value = cursor.required_short(option, attached)?;
            set_short(&mut draft.config, option, parse_num(&value)?)?;
        }
        'C' => {
            let value = cursor.required_short(option, attached)?;
            if value == "-" {
                draft.config.contains_restart = true
            } else {
                draft.config.contains_indent = parse_num(&value)?;
                draft.config.contains_restart = false
            }
        }
        'i' => {
            if attached == "-" {
                draft.config.apply_indent = false
            } else if attached == "free" {
                draft.force_free_input = true;
            } else if attached == "auto" {
                draft.force_free_input = false;
            } else if attached == "fixed" {
                return Err(FormatError::Unsupported(
                    "fixed-form input/output is not supported".into(),
                ));
            } else {
                let value = cursor.required_short(option, attached)?;
                draft.config.indent = parse_num(&value)?;
                draft.config.construct_indents.set_all(draft.config.indent);
                draft.config.contains_indent = draft.config.indent;
                draft.config.continuation_indent = draft.config.indent;
                draft.config.case_indent = draft
                    .config
                    .indent
                    .saturating_sub(draft.config.indent / 2);
                draft.config.entry_indent = draft.config.case_indent
            }
        }
        'I' => {
            let value = cursor.required_short(option, attached)?;
            set_start(&mut draft.config, &value)?;
        }
        'k' => {
            let value = cursor.required_short(option, attached)?;
            if value == "-" || value == "none" {
                draft.config.indent_continuation = false
            } else if value == "d" || value == "default" {
                draft.config.indent_continuation = true
            } else {
                draft.config.continuation_indent = parse_num(&value)?
            }
        }
        'D' => {
            let value = cursor.required_short(option, attached)?;
            push_define(&mut draft.config, &value);
        }
        'K' => draft.config.indent_ampersand = true,
        'l' => {
            let value = cursor.required_short(option, attached)?;
            draft.config.label_left = parse_bool(&value)?;
        }
        'M' => {
            let value = cursor.required_short(option, attached)?;
            draft.config.max_indent = parse_num(&value)?;
        }
        'R' => {
            draft.config.refactor_end = true;
            draft.config.uppercase_end = attached == "R"
        }
        _ => return Err(FormatError::InvalidOption(arg)),
    }
    Ok(())
}
