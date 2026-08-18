use crate::{
    analysis::DeclaredNameIndex,
    config::{KeywordCase, StyleConfig},
    source::{
        regions::{LexState, RegionKind},
        tokens::{tokenize, TokenKind},
        PhysicalLineKind,
    },
    transform::{document::Document, edit::EditBuffer, pipeline::PassContext, vocab},
};

#[path = "case.rs"]
pub(super) mod case;
#[path = "comment_spacing.rs"]
pub(super) mod comment_spacing;
#[path = "delimiter_spacing.rs"]
pub(super) mod delimiter_spacing;
#[path = "keyword_spacing.rs"]
pub(super) mod keyword_spacing;
#[path = "write_spacing.rs"]
pub(super) mod write_spacing;

fn horizontal_gap(line: &[u8], start: usize, end: usize) -> bool {
    start <= end
        && line[start..end]
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t'))
}

fn is_multiword_keyword_pair(first: &[u8], second: &[u8]) -> bool {
    vocab::MULTIWORD_KEYWORD_PAIRS.iter().any(|(left, right)| {
        first.eq_ignore_ascii_case(left.as_bytes()) && second.eq_ignore_ascii_case(right.as_bytes())
    })
}

fn first_statement_index(tokens: &[crate::source::Token<'_>]) -> usize {
    usize::from(
        tokens
            .first()
            .is_some_and(|token| token.kind == TokenKind::Number),
    )
}

fn first_statement_token<'a>(
    tokens: &'a [crate::source::Token<'a>],
) -> Option<&'a crate::source::Token<'a>> {
    tokens
        .get(first_statement_index(tokens))
        .filter(|token| token.kind == TokenKind::Name)
}

fn is_followed_by_lparen(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.kind == TokenKind::LParen)
}

fn previous_name_is(tokens: &[crate::source::Token<'_>], index: usize, name: &[u8]) -> bool {
    index > 0 && tokens[index - 1].is_name(name)
}

fn matching_close(tokens: &[crate::source::Token<'_>], open: usize) -> Option<usize> {
    let opening = tokens.get(open)?;
    let close_kind = match opening.kind {
        TokenKind::LParen => TokenKind::RParen,
        TokenKind::LBracket => TokenKind::RBracket,
        _ => return None,
    };
    tokens
        .iter()
        .enumerate()
        .skip(open + 1)
        .find(|(_, token)| token.kind == close_kind && token.depth == opening.depth)
        .map(|(index, _)| index)
}

fn if_condition_close(tokens: &[crate::source::Token<'_>]) -> Option<usize> {
    let mut index = first_statement_index(tokens);
    if tokens
        .get(index)
        .is_some_and(|token| token.is_name(b"else"))
    {
        index += 1;
    }
    if !tokens.get(index).is_some_and(|token| token.is_name(b"if")) {
        return None;
    }
    let open = index + 1;
    tokens
        .get(open)
        .filter(|token| token.kind == TokenKind::LParen)?;
    matching_close(tokens, open)
}

fn is_labelled_format_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    first_statement_index(tokens) == 1 && is_format_statement(tokens)
}

pub(super) fn is_format_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    let index = first_statement_index(tokens);
    tokens
        .get(index)
        .is_some_and(|token| token.is_name(b"format"))
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen)
}

fn common_block_edit(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
) -> Option<(usize, usize, Vec<u8>)> {
    let index = first_statement_index(tokens);
    if !tokens
        .get(index)
        .is_some_and(|token| token.is_name(b"common"))
    {
        return None;
    }
    let slash = tokens.get(index + 1)?;
    let name = tokens.get(index + 2)?;
    let close = tokens.get(index + 3)?;
    if slash.text != b"/"
        || name.kind != TokenKind::Name
        || close.text != b"/"
        || !horizontal_gap(line, slash.span.end, name.span.start)
        || !horizontal_gap(line, name.span.end, close.span.start)
    {
        return None;
    }
    let mut end = close.span.end;
    while end < line.len() && matches!(line[end], b' ' | b'\t') {
        end += 1;
    }
    let mut replacement = b"common /".to_vec();
    replacement.extend_from_slice(name.text);
    replacement.extend_from_slice(b"/");
    if end < line.len() && line[end] != b'!' {
        replacement.push(b' ');
    }
    Some((tokens[index].span.start, end, replacement))
}

