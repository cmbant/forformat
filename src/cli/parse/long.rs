use super::{
    parse_bool, parse_num, parse_style_choice, push_define, reject_value, set_construct, set_start,
    ArgCursor, ConfigSelection,
};
use crate::{
    cli::{draft::DraftInvocation, options::OptionId},
    config::{FormatMode, KeywordCase},
    error::FormatError,
};
use std::path::PathBuf;

pub(super) fn parse_long<I>(
    name: &str,
    mut value: Option<String>,
    cursor: &mut ArgCursor<I>,
    draft: &mut DraftInvocation,
    config_selection: &mut ConfigSelection,
) -> Result<(), FormatError>
where
    I: Iterator<Item = String>,
{
    let Some(option) = crate::cli::options::lookup_long(name) else {
        if let Some(construct) = name.strip_prefix("indent-") {
            let value = parse_num(&cursor.required_long(&mut value)?)?;
            return set_construct(&mut draft.config, construct, value);
        }
        return Err(FormatError::InvalidOption(format!("--{name}")));
    };

    match option {
        OptionId::Config => {
            let path = cursor.required_long(&mut value)?;
            config_selection.explicit.push(path);
        }
        OptionId::NoConfig => {
            reject_value(name, &value)?;
            config_selection.no_config = true;
        }
        OptionId::Indent => {
            let value = cursor.required_long(&mut value)?;
            if value == "none" {
                draft.config.apply_indent = false
            } else {
                draft.config.indent = parse_num(&value)?;
                draft.config.construct_indents.set_all(draft.config.indent);
                draft.config.contains_indent = draft.config.indent;
                draft.config.continuation_indent = draft.config.indent;
                draft.config.case_indent =
                    draft.config.indent.saturating_sub(draft.config.indent / 2);
                draft.config.entry_indent = draft.config.case_indent
            }
        }
        OptionId::StartIndent => {
            let value = cursor.required_long(&mut value)?;
            set_start(&mut draft.config, &value)?;
        }
        OptionId::IndentContains => {
            let value = cursor.required_long(&mut value)?;
            if value == "restart" {
                draft.config.contains_restart = true
            } else {
                draft.config.contains_indent = parse_num(&value)?
            }
        }
        OptionId::IncludeLeft => {
            draft.config.include_left = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::LabelLeft => {
            draft.config.label_left = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::MaxIndent => {
            draft.config.max_indent = parse_num(&cursor.required_long(&mut value)?)?
        }
        OptionId::Openmp => draft.config.openmp = parse_bool(&cursor.required_long(&mut value)?)?,
        OptionId::IndentAmpersand => {
            draft.config.indent_ampersand = value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true)
        }
        OptionId::IndentContinuation => {
            let value = cursor.required_long(&mut value)?;
            if value == "none" || value == "-" {
                draft.config.indent_continuation = false;
            } else if value == "default" || value == "d" {
                draft.config.indent_continuation = true;
            } else {
                draft.config.continuation_indent = parse_num(&value)?;
            }
        }
        OptionId::AlignParen => {
            draft.config.align_paren_value =
                value.as_deref().map(parse_num).transpose()?.unwrap_or(1);
            draft.config.align_paren = draft.config.align_paren_value != 0;
        }
        OptionId::WsRemred => {
            draft.config.ws_remred_value = parse_whitespace_reduction(value.as_deref())?;
            draft.config.ws_remred = draft.config.ws_remred_value != 0;
        }
        OptionId::AlignDeclarations => {
            draft.config.align_declarations = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::AlignComments => {
            draft.config.align_comments = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::LastIndent => {
            reject_value(name, &value)?;
            draft.config.last_indent = true;
        }
        OptionId::LastUsable => {
            reject_value(name, &value)?;
            draft.config.last_usable = true;
        }
        OptionId::All => {
            reject_value(name, &value)?;
            draft.all = true;
        }
        OptionId::AllFiles => {
            reject_value(name, &value)?;
            draft.all_files = true;
        }
        OptionId::NoSubmodules => {
            draft.no_submodules = value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true)
        }
        OptionId::ContextPath => {
            let path = cursor.required_long(&mut value)?;
            if path.is_empty() {
                return Err(FormatError::InvalidOption(
                    "--context-path requires a path".into(),
                ));
            }
            draft.context_paths.push(crate::cli::ContextPath {
                path: PathBuf::from(path),
                base: None,
            });
        }
        OptionId::ProjectContext => {
            let path = cursor.required_long(&mut value)?;
            if path.is_empty() {
                return Err(FormatError::InvalidOption(
                    "--project-context requires a path".into(),
                ));
            }
            if draft.project_context.is_some() {
                return Err(FormatError::InvalidOption(
                    "--project-context may be specified only once".into(),
                ));
            }
            draft.project_context = Some(PathBuf::from(path));
        }
        OptionId::Stdin => {
            reject_value(name, &value)?;
            draft.stdin = true;
        }
        OptionId::Stdout => {
            reject_value(name, &value)?;
            draft.stdout = true;
        }
        OptionId::Isolated => {
            reject_value(name, &value)?;
            draft.isolated = true;
        }
        OptionId::Check => {
            reject_value(name, &value)?;
            draft.check = true;
        }
        OptionId::Diff => {
            reject_value(name, &value)?;
            draft.diff = true;
        }
        OptionId::ShowFiles => {
            reject_value(name, &value)?;
            draft.show_files = true;
        }
        OptionId::QueryFormat => {
            reject_value(name, &value)?;
            draft.query_format = true;
        }
        OptionId::Exclude | OptionId::ExtendExclude => {
            let pattern = cursor.required_long(&mut value)?;
            if pattern.is_empty() {
                return Err(FormatError::InvalidOption(format!(
                    "--{name} requires a non-empty glob"
                )));
            }
            if option == OptionId::Exclude {
                draft.exclude.get_or_insert_with(Vec::new).push(pattern);
            } else {
                draft.extend_exclude.push(pattern);
            }
        }
        OptionId::IndentOnly => {
            reject_value(name, &value)?;
            draft.config.mode = FormatMode::IndentOnly;
            draft.config.style.normalize_whitespace = true;
        }
        OptionId::Full => {
            reject_value(name, &value)?;
            draft.config.mode = FormatMode::Full;
            draft.config.style.normalize_whitespace = true;
        }
        OptionId::NormalizeOnly => {
            reject_value(name, &value)?;
            draft.config.mode = FormatMode::NormalizeOnly;
            draft.config.style.normalize_whitespace = true;
        }
        OptionId::CanonicalizeOnly => {
            reject_value(name, &value)?;
            // Canonicalization-only is a normalize-only preset: it takes the
            // existing no-layout return path while line rules suppress
            // whitespace-only edits.
            draft.config.mode = FormatMode::NormalizeOnly;
            draft.config.style.normalize_whitespace = false;
        }
        OptionId::Wrap => {
            draft.config.wrap.enabled = value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true)
        }
        OptionId::NoWrap => {
            let disabled = value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true);
            draft.config.wrap.enabled = !disabled;
        }
        OptionId::Rewrap => {
            draft.config.rewrap = value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true);
        }
        OptionId::LineLength => {
            draft.config.wrap.line_length = parse_num(&cursor.required_long(&mut value)?)?
        }
        OptionId::UppercaseSingleL => {
            draft.config.uppercase_single_l = value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true)
        }
        OptionId::Define => {
            let value = cursor.required_long(&mut value)?;
            push_define(&mut draft.config, &value);
        }
        OptionId::KeywordCase => {
            let value = cursor.required_long(&mut value)?;
            draft.config.style.keyword_case = parse_style_choice(
                name,
                &value,
                &[
                    ("lower", KeywordCase::Lower),
                    ("upper", KeywordCase::Upper),
                    ("preserve", KeywordCase::Preserve),
                ],
            )?;
        }
        OptionId::RelationalSymbols => {
            draft.config.style.relational_symbols = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::ArrayBrackets => {
            draft.config.style.array_brackets = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::CompactMultiplicative => {
            draft.config.style.compact_multiplicative =
                parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::JoinGoto => {
            draft.config.style.join_goto = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::SplitCompoundKeywords => {
            draft.config.style.split_compound_keywords =
                parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::StripEmptyArgs => {
            draft.config.style.strip_empty_args = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::RemoveRedundantParens => {
            draft.config.style.remove_redundant_parens =
                parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::RemoveTerminalReturn => {
            draft.config.style.remove_terminal_return =
                parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::ProgramUnitSpacing => {
            draft.config.style.program_unit_spacing =
                parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::MaxBlankLines => {
            let value = cursor.required_long(&mut value)?;
            draft.config.style.max_blank_lines = if value == "preserve" {
                None
            } else {
                Some(parse_num(&value)?)
            };
        }
        OptionId::DelimiterSpacing => {
            draft.config.style.delimiter_spacing = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::CommentSpacing => {
            draft.config.style.comment_spacing = parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::ContinuationMarkers => {
            draft.config.style.continuation_markers =
                parse_bool(&cursor.required_long(&mut value)?)?
        }
        OptionId::IndentChangeteam => {
            draft.config.construct_indents.changeteam =
                parse_num(&cursor.required_long(&mut value)?)?
        }
        OptionId::RefactorEnd => {
            let (enabled, uppercase) = match value.as_deref() {
                None => (true, false),
                Some("upcase") => (true, true),
                Some(value) => (parse_bool(value)?, false),
            };
            draft.config.refactor_end = enabled;
            draft.config.uppercase_end = uppercase;
        }
        OptionId::InputFormat => {
            match cursor
                .required_long(&mut value)?
                .to_ascii_lowercase()
                .as_str()
            {
                "free" => draft.force_free_input = true,
                "auto" => draft.force_free_input = false,
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
            }
        }
        OptionId::OutputFormat => {
            match cursor
                .required_long(&mut value)?
                .to_ascii_lowercase()
                .as_str()
            {
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
            }
        }
        OptionId::Help | OptionId::Version => unreachable!("handled before long-option parsing"),
    }
    Ok(())
}

/// Parse the native reduction level while retaining findent's numeric levels.
/// TOML booleans are serialized through the same option parser, so accepting
/// boolean words here makes `reduce_whitespace = true/false` natural without
/// losing `--reduce-whitespace=N` or the legacy `--ws_remred=N` spellings.
fn parse_whitespace_reduction(value: Option<&str>) -> Result<usize, FormatError> {
    match value {
        None | Some("true") | Some("yes") => Ok(1),
        Some("false") | Some("no") => Ok(0),
        Some(value) => parse_num(value),
    }
}
