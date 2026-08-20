use super::*;

/// Rule 4: delimiter and comma spacing.
///
/// Port target: `normalize_delimiter_spacing` — one space after a comma, none
/// before it, none inside brackets, and the compact behaviour of `*`, `/`,
/// `**`, `//` ([`vocab::COMPACT_ARITHMETIC_OPERATORS`]).
pub fn normalize_delimiter_spacing(line: &[u8], cx: &PassContext) -> Vec<u8> {
    normalize_delimiter_spacing_with_state(line, cx, LexState::default(), false)
}

pub(crate) fn normalize_delimiter_spacing_with_state(
    line: &[u8],
    cx: &PassContext,
    incoming: LexState,
    continued_statement: bool,
) -> Vec<u8> {
    if !cx.config.style.delimiter_spacing {
        return line.to_vec();
    }
    let mut text = line.to_vec();
    let tokens = tokenize(&text, &mut incoming.clone());
    if !continued_statement && is_declaration_statement(&tokens) {
        if let Some(separator) = top_level_separator(&tokens) {
            text =
                reorder_optional_attribute(&text, tokens[separator].span.start, incoming.clone());
        } else {
            text = normalize_old_style_declaration(&text, incoming.clone());
        }
    }

    let compact_state = incoming.clone();
    let mut state = incoming;
    let regions = state.regions(&text);
    let mut result = Vec::with_capacity(text.len());
    for (index, region) in regions.iter().enumerate() {
        if region.kind == RegionKind::Code {
            let following_content = regions
                .get(index + 1)
                .is_some_and(|next| next.kind != RegionKind::Comment);
            normalize_delimiters_in_code(
                &text[region.range.clone()],
                &mut result,
                following_content,
            );
        } else {
            result.extend_from_slice(&text[region.range.clone()]);
        }
    }
    compact_multiple_subscript_spacing(&result, compact_state)
}

fn reorder_optional_attribute(line: &[u8], separator: usize, incoming: LexState) -> Vec<u8> {
    let prefix = &line[..separator];
    let tokens = tokenize(prefix, &mut incoming.clone());
    let commas: Vec<usize> = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Comma && token.depth == 0)
        .map(|token| token.span.start)
        .collect();
    let mut ranges = Vec::with_capacity(commas.len() + 1);
    let mut start = 0;
    for comma in commas {
        ranges.push(start..comma);
        start = comma + 1;
    }
    ranges.push(start..prefix.len());
    let optional: Vec<&[u8]> = ranges
        .iter()
        .filter_map(|range| {
            let attribute = &prefix[range.clone()];
            trim_ascii(attribute)
                .eq_ignore_ascii_case(b"optional")
                .then_some(attribute)
        })
        .collect();
    if optional.is_empty() {
        return line.to_vec();
    }
    let mut attributes: Vec<&[u8]> = ranges
        .iter()
        .map(|range| &prefix[range.clone()])
        .filter(|attribute| !trim_ascii(attribute).eq_ignore_ascii_case(b"optional"))
        .collect();
    attributes.extend(optional);
    let mut replacement = Vec::with_capacity(prefix.len());
    for (index, attribute) in attributes.iter().enumerate() {
        if index > 0 {
            replacement.push(b',');
        }
        replacement.extend_from_slice(attribute);
    }
    let mut edits = EditBuffer::new(line);
    edits.replace(0..separator, &replacement);
    edits.finish()
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn normalize_old_style_declaration(line: &[u8], incoming: LexState) -> Vec<u8> {
    let mut state = incoming;
    let mut result = Vec::with_capacity(line.len());
    state.scan(line, |region| {
        if region.kind == RegionKind::Code {
            let code = &line[region.range];
            let mut one = Vec::new();
            normalize_old_style_code(code, &mut one);
            result.extend_from_slice(&one);
        } else {
            result.extend_from_slice(&line[region.range]);
        }
    });
    result
}

fn normalize_old_style_code(code: &[u8], out: &mut Vec<u8>) {
    let mut source = code.to_vec();
    let tokens = crate::source::tokens::tokens(code);
    let first = first_statement_index(&tokens);
    if let (Some(type_token), Some(next)) = (tokens.get(first), tokens.get(first + 1)) {
        let mut spec_end = type_token.span.end;
        if type_token.is_name(b"double") && next.is_name(b"precision") {
            spec_end = next.span.end;
        } else if matches!(
            type_token.text.to_ascii_lowercase().as_slice(),
            b"integer" | b"real" | b"complex" | b"logical" | b"character" | b"type" | b"class"
        ) && next.kind == TokenKind::LParen
        {
            if let Some(close) = matching_close(&tokens, first + 1) {
                spec_end = tokens[close].span.end;
            }
        }
        if let Some(entity) = tokens.iter().find(|token| {
            token.kind == TokenKind::Name && token.span.start >= spec_end && token.depth == 0
        }) {
            if entity.span.start == spec_end {
                let mut edits = EditBuffer::new(code);
                edits.insert(spec_end, b" ");
                source = edits.finish();
            }
        }
    }
    let leading = code
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(code.len());
    out.extend_from_slice(&source[..leading.min(source.len())]);
    let mut pending = false;
    for byte in &source[leading.min(source.len())..] {
        if matches!(byte, b' ' | b'\t') {
            pending = true;
        } else {
            if pending {
                out.push(b' ');
                pending = false;
            }
            out.push(*byte);
        }
    }
}

