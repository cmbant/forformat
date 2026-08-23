use super::*;
use crate::source::syntax::is_directive_comment;

/// Rule 5: comment marker spacing and commented-out assignments.
///
/// Port target: `normalize_comment_spacing` plus `format_comment_operators`,
/// which is the one transform allowed to touch comment text (I3).
pub fn normalize_comment_spacing(line: &[u8], cx: &PassContext) -> Vec<u8> {
    normalize_comment_spacing_with_state(line, cx, LexState::default(), false, 0)
}

/// The width of the code on a line, ignoring indentation and any comment.
pub(crate) fn code_span_len(line: &[u8]) -> usize {
    let end = crate::source::regions::comment_start(line).unwrap_or(line.len());
    line[..end].trim_ascii().len()
}

pub(crate) fn normalize_comment_spacing_with_state(
    line: &[u8],
    cx: &PassContext,
    incoming: LexState,
    preserve_after: bool,
    code_growth: isize,
) -> Vec<u8> {
    if !cx.config.style.comment_spacing {
        return line.to_vec();
    }
    let mut state = incoming;
    let mut comment_start = None;
    state.scan(line, |region| {
        if comment_start.is_none() && region.kind == RegionKind::Comment {
            comment_start = Some(region.range.start);
        }
    });
    let Some(start) = comment_start else {
        return line.to_vec();
    };
    let original_comment = &line[start..];
    if original_comment.starts_with(b"!!")
        || is_directive_comment(original_comment)
        || original_comment[1..].iter().all(u8::is_ascii_whitespace)
    {
        return line.to_vec();
    }

    let mut comment = original_comment.to_vec();
    if is_commented_assignment(&comment) {
        comment = format_comment_operators(&comment);
    }
    let before = &line[..start];
    let leading = before.iter().position(|byte| !matches!(byte, b' ' | b'\t'));
    let leading_end = leading.unwrap_or(before.len());
    let mut code = before[leading_end..].to_vec();
    while code.last().is_some_and(u8::is_ascii_whitespace) {
        code.pop();
    }
    let mut out = Vec::with_capacity(line.len() + 2);
    out.extend_from_slice(&before[..leading_end]);
    if !code.is_empty() {
        out.extend_from_slice(&code);
        let gap = before.len() - leading_end - code.len();
        let corrected = (gap as isize - code_growth).max(1) as usize;
        out.resize(out.len() + corrected.min(gap.max(1)), b' ');
    }
    out.push(b'!');
    if preserve_after
        && comment
            .get(1)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        out.extend_from_slice(&comment[1..]);
        return out;
    }
    out.push(b' ');
    let mut after = &comment[1..];
    while after
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        after = &after[1..];
    }
    out.extend_from_slice(after);
    out
}

pub(crate) fn preserve_full_comment_spacing(
    document: &Document,
    index: usize,
    cx: &PassContext,
) -> bool {
    if cx
        .analysis
        .buffer
        .lines
        .get(index)
        .is_none_or(|line| line.kind != PhysicalLineKind::Comment)
    {
        return false;
    }
    let is_full_comment = |line: &[u8]| {
        let start = line
            .iter()
            .position(|byte| !matches!(byte, b' ' | b'\t'))
            .unwrap_or(line.len());
        let comment = &line[start..];
        comment.first() == Some(&b'!')
            && !comment.starts_with(b"!!")
            && !is_directive_comment(comment)
    };
    let current = document
        .lines
        .get(index)
        .is_some_and(|line| is_full_comment(line));
    if !current {
        return false;
    }
    let previous = index
        .checked_sub(1)
        .and_then(|line| document.lines.get(line))
        .is_some_and(|line| is_full_comment(line));
    let next = document
        .lines
        .get(index + 1)
        .is_some_and(|line| is_full_comment(line));
    previous || next
}

fn is_commented_assignment(comment: &[u8]) -> bool {
    let mut index = 1;
    while comment
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        index += 1;
    }
    let Some(end) = identifier_end(comment, index) else {
        return false;
    };
    if end == index {
        return false;
    }
    index = end;
    loop {
        while comment
            .get(index)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            index += 1;
        }
        if comment.get(index) == Some(&b'%') {
            index += 1;
            let Some(end) = identifier_end(comment, index) else {
                return false;
            };
            if end == index {
                return false;
            }
            index = end;
        } else if comment.get(index) == Some(&b'(') {
            while index < comment.len() && comment[index] != b')' && comment[index] != b'!' {
                index += 1;
            }
            if comment.get(index) != Some(&b')') {
                return false;
            }
            index += 1;
        } else {
            break;
        }
    }
    while comment
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        index += 1;
    }
    comment.get(index) == Some(&b'=')
        && (index == 0 || !matches!(comment[index - 1], b'<' | b'>' | b'=' | b'/'))
        && comment.get(index + 1) != Some(&b'=')
        && comment.get(index + 1) != Some(&b'>')
}

