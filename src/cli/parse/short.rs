use super::ArgCursor;
use crate::{
    cli::{
        draft::DraftInvocation,
        options::{Construct, OptionId},
        settings::{self, FormatSetting},
    },
    error::FormatError,
};

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

    if let Some(construct) = short_construct(option) {
        let value = cursor.required_short(option, attached)?;
        let id = OptionId::IndentConstruct(construct);
        draft.push_format(
            id,
            FormatSetting::ConstructIndent(construct, settings::parse_num(&value)?),
        );
        return Ok(());
    }

    match option {
        'C' => {
            let value = cursor.required_short(option, attached)?;
            let setting = if value == "-" {
                FormatSetting::RestartContains
            } else {
                FormatSetting::SetContainsIndent(settings::parse_num(&value)?)
            };
            draft.push_format(OptionId::IndentContains, setting);
        }
        'i' => {
            if attached == "-" {
                draft.push_format(OptionId::Indent, FormatSetting::DisableIndent);
            } else if attached == "free" {
                draft.options.force_free_input = Some(true);
            } else if attached == "auto" {
                draft.options.force_free_input = Some(false);
            } else if attached == "fixed" {
                return Err(FormatError::Unsupported(
                    "fixed-form input/output is not supported".into(),
                ));
            } else {
                let value = cursor.required_short(option, attached)?;
                draft.push_format(
                    OptionId::Indent,
                    FormatSetting::SetIndent(settings::parse_num(&value)?),
                );
            }
        }
        'I' => {
            let value = cursor.required_short(option, attached)?;
            if let Some(setting) =
                settings::parse_format_setting(OptionId::StartIndent, Some(&value))?
            {
                draft.push_format(OptionId::StartIndent, setting);
            }
        }
        'k' => {
            let value = cursor.required_short(option, attached)?;
            if let Some(setting) =
                settings::parse_format_setting(OptionId::IndentContinuation, Some(&value))?
            {
                draft.push_format(OptionId::IndentContinuation, setting);
            }
        }
        'D' => {
            let value = cursor.required_short(option, attached)?;
            if let Some(setting) = settings::parse_format_setting(OptionId::Define, Some(&value))? {
                draft.push_format(OptionId::Define, setting);
            }
        }
        'K' => draft.push_format(
            OptionId::IndentAmpersand,
            FormatSetting::IndentAmpersand(true),
        ),
        'l' => {
            let value = cursor.required_short(option, attached)?;
            draft.push_format(
                OptionId::LabelLeft,
                FormatSetting::LabelLeft(settings::parse_bool(&value)?),
            );
        }
        'M' => {
            let value = cursor.required_short(option, attached)?;
            draft.push_format(
                OptionId::MaxIndent,
                FormatSetting::MaxIndent(settings::parse_num(&value)?),
            );
        }
        'R' => {
            let setting = match attached {
                "r" => FormatSetting::RefactorEnd {
                    enabled: true,
                    uppercase: false,
                },
                "R" => FormatSetting::RefactorEnd {
                    enabled: true,
                    uppercase: true,
                },
                _ => return Err(FormatError::InvalidOption(arg)),
            };
            draft.push_format(OptionId::RefactorEnd, setting);
        }
        _ => return Err(FormatError::InvalidOption(arg)),
    }
    Ok(())
}

fn short_construct(option: char) -> Option<Construct> {
    Some(match option {
        'a' => Construct::Associate,
        'b' => Construct::Block,
        'c' => Construct::Case,
        'd' => Construct::Do,
        'e' => Construct::Entry,
        'E' => Construct::Enum,
        'f' => Construct::If,
        'F' => Construct::Forall,
        'j' => Construct::Interface,
        'm' => Construct::Module,
        'r' => Construct::Procedure,
        's' => Construct::Select,
        't' => Construct::Type,
        'w' => Construct::Where,
        'x' => Construct::Critical,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::{cli::parse, error::FormatError};

    #[test]
    fn rejects_undocumented_refactor_short_forms() {
        for argument in ["-R", "-Rx"] {
            assert!(matches!(
                parse(["forformat".to_string(), argument.to_string()].into_iter()),
                Err(FormatError::InvalidOption(message)) if message == argument
            ));
        }
    }
}
