use super::*;

/// Rule 3: `WRITE(...)item` spacing.
///
/// Port target: `normalize_write_output_spacing`.
pub fn normalize_write_output_spacing(line: &[u8], cx: &PassContext) -> Vec<u8> {
    normalize_write_output_spacing_with_state(line, cx, LexState::default())
}

pub(crate) fn normalize_write_output_spacing_with_state(
    line: &[u8],
    cx: &PassContext,
    incoming: LexState,
) -> Vec<u8> {
    let _ = cx;
    let tokens = tokenize(line, &mut incoming.clone());
    let mut edits = EditBuffer::new(line);
    for (index, token) in tokens.iter().enumerate() {
        if !token.is_name(b"write") || !is_followed_by_lparen(&tokens, index) {
            continue;
        }
        let open = index + 1;
        let Some(close) = matching_close(&tokens, open) else {
            continue;
        };
        let end = tokens[close].span.end;
        if end < line.len()
            && !line[end].is_ascii_whitespace()
            && !matches!(line[end], b'&' | b'!' | b';' | b'\n')
        {
            edits.insert(end, b" ");
        }
    }
    edits.finish()
}