fn top_level_separator(tokens: &[crate::source::Token<'_>]) -> Option<usize> {
    tokens.iter().position(|token| {
        token.kind == TokenKind::Operator && token.text == b"::" && token.depth == 0
    })
}

pub(super) fn is_declaration_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    let index = first_statement_index(tokens);
    let Some(first) = tokens.get(index) else {
        return false;
    };
    if first.kind != TokenKind::Name {
        return false;
    }
    if first.is_name(b"double") {
        return tokens
            .get(index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    }
    matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer"
            | b"real"
            | b"complex"
            | b"logical"
            | b"character"
            | b"type"
            | b"class"
            | b"procedure"
            | b"dimension"
            | b"allocatable"
            | b"pointer"
            | b"target"
            | b"optional"
            | b"parameter"
            | b"save"
            | b"value"
            | b"volatile"
            | b"asynchronous"
            | b"contiguous"
            | b"codimension"
    )
}

fn is_specifier_keyword_argument(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.text == b"=" && is_named_parameter_token(tokens, index + 1))
}

fn is_contextual_declaration_name(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    index: usize,
    continued_entity_list: bool,
) -> bool {
    if tokens.get(index).is_none_or(|token| token.depth != 0) {
        return false;
    }
    let entities_start = match tokens[..index].iter().rposition(|token| {
        token.kind == TokenKind::Operator && token.text == b"::" && token.depth == 0
    }) {
        Some(separator) => separator + 1,
        None if continued_entity_list => 0,
        None => return false,
    };
    let mut item_start = entities_start;
    for (position, token) in tokens.iter().enumerate().take(index).skip(entities_start) {
        if token.kind == TokenKind::Comma && token.depth == 0 {
            item_start = position + 1;
        }
    }
    for token in tokens.iter().take(index).skip(item_start) {
        if token.kind != TokenKind::Operator || token.text != b"=" {
            continue;
        }
        let previous = token.span.start.checked_sub(1).and_then(|at| line.get(at));
        let following = line.get(token.span.end);
        if following == Some(&b'>')
            || (previous != Some(&b'<')
                && previous != Some(&b'>')
                && previous != Some(&b'=')
                && previous != Some(&b'/')
                && following != Some(&b'=')
                && following != Some(&b'>'))
        {
            return false;
        }
    }
    true
}

fn is_old_style_declaration_entity(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    if !is_declaration_statement(tokens) || top_level_separator(tokens).is_some() {
        return false;
    }
    let first = first_statement_index(tokens);
    let Some(type_token) = tokens.get(first) else {
        return false;
    };
    let mut entity_start = first + 1;
    if type_token.is_name(b"double")
        && tokens
            .get(entity_start)
            .is_some_and(|token| token.is_name(b"precision"))
    {
        entity_start += 1;
    } else if matches!(
        type_token.text.to_ascii_lowercase().as_slice(),
        b"type" | b"class"
    ) && tokens
        .get(entity_start)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        let Some(close) = matching_close(tokens, entity_start) else {
            return false;
        };
        entity_start = close + 1;
    } else if tokens
        .get(entity_start)
        .is_some_and(|token| token.text == b"*")
    {
        entity_start += 1;
        if tokens
            .get(entity_start)
            .is_some_and(|token| token.kind == TokenKind::Number)
        {
            entity_start += 1;
        }
    } else if tokens
        .get(entity_start)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        let Some(close) = matching_close(tokens, entity_start) else {
            return false;
        };
        entity_start = close + 1;
    }
    if !tokens
        .get(entity_start)
        .is_some_and(|token| token.kind == TokenKind::Name)
    {
        return false;
    }
    let mut item_start = entity_start;
    for (position, token) in tokens.iter().enumerate().take(index).skip(entity_start) {
        if token.kind == TokenKind::Comma && token.depth == 0 {
            item_start = position + 1;
        }
        if token.kind == TokenKind::Operator && token.text == b"=" && token.depth == 0 {
            return false;
        }
    }
    index == item_start
}

