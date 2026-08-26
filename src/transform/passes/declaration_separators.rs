use std::ops::Range;

use crate::{
    error::FormatError,
    source::{
        syntax::{
            declaration_type_head_len, first_statement_index, is_declaration_statement,
            matching_close, top_level_separator,
        },
        tokens::{tokens, Token, TokenKind},
        LogicalStatement,
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

/// Insert the modern declaration separator in recognized declaration forms
/// whose legacy spelling permits it to be omitted.
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
            let Some(entity) = declaration_entity_index(&statement_tokens) else {
                continue;
            };
            let Some((line, entity_absolute)) =
                group.source_of_statement(statement, statement_tokens[entity].span.start)
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
                if let Some(previous) = entity
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

fn declaration_entity_index(tokens: &[Token<'_>]) -> Option<usize> {
    if !is_declaration_statement(tokens) || top_level_separator(tokens).is_some() {
        return None;
    }
    let first = first_statement_index(tokens);
    let head = tokens.get(first)?;

    if head.is_name(b"parameter") || is_select_type_guard(tokens, first) {
        return None;
    }
    if tokens.iter().skip(first + 1).any(|token| {
        token.depth == 0 && token.kind == TokenKind::Name && token.is_name(b"function")
    }) {
        return None;
    }

    if let Some(head_len) = declaration_type_head_len(tokens, first) {
        let mut start = first + head_len;
        if tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::LParen)
        {
            let close = matching_close(tokens, start)?;
            if tokens
                .get(close + 1)
                .is_some_and(is_designator_continuation)
            {
                return None;
            }
            start = close + 1;
        } else if tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::LBracket)
        {
            return None;
        }
        if tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::Comma && token.depth == 0)
        {
            return None;
        }
        if tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::Operator && token.text == b"*")
        {
            start = after_star_selector(tokens, start)?;
        }
        return tokens
            .get(start)
            .filter(|token| token.depth == 0 && token.kind == TokenKind::Name)
            .map(|_| start);
    }

    if head.is_name(b"procedure") {
        let mut start = first + 1;
        if tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::LParen)
        {
            start = matching_close(tokens, start)? + 1;
        }
        if tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::Comma && token.depth == 0)
        {
            return None;
        }
        return tokens
            .get(start)
            .filter(|token| token.depth == 0 && token.kind == TokenKind::Name)
            .map(|_| start);
    }

    let (entity, token) = tokens
        .iter()
        .enumerate()
        .skip(first + 1)
        .find(|(_, token)| token.depth == 0 && token.kind != TokenKind::Comment)?;
    if token.kind == TokenKind::Name
        || (head.is_name(b"save") && token.kind == TokenKind::Operator && token.text == b"/")
    {
        Some(entity)
    } else {
        None
    }
}

fn is_designator_continuation(token: &Token<'_>) -> bool {
    token.depth == 0
        && (matches!(token.kind, TokenKind::LParen | TokenKind::LBracket)
            || (token.kind == TokenKind::Operator && matches!(token.text, b"=" | b"=>" | b"%")))
}

fn after_star_selector(tokens: &[Token<'_>], star: usize) -> Option<usize> {
    let selector = tokens.get(star + 1)?;
    if selector.kind == TokenKind::LParen {
        let depth = selector.depth;
        return tokens
            .iter()
            .enumerate()
            .skip(star + 2)
            .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == depth)
            .map(|(index, _)| index + 1);
    }

    let mut start = star + 2;
    if selector.kind == TokenKind::Name
        && tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        let depth = tokens[start].depth;
        start = tokens
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == depth)
            .map(|(index, _)| index + 1)?;
    }
    Some(start)
}

fn is_select_type_guard(tokens: &[Token<'_>], first: usize) -> bool {
    let Some(head) = tokens.get(first) else {
        return false;
    };
    if !(head.is_name(b"type") || head.is_name(b"class")) {
        return false;
    }
    tokens.get(first + 1).is_some_and(|token| {
        token.depth == 0 && (token.is_name(b"is") || token.is_name(b"default"))
    })
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
    fn inserts_separator_in_old_style_declarations() {
        let source = b"real    x\ninteger*4 i\ndouble precision y\ntype box\ntype(foo) item\nprocedure(cb) handler\nsave /state/\n";
        let output = format_source(source, &config(FormatMode::NormalizeOnly))
            .unwrap()
            .bytes;
        assert_eq!(
            output,
            b"real :: x\ninteger*4 :: i\ndouble precision :: y\ntype :: box\ntype(foo) :: item\nprocedure(cb) :: handler\nsave :: /state/\n"
        );
    }

    #[test]
    fn preserves_existing_separators_and_non_declaration_lookalikes() {
        let source = b"real :: x\nreal elemental function f(x)\ntype is(real)\nclass default\nparameter (n=3)\n";
        let output = format_source(source, &config(FormatMode::CanonicalizeOnly))
            .unwrap()
            .bytes;
        assert_eq!(output, source);
    }

    #[test]
    fn preserves_keyword_spelled_designators_and_cray_pointer() {
        let source = b"pointer(i) = x\nsave(i) = x\nreal(i) = x\npointer (p, x)\n";
        let output = format_source(source, &config(FormatMode::CanonicalizeOnly))
            .unwrap()
            .bytes;
        assert_eq!(output, source);
    }

    #[test]
    fn modernizes_multiple_statements_and_continuations() {
        let source = b"real x; integer y\ncomplex &\n  z\n";
        let output = format_source(source, &config(FormatMode::NormalizeOnly))
            .unwrap()
            .bytes;
        assert_eq!(output, b"real :: x; integer :: y\ncomplex &\n  :: z\n");
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