fn identifier_end(bytes: &[u8], start: usize) -> Option<usize> {
    if !bytes.get(start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    Some(end)
}

fn format_comment_operators(comment: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(comment.len() + 8);
    let mut quote = 0u8;
    let mut index = 0;
    while index < comment.len() {
        let byte = comment[index];
        if quote != 0 {
            output.push(byte);
            if byte == quote {
                if comment.get(index + 1) == Some(&quote) {
                    output.push(quote);
                    index += 2;
                    continue;
                }
                quote = 0;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = byte;
            output.push(byte);
            index += 1;
            continue;
        }
        if let Some((length, replacement)) = legacy_operator_at(comment, index) {
            append_comment_operator(&mut output, replacement, true);
            index = skip_horizontal(comment, index + length);
            continue;
        }
        if let Some(length) = spaced_operator_len(comment, index, output.last().copied()) {
            let named = comment[index] == b'='
                && comment.get(index + 1) != Some(&b'=')
                && is_named_parameter_at(comment, index);
            append_comment_operator(&mut output, &comment[index..index + length], !named);
            index = skip_horizontal(comment, index + length);
            continue;
        }
        if let Some(length) = arithmetic_operator_len(comment, index) {
            let operator = &comment[index..index + length];
            if operator == b"+" && super::is_binary_arithmetic_operator(comment, index, operator) {
                append_comment_operator(
                    &mut output,
                    operator,
                    !vocab::contains(vocab::COMPACT_ARITHMETIC_OPERATORS, operator),
                );
            } else {
                output.extend_from_slice(operator);
                index += length;
                continue;
            }
            index = skip_horizontal(comment, index + length);
            continue;
        }
        output.push(byte);
        index += 1;
    }
    output
}

fn append_comment_operator(output: &mut Vec<u8>, operator: &[u8], spaced: bool) {
    if spaced {
        while output.last().is_some_and(u8::is_ascii_whitespace) {
            output.pop();
        }
        if !output.is_empty() {
            output.push(b' ');
        }
        output.extend_from_slice(operator);
        output.push(b' ');
    } else {
        while output.last().is_some_and(u8::is_ascii_whitespace) {
            output.pop();
        }
        output.extend_from_slice(operator);
    }
}

fn skip_horizontal(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    index
}

fn is_named_parameter_at(line: &[u8], index: usize) -> bool {
    let mut end = index;
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (line[start - 1].is_ascii_alphanumeric() || line[start - 1] == b'_') {
        start -= 1;
    }
    if start == end || !line[start].is_ascii_alphabetic() {
        return false;
    }
    let mut prefix = start;
    while prefix > 0 && line[prefix - 1].is_ascii_whitespace() {
        prefix -= 1;
    }
    if prefix == 0 || !matches!(line[prefix - 1], b'(' | b',') {
        return false;
    }
    let mut depth = 0isize;
    let mut quote = 0u8;
    for &byte in &line[..index] {
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = byte;
        } else if matches!(byte, b'(' | b'[') {
            depth += 1;
        } else if matches!(byte, b')' | b']') {
            depth -= 1;
        }
    }
    depth > 0
}

fn legacy_operator_at(line: &[u8], index: usize) -> Option<(usize, &'static [u8])> {
    for (source, replacement) in [
        (b".eq.".as_slice(), b"==".as_slice()),
        (b".ne.", b"/="),
        (b".lt.", b"<"),
        (b".le.", b"<="),
        (b".gt.", b">"),
        (b".ge.", b">="),
    ] {
        if line[index..].len() >= source.len()
            && line[index..index + source.len()].eq_ignore_ascii_case(source)
        {
            return Some((source.len(), replacement));
        }
    }
    None
}

fn spaced_operator_len(line: &[u8], index: usize, previous: Option<u8>) -> Option<usize> {
    for operator in [b".and.".as_slice(), b".or.", b".not.", b".eqv.", b".neqv."] {
        if line[index..].len() >= operator.len()
            && line[index..index + operator.len()].eq_ignore_ascii_case(operator)
        {
            return Some(operator.len());
        }
    }
    for operator in [b"=>".as_slice(), b"==", b"/=", b"<=", b">="] {
        if line[index..].starts_with(operator) {
            return Some(operator.len());
        }
    }
    let byte = *line.get(index)?;
    let next = line.get(index + 1).copied();
    let valid = match byte {
        b'<' => !matches!(previous, Some(b'=' | b'<' | b'>')) && !matches!(next, Some(b'<' | b'>')),
        b'>' => {
            !matches!(previous, Some(b'=' | b'<' | b'>' | b'-'))
                && !matches!(next, Some(b'<' | b'>'))
        }
        b'=' => {
            !matches!(previous, Some(b'<' | b'>' | b'=' | b'/'))
                && !matches!(next, Some(b'=' | b'>'))
        }
        _ => false,
    };
    valid.then_some(1)
}

fn arithmetic_operator_len(line: &[u8], index: usize) -> Option<usize> {
    for operator in [b"**".as_slice(), b"//", b"+", b"-", b"*", b"/"] {
        if line[index..].starts_with(operator) {
            return Some(operator.len());
        }
    }
    None
}