pub(super) fn is_named_parameter_token(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].kind == TokenKind::Name
        && (tokens[index - 2].kind == TokenKind::LParen
            || (tokens[index - 2].kind == TokenKind::Comma && tokens[index - 2].depth > 0))
}

fn is_io_specifier_star(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    token: &crate::source::Token<'_>,
) -> bool {
    if token.text != b"*" {
        return false;
    }
    let Some(io) = tokens.iter().enumerate().find_map(|(io, candidate)| {
        if !(candidate.is_name(b"print")
            || candidate.is_name(b"read")
            || candidate.is_name(b"write"))
        {
            return None;
        }
        let first = first_statement_index(tokens);
        let is_head =
            io == first || if_condition_close(tokens).is_some_and(|close| io == close + 1);
        is_head.then_some(io)
    }) else {
        return false;
    };
    let open = io + 1;
    if tokens
        .get(open)
        .is_some_and(|next| next.kind == TokenKind::LParen)
    {
        let Some(close) = matching_close(tokens, open) else {
            return false;
        };
        return index > open
            && index < close
            && matches!(tokens[index - 1].kind, TokenKind::LParen | TokenKind::Comma);
    }
    index == io + 1
}

pub(super) fn is_continued_named_parameter(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    inside_paren: bool,
) -> bool {
    inside_paren
        && index > 0
        && tokens[index - 1].kind == TokenKind::Name
        && (index == 1
            || tokens[index - 2].kind == TokenKind::Comma
            || tokens[..index - 1]
                .iter()
                .all(|token| token.kind == TokenKind::Ampersand))
}

pub(super) fn inside_paren_at(
    open_groups: &[bool],
    tokens: &[crate::source::Token<'_>],
) -> Vec<bool> {
    let mut open = open_groups.to_vec();
    let mut result = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token.kind {
            TokenKind::LParen => {
                result.push(open.last().copied().unwrap_or(false));
                open.push(true);
            }
            TokenKind::LBracket => {
                result.push(open.last().copied().unwrap_or(false));
                open.push(false);
            }
            TokenKind::RParen | TokenKind::RBracket => {
                open.pop();
                result.push(open.last().copied().unwrap_or(false));
            }
            _ => result.push(open.last().copied().unwrap_or(false)),
        }
    }
    result
}

fn real_exponent_marker(number: &[u8]) -> Option<usize> {
    let mut index = 0;
    let mut digits = 0;
    while number.get(index).is_some_and(u8::is_ascii_digit) {
        digits += 1;
        index += 1;
    }
    if number.get(index) == Some(&b'.') {
        index += 1;
        while number.get(index).is_some_and(u8::is_ascii_digit) {
            digits += 1;
            index += 1;
        }
    }
    if digits == 0 || !matches!(number.get(index), Some(b'E' | b'e' | b'D' | b'd')) {
        return None;
    }
    let marker = index;
    index += 1;
    if matches!(number.get(index), Some(b'+' | b'-')) {
        index += 1;
    }
    if !number.get(index).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some(marker)
}

fn modern_operator(token: &[u8]) -> Option<&'static [u8]> {
    if token.len() != 4 || token[0] != b'.' || token[3] != b'.' {
        return None;
    }
    match token[1].to_ascii_lowercase() {
        b'e' if token[2].eq_ignore_ascii_case(&b'q') => Some(b"=="),
        b'n' if token[2].eq_ignore_ascii_case(&b'e') => Some(b"/="),
        b'l' if token[2].eq_ignore_ascii_case(&b't') => Some(b"<"),
        b'l' if token[2].eq_ignore_ascii_case(&b'e') => Some(b"<="),
        b'g' if token[2].eq_ignore_ascii_case(&b't') => Some(b">"),
        b'g' if token[2].eq_ignore_ascii_case(&b'e') => Some(b">="),
        _ => None,
    }
}

