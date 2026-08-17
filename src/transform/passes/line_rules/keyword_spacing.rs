use super::*;

/// Rule 2: keyword and layout spacing.
///
/// This stage owns the statement-level rewrites (array constructors, compound
/// and multiword keywords, `goto`) before the token-local spacing rules.
pub fn normalize_keyword_spacing(
    line: &[u8],
    declared_names: &DeclaredNameIndex,
    line_index: usize,
) -> Vec<u8> {
    normalize_keyword_spacing_with_state(
        line,
        declared_names,
        line_index,
        LexState::default(),
        false,
        &StyleConfig::default(),
    )
}

pub(crate) fn normalize_keyword_spacing_with_state(
    line: &[u8],
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    incoming: LexState,
    continued_format: bool,
    style: &StyleConfig,
) -> Vec<u8> {
    let tokens = tokenize(line, &mut incoming.clone());
    let mut edits = EditBuffer::new(line);

    if let Some((start, end, replacement)) = common_block_edit(line, &tokens) {
        edits.replace(start..end, &replacement);
    }
    if style.array_brackets && !is_format_statement(&tokens) && !continued_format {
        for pair in tokens.windows(2) {
            if pair[0].kind == TokenKind::LParen
                && pair[1].kind == TokenKind::Operator
                && pair[1].text == b"/"
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            {
                let mut end = pair[1].span.end;
                while end < line.len() && matches!(line[end], b' ' | b'\t') {
                    end += 1;
                }
                edits.replace(pair[0].span.start..end, b"[");
            }
        }
        for pair in tokens.windows(2) {
            if pair[0].kind == TokenKind::Operator
                && pair[0].text == b"/"
                && pair[1].kind == TokenKind::RParen
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            {
                let mut start = pair[0].span.start;
                while start > 0 && matches!(line[start - 1], b' ' | b'\t') {
                    start -= 1;
                }
                edits.replace(start..pair[1].span.end, b"]");
            }
        }
    }

    if style.join_goto {
        for pair in tokens.windows(2) {
            if pair[0].is_name(b"go")
                && pair[1].is_name(b"to")
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            {
                let mut replacement = apply_case(pair[0].text, style.keyword_case);
                replacement.extend_from_slice(&apply_case(pair[1].text, style.keyword_case));
                if if_condition_close(&tokens)
                    .is_some_and(|close| tokens[close].span.end == pair[0].span.start)
                {
                    replacement.insert(0, b' ');
                }
                edits.replace(pair[0].span.start..pair[1].span.end, &replacement);
            }
        }
    }

    for pair in tokens.windows(2) {
        if pair[0].kind == TokenKind::Name
            && pair[1].kind == TokenKind::Name
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            && is_multiword_keyword_pair(pair[0].text, pair[1].text)
        {
            let mut replacement = apply_case(pair[0].text, style.keyword_case);
            replacement.push(b' ');
            replacement.extend_from_slice(&apply_case(pair[1].text, style.keyword_case));
            edits.replace(pair[0].span.start..pair[1].span.end, &replacement);
        }
    }

    if style.split_compound_keywords {
        if let Some(first) = first_statement_token(&tokens) {
            if let Some(replacement) = vocab::lookup_pair(vocab::COMPOUND_KEYWORDS, first.text) {
                let next = tokens.get(first_statement_index(&tokens) + 1);
                let assignment = next.is_some_and(|token| token.text == b"=");
                if !assignment && !declared_names.suppresses_keyword(line_index, first.text, false)
                {
                    let mut replacement = compound_spelling(first.text, replacement);
                    if style.keyword_case != KeywordCase::Preserve {
                        replacement = apply_case(&replacement, style.keyword_case);
                    }
                    edits.replace(first.span.clone(), &replacement);
                    if let Some(next) = next {
                        if next.kind == TokenKind::Name
                            && horizontal_gap(line, first.span.end, next.span.start)
                        {
                            edits.replace(first.span.end..next.span.start, b" ");
                        }
                    }
                    if first.is_name(b"elseif") {
                        if let Some(paren) = tokens.get(first_statement_index(&tokens) + 1) {
                            if paren.kind == TokenKind::LParen {
                                edits.replace(first.span.end..paren.span.start, b" ");
                            }
                        }
                    }
                }
            }
        }
    }

    for (index, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Name {
            if token.is(b"end") && !declared_names.suppresses_keyword(line_index, token.text, false)
            {
                if let Some(next) = tokens.get(index + 1) {
                    if next.kind == TokenKind::Name
                        && horizontal_gap(line, token.span.end, next.span.start)
                    {
                        edits.replace(token.span.end..next.span.start, b" ");
                        if let Some(after) = tokens.get(index + 2) {
                            if after.kind == TokenKind::Name
                                && horizontal_gap(line, next.span.end, after.span.start)
                            {
                                edits.replace(next.span.end..after.span.start, b" ");
                            }
                        }
                    }
                }
            }
            if token.is(b"do") && !declared_names.suppresses_keyword(line_index, token.text, false)
            {
                if let Some(next) = tokens.get(index + 1) {
                    if next.kind == TokenKind::Name
                        && horizontal_gap(line, token.span.end, next.span.start)
                    {
                        edits.replace(token.span.end..next.span.start, b" ");
                        if next.is_name(b"while") {
                            if let Some(paren) = tokens.get(index + 2) {
                                if paren.kind == TokenKind::LParen
                                    && horizontal_gap(line, next.span.end, paren.span.start)
                                {
                                    edits.replace(next.span.end..paren.span.start, b" ");
                                }
                            }
                        }
                    }
                }
            }
            if token.is(b"only")
                && !declared_names.suppresses_keyword(line_index, token.text, false)
                && tokens.get(index + 1).is_some_and(|next| next.text == b":")
            {
                let colon = tokens[index + 1].span.start;
                let keyword = apply_case(token.text, style.keyword_case);
                if token.text != keyword || horizontal_gap(line, token.span.end, colon) {
                    edits.replace(token.span.start..colon, &keyword);
                }
            }
            if (token.is(b"module")
                || token.is(b"use")
                || token.is(b"call")
                || token.is(b"subroutine"))
                && !declared_names.suppresses_keyword(line_index, token.text, false)
            {
                if let Some(next) = tokens.get(index + 1) {
                    if next.kind == TokenKind::Name
                        && horizontal_gap(line, token.span.end, next.span.start)
                    {
                        edits.replace(token.span.end..next.span.start, b" ");
                    }
                }
            }
        }

        if token.kind == TokenKind::Name && is_followed_by_lparen(&tokens, index) {
            let next = &tokens[index + 1];
            if !horizontal_gap(line, token.span.end, next.span.start) {
                continue;
            }
            let declared = declared_names.suppresses_keyword(line_index, token.text, false);
            let selected_type = index > 0 && tokens[index - 1].is_name(b"select");
            let no_space = vocab::contains(vocab::PARENTHESIZED_STATEMENT_NAMES, token.text)
                || token.is(b"dimension")
                || token.is(b"associate")
                || token.is(b"result")
                || (token.is(b"type") && !selected_type)
                || (token.is(b"class") && !selected_type);
            let one_space = token.is(b"if") || token.is(b"select");
            if !declared && (no_space || one_space) {
                edits.replace(
                    token.span.end..next.span.start,
                    if no_space { b"" } else { b" " },
                );
            }
        }

        if token.is(b"select") {
            if let (Some(ty), Some(paren)) = (tokens.get(index + 1), tokens.get(index + 2)) {
                if ty.is_name(b"type") && paren.kind == TokenKind::LParen {
                    if horizontal_gap(line, token.span.end, ty.span.start) {
                        edits.replace(token.span.end..ty.span.start, b" ");
                    }
                    if horizontal_gap(line, ty.span.end, paren.span.start) {
                        edits.replace(ty.span.end..paren.span.start, b" ");
                    }
                }
            }
            if let (Some(ty), Some(is), Some(paren)) = (
                tokens.get(index + 1),
                tokens.get(index + 2),
                tokens.get(index + 3),
            ) {
                if ty.is_name(b"type") && is.is_name(b"is") && paren.kind == TokenKind::LParen {
                    if horizontal_gap(line, token.span.end, ty.span.start) {
                        edits.replace(token.span.end..ty.span.start, b" ");
                    }
                    if horizontal_gap(line, ty.span.end, is.span.start) {
                        edits.replace(ty.span.end..is.span.start, b" ");
                    }
                    if horizontal_gap(line, is.span.end, paren.span.start) {
                        edits.replace(is.span.end..paren.span.start, b" ");
                    }
                }
            }
        }

        if token.is(b"change") || token.is(b"form") || token.is(b"select") || token.is(b"sync") {
            if let (Some(rank_or_team), Some(paren)) =
                (tokens.get(index + 1), tokens.get(index + 2))
            {
                if (rank_or_team.is_name(b"rank") || rank_or_team.is_name(b"team"))
                    && paren.kind == TokenKind::LParen
                    && horizontal_gap(line, rank_or_team.span.end, paren.span.start)
                {
                    edits.replace(rank_or_team.span.end..paren.span.start, b" ");
                }
            }
        }
    }

    for pair in tokens.windows(2) {
        if (pair[0].kind == TokenKind::LParen || pair[0].kind == TokenKind::LBracket)
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            && !is_trailing_continuation_marker(line, pair[1].span.start)
        {
            edits.replace(pair[0].span.end..pair[1].span.start, b"");
        }
        if (pair[1].kind == TokenKind::RParen || pair[1].kind == TokenKind::RBracket)
            && !matches!(pair[0].kind, TokenKind::String | TokenKind::Hollerith)
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
        {
            edits.replace(pair[0].span.end..pair[1].span.start, b"");
        }
        if pair[0].kind == TokenKind::RParen
            && pair[1].is_name(b"then")
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
        {
            edits.replace(pair[0].span.end..pair[1].span.start, b" ");
        }
    }

    if let Some(close) = if_condition_close(&tokens) {
        if let Some(next) = tokens.get(close + 1) {
            if next.kind != TokenKind::Comment
                && next.text != b"&"
                && !next.is_name(b"then")
                && line[next.span.start..]
                    .iter()
                    .any(|byte| !byte.is_ascii_whitespace())
            {
                edits.replace(tokens[close].span.end..next.span.start, b" ");
            }
        }
    }

    if style.strip_empty_args {
        for (index, subroutine) in tokens.iter().enumerate() {
            if !subroutine.is_name(b"subroutine") || index > 0 && tokens[index - 1].is_name(b"end")
            {
                continue;
            }
            if let (Some(name), Some(open), Some(close)) = (
                tokens.get(index + 1),
                tokens.get(index + 2),
                tokens.get(index + 3),
            ) {
                if name.kind == TokenKind::Name
                    && open.kind == TokenKind::LParen
                    && close.kind == TokenKind::RParen
                {
                    edits.replace(open.span.start..close.span.end, b"");
                }
            }
        }
    }

    let mut output = edits.finish();
    let start = output
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(output.len());
    if output
        .get(start..)
        .is_some_and(|tail| tail.starts_with(b"else if("))
    {
        output.insert(start + b"else if".len(), b' ');
    }
    output
}