fn normalize_delimiters_in_code(code: &[u8], out: &mut Vec<u8>, following_content: bool) {
    let mut index = 0;
    while index < code.len() {
        if code[index] == b',' {
            let mut keep = out.len();
            while keep > 0 && matches!(out[keep - 1], b' ' | b'\t') {
                keep -= 1;
            }
            if out[..keep].iter().any(|byte| !matches!(byte, b' ' | b'\t')) {
                out.truncate(keep);
            }
            out.push(b',');
            index += 1;
            while index < code.len() && matches!(code[index], b' ' | b'\t') {
                index += 1;
            }
            if (index < code.len() && code[index] != b'\n')
                || (index == code.len() && following_content)
            {
                out.push(b' ');
            }
            continue;
        }
        if code[index..].starts_with(b"::") {
            out.extend_from_slice(b"::");
            index += 2;
            if index < code.len() && !code[index].is_ascii_whitespace() {
                out.push(b' ');
            }
            continue;
        }
        out.push(code[index]);
        index += 1;
    }
}

/// Fortran 2023 multiple subscripts use `@` as a prefix and their optional
/// triplet colons are part of that same compact designator: `@V`, `@[1, 3]`,
/// `@lo:hi:step`, and `@::step`. Keep only those punctuation seams compact;
/// ordinary section-subscript colons in sibling items retain their authored
/// spacing. Tokenization keeps strings and comments out of this rewrite.
fn compact_multiple_subscript_spacing(line: &[u8], mut state: LexState) -> Vec<u8> {
    let tokens = tokenize(line, &mut state);
    let mut compact_gaps: Vec<std::ops::Range<usize>> = Vec::new();

    for (index, at) in tokens.iter().enumerate() {
        if at.kind != TokenKind::Operator || at.text != b"@" {
            continue;
        }

        if let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) {
            if matches!(previous.kind, TokenKind::LParen | TokenKind::LBracket)
                && horizontal_gap(line, previous.span.end, at.span.start)
                && previous.span.end < at.span.start
            {
                compact_gaps.push(previous.span.end..at.span.start);
            }
        }

        if let Some(next) = tokens.get(index + 1) {
            if !matches!(next.kind, TokenKind::Ampersand | TokenKind::Comment)
                && horizontal_gap(line, at.span.end, next.span.start)
                && at.span.end < next.span.start
            {
                compact_gaps.push(at.span.end..next.span.start);
            }
        }

        for (offset, punctuation) in tokens.iter().enumerate().skip(index + 1) {
            if punctuation.kind == TokenKind::Comment || punctuation.depth < at.depth {
                break;
            }
            if punctuation.depth == at.depth && punctuation.kind == TokenKind::Comma {
                break;
            }
            if punctuation.depth != at.depth
                || punctuation.kind != TokenKind::Operator
                || !matches!(punctuation.text, b":" | b"::")
            {
                continue;
            }

            if let Some(previous) = offset.checked_sub(1).and_then(|i| tokens.get(i)) {
                if horizontal_gap(line, previous.span.end, punctuation.span.start)
                    && previous.span.end < punctuation.span.start
                {
                    compact_gaps.push(previous.span.end..punctuation.span.start);
                }
            }
            if let Some(next) = tokens.get(offset + 1) {
                if !matches!(next.kind, TokenKind::Ampersand | TokenKind::Comment)
                    && horizontal_gap(line, punctuation.span.end, next.span.start)
                    && punctuation.span.end < next.span.start
                {
                    compact_gaps.push(punctuation.span.end..next.span.start);
                }
            }
        }
    }

    if compact_gaps.is_empty() {
        return line.to_vec();
    }
    compact_gaps.sort_by_key(|range| (range.start, range.end));
    compact_gaps.dedup_by(|left, right| left.start == right.start && left.end == right.end);

    let mut edits = EditBuffer::new(line);
    for gap in compact_gaps {
        edits.replace(gap, b"");
    }
    edits.finish()
}