fn apply_case(bytes: &[u8], case: KeywordCase) -> Vec<u8> {
    match case {
        KeywordCase::Lower => bytes.to_ascii_lowercase(),
        KeywordCase::Upper => bytes.to_ascii_uppercase(),
        KeywordCase::Preserve => bytes.to_vec(),
    }
}

fn compound_spelling(source: &[u8], canonical: &str) -> Vec<u8> {
    let first_len = canonical
        .split_once(' ')
        .map_or(canonical.len(), |(first, _)| first.len());
    if source.len() < first_len {
        return canonical.as_bytes().to_vec();
    }
    let mut result = source[..first_len].to_vec();
    result.push(b' ');
    result.extend_from_slice(&source[first_len..]);
    result
}

fn dotted_case(token: &[u8], case: KeywordCase) -> Vec<u8> {
    let mut result = token.to_vec();
    if result.len() > 2 {
        let interior = apply_case(&result[1..result.len() - 1], case);
        result.splice(1..result.len() - 1, interior);
    }
    result
}

fn dotted_word_case(token: &[u8], case: KeywordCase) -> Option<Vec<u8>> {
    let word = token.strip_prefix(b".")?.strip_suffix(b".")?;
    if word.is_empty() {
        return None;
    }
    let canonical = word.to_ascii_lowercase();
    if !vocab::contains(vocab::INTRINSIC_NAMES, &canonical) {
        return None;
    }
    let mut out = Vec::with_capacity(token.len());
    out.push(b'.');
    out.extend_from_slice(&apply_case(word, case));
    out.push(b'.');
    Some(out)
}

fn is_spaced_dotted_operator(token: &[u8]) -> bool {
    [b".and.".as_slice(), b".or.", b".not.", b".eqv.", b".neqv."]
        .iter()
        .any(|operator| token.eq_ignore_ascii_case(operator))
}

