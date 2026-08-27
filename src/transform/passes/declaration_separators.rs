use std::ops::Range;

use crate::{
    error::FormatError,
    source::{
        declaration_separator::{declaration_separator, DeclarationSeparator},
        tokens::tokens,
        LogicalStatement, Token,
    },
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
    },
};

struct Edit {
    line: usize,
    range: Range<usize>,
    replacement: &'static [u8],
}

/// Insert the modern declaration separator at syntax-recognized omission sites.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    if !cx.config.style.modernize_declarations {
        return Ok(Changed::No);
    }

    let mut edits = Vec::new();
    for group in &cx.analysis.groups {
        for statement in &group.statements {
            if statement.is_fix {
                continue;
            }
            let statement_tokens = tokens(&statement.text);
            let DeclarationSeparator::Missing { insert_before } =
                declaration_separator(&statement_tokens)
            else {
                continue;
            };
            let Some(entity) = statement_tokens.get(insert_before) else {
                continue;
            };
            let Some((line, entity_absolute)) =
                group.source_of_statement(statement, entity.span.start)
            else {
                continue;
            };
            let Some(physical) = cx.analysis.buffer.lines.get(line) else {
                continue;
            };
            let entity_column = (entity_absolute - physical.span.start) as usize;
            let Some(text) = document.lines.get(line) else {
                continue;
            };
            if entity_column > text.len() {
                continue;
            }

            let mut range = entity_column..entity_column;
            let mut replacement: &'static [u8] = b":: ";
            if cx.config.mode.normalizes_whitespace() {
                if let Some(previous) = insert_before
                    .checked_sub(1)
                    .and_then(|index| statement_tokens.get(index))
                {
                    if let Some(previous_end) =
                        previous_source_end(group, statement, previous, line)
                    {
                        let previous_column = (previous_end - physical.span.start) as usize;
                        if previous_column <= entity_column
                            && text[previous_column..entity_column]
                                .iter()
                                .all(|byte| matches!(byte, b' ' | b'\t'))
                        {
                            range = previous_column..entity_column;
                            replacement = b" :: ";
                        }
                    }
                }
            }
            edits.push(Edit {
                line,
                range,
                replacement,
            });
        }
    }

    if edits.is_empty() {
        return Ok(Changed::No);
    }

    edits.sort_unstable_by(|left, right| {
        right
            .line
            .cmp(&left.line)
            .then_with(|| right.range.start.cmp(&left.range.start))
    });
    for edit in edits {
        let Some(line) = document.lines.get_mut(edit.line) else {
            continue;
        };
        line.splice(edit.range, edit.replacement.iter().copied());
    }
    Ok(Changed::Text)
}

fn previous_source_end(
    group: &crate::source::LogicalGroup,
    statement: &LogicalStatement,
    token: &Token<'_>,
    entity_line: usize,
) -> Option<u32> {
    let offset = token.span.end.checked_sub(1)?;
    let (line, absolute) = group.source_of_statement(statement, offset)?;
    (line == entity_line).then_some(absolute + 1)
}

#[cfg(test)]
mod tests {
    use crate::{config::FormatConfig, format_source, FormatMode};

    fn config(mode: FormatMode) -> FormatConfig {
        let mut config = FormatConfig {
            mode,
            apply_indent: false,
            ..FormatConfig::default()
        };
        config.style.modernize_declarations = true;
        config
    }

    #[test]
    fn modernizes_multiple_statements_and_continuations() {
        let source = b"real x; intent(in) y\ncomplex &\n  z\n";
        let output = format_source(source, &config(FormatMode::NormalizeOnly))
            .unwrap()
            .bytes;
        assert_eq!(output, b"real :: x; intent(in) :: y\ncomplex &\n  :: z\n");
    }

    #[test]
    fn canonicalization_preserves_authored_gap_before_separator() {
        let source = b"real    x\n";
        let output = format_source(source, &config(FormatMode::CanonicalizeOnly))
            .unwrap()
            .bytes;
        assert_eq!(output, b"real    :: x\n");
    }

    #[test]
    fn option_is_off_by_default() {
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            apply_indent: false,
            ..FormatConfig::default()
        };
        let output = format_source(b"real x\n", &config).unwrap().bytes;
        assert_eq!(output, b"real x\n");
    }
}