fn is_spaced_operator_token(
    line: &[u8],
    _tokens: &[crate::source::Token<'_>],
    _index: usize,
    token: &crate::source::Token<'_>,
) -> bool {
    let start = token.span.start;
    let end = token.span.end;
    match token.text {
        b"=>" | b"==" | b"/=" | b"<=" | b">=" => true,
        b"<" => {
            (start == 0 || !matches!(line[start - 1], b'=' | b'<' | b'>'))
                && (end == line.len() || !matches!(line[end], b'<' | b'>'))
        }
        b">" => {
            (start == 0 || !matches!(line[start - 1], b'=' | b'<' | b'>' | b'-'))
                && (end == line.len() || !matches!(line[end], b'<' | b'>'))
        }
        b"=" => {
            (start == 0 || !matches!(line[start - 1], b'<' | b'>' | b'=' | b'/'))
                && (end == line.len() || !matches!(line[end], b'=' | b'>'))
        }
        _ => false,
    }
}

fn is_arithmetic_operator(operator: &[u8]) -> bool {
    matches!(operator, b"+" | b"-" | b"*" | b"/" | b"**" | b"//")
}

fn binary_operator_spaced(operator: &[u8], compact_multiplicative: bool) -> bool {
    !(compact_multiplicative && matches!(operator, b"*" | b"/" | b"**"))
}

fn is_declaration_type_star(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    operator: &[u8],
) -> bool {
    operator == b"*"
        && index > 0
        && index - 1 == first_statement_index(tokens)
        && tokens[index - 1].kind == TokenKind::Name
        && matches!(
            tokens[index - 1].text.to_ascii_lowercase().as_slice(),
            b"character" | b"integer" | b"real" | b"complex" | b"logical"
        )
        && tokens[index].depth == 0
}

fn is_binary_arithmetic_operator(line: &[u8], index: usize, operator: &[u8]) -> bool {
    let mut previous = index;
    while previous > 0 && line[previous - 1].is_ascii_whitespace() {
        previous -= 1;
    }
    let mut following = index + operator.len();
    while following < line.len() && line[following].is_ascii_whitespace() {
        following += 1;
    }
    if operator == b"//" {
        return following < line.len();
    }
    if previous == 0 || following >= line.len() {
        return false;
    }
    let previous_byte = line[previous - 1];
    if !matches!(
        previous_byte,
        b')' | b']' | b'_' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'
    ) {
        return false;
    }
    if previous_byte == b'.' && dotted_operator_before(line, previous) {
        return false;
    }
    if matches!(operator, b"+" | b"-") && exponent_before(line, index) {
        return false;
    }
    !matches!(line[following], b')' | b']' | b',')
}

fn dotted_operator_before(line: &[u8], end: usize) -> bool {
    let dot = end.saturating_sub(1);
    let mut start = dot;
    while start > 0 && line[start - 1].is_ascii_alphabetic() {
        start -= 1;
    }
    start < dot && start > 0 && line[start - 1] == b'.'
}

fn exponent_before(line: &[u8], index: usize) -> bool {
    index > 0
        && matches!(line[index - 1], b'e' | b'E' | b'd' | b'D')
        && index > 1
        && (line[index - 2].is_ascii_digit() || line[index - 2] == b'.')
}

#[derive(Default)]
struct OperatorSpacing {
    previous_end: Option<usize>,
    previous_trailing_space: bool,
    previous_compact_named: bool,
}

fn add_operator_edit(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    operator: &[u8],
    spaced: bool,
    spacing: &mut OperatorSpacing,
) {
    let floor = spacing.previous_end.unwrap_or(0);
    let mut left = token.span.start;
    while left > floor && line[left - 1].is_ascii_whitespace() {
        left -= 1;
    }
    let mut right = token.span.end;
    while right < line.len() && line[right].is_ascii_whitespace() {
        right += 1;
    }
    let abuts_previous = spacing.previous_end == Some(left);
    let suppress_leading_space =
        abuts_previous && (spacing.previous_trailing_space || spacing.previous_compact_named);
    let mut replacement = Vec::new();
    if left == 0 {
        replacement.extend_from_slice(&line[..token.span.start]);
    }
    if spaced && left > 0 && !suppress_leading_space {
        replacement.push(b' ');
    }
    replacement.extend_from_slice(operator);
    let trailing = spaced || is_trailing_continuation_marker(line, token.span.end);
    if trailing {
        replacement.push(b' ');
    }
    spacing.previous_end = Some(right);
    spacing.previous_trailing_space = trailing;
    spacing.previous_compact_named = false;
    edits.replace(left..right, &replacement);
}

fn remove_operator_trailing_whitespace(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    spacing: &mut OperatorSpacing,
) {
    let mut end = token.span.end;
    while end < line.len() && line[end].is_ascii_whitespace() {
        end += 1;
    }
    if end > token.span.end && !is_trailing_continuation_marker(line, token.span.end) {
        edits.replace(token.span.end..end, b"");
        spacing.previous_end = Some(end);
    } else {
        spacing.previous_end = Some(token.span.end);
    }
    spacing.previous_trailing_space = false;
    spacing.previous_compact_named = false;
}

fn is_trailing_continuation_marker(line: &[u8], start: usize) -> bool {
    let mut index = start;
    while index < line.len() && line[index].is_ascii_whitespace() {
        index += 1;
    }
    index < line.len()
        && line[index] == b'&'
        && line[index + 1..]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
}

pub fn is_protected(line: &[u8], offset: usize) -> bool {
    let mut protected = false;
    LexState::default().scan(line, |region| {
        if region.range.contains(&offset) && region.kind != RegionKind::Code {
            protected = true;
        }
    });
    protected
}
