pub fn reduce(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    reduce_into(s, |byte| out.push(byte));
    out
}

/// Apply redundant-whitespace reduction without allocating an output buffer.
/// Quotes, doubled quotes, and comments are copied byte-for-byte.
pub fn reduce_into<F: FnMut(u8)>(s: &[u8], mut put: F) {
    let mut quote = 0u8;
    reduce_line_into(s, &mut quote, &mut put);
}

/// Reduce one physical line while carrying a quote state across free-form
/// continuation lines. Fortran strings may span physical lines, so a
/// reducer that starts each line in the unquoted state would rewrite spaces
/// inside the continued literal.
pub fn reduce_line_into<F: FnMut(u8)>(s: &[u8], quote: &mut u8, put: &mut F) {
    reduce_line_into_protected(s, quote, false, false, put);
}

/// `reduce_line_into`, with the option to leave two *kinds* of gap exactly as
/// authored rather than collapsed to one space, wherever either occurs on the
/// line and however wide the authored padding is: the whitespace immediately
/// before a declaration `::` (when `protect_declaration_gap` is set) and
/// immediately before a trailing `!` comment (when `protect_comment_gap` is
/// set).
///
/// Both kinds are owned by a post-layout alignment pass
/// (`layout_post::declaration_separator_alignment` /
/// `trailing_comment_alignment`) that runs after this one and decides the
/// real column from the *authored* spacing; if this pass collapses the gap
/// first, that decision is lost before the alignment pass ever sees it.
///
/// The caller is responsible for only setting a flag when that owning pass
/// will actually run afterward — currently full mode only. Indent-only
/// reaches this same reducer (`format::engine`, shared with full mode) but
/// never runs the post-layout passes, so protecting a gap there would leave
/// it merely un-collapsed rather than aligned, which is not what
/// `--ws-remred` means in indent-only and would break its byte-exact findent
/// contract.
pub fn reduce_line_into_protected<F: FnMut(u8)>(
    s: &[u8],
    quote: &mut u8,
    protect_declaration_gap: bool,
    protect_comment_gap: bool,
    put: &mut F,
) {
    let mut space_start: Option<usize> = None;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if *quote != 0 {
            put(c);
            if c == *quote {
                if s.get(i + 1) == Some(&*quote) {
                    put(s[i + 1]);
                    i += 1;
                } else {
                    *quote = 0;
                }
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            flush_space(s, &mut space_start, i, false, put);
            *quote = c;
            put(c);
        } else if c == b'!' {
            flush_space(s, &mut space_start, i, protect_comment_gap, put);
            for &byte in &s[i..] {
                put(byte);
            }
            break;
        } else if c == b' ' || c == b'\t' {
            if space_start.is_none() {
                space_start = Some(i);
            }
        } else {
            let protect_here = protect_declaration_gap && c == b':' && s.get(i + 1) == Some(&b':');
            flush_space(s, &mut space_start, i, protect_here, put);
            put(c);
        }
        i += 1;
    }
}

/// Emit the whitespace run `s[start..at]` collected in `space_start`, either
/// collapsed to one space or, when `protect`, exactly as authored.
fn flush_space<F: FnMut(u8)>(
    s: &[u8],
    space_start: &mut Option<usize>,
    at: usize,
    protect: bool,
    put: &mut F,
) {
    if let Some(start) = space_start.take() {
        if protect {
            for &byte in &s[start..at] {
                put(byte);
            }
        } else {
            put(b' ');
        }
    }
}

pub fn reduce_to<W: std::io::Write>(s: &[u8], out: &mut W) -> std::io::Result<()> {
    let mut quote = 0u8;
    reduce_to_with_quote(s, &mut quote, out)
}

pub fn reduce_to_with_quote<W: std::io::Write>(
    s: &[u8],
    quote: &mut u8,
    out: &mut W,
) -> std::io::Result<()> {
    reduce_to_with_quote_protected(s, quote, false, false, out)
}

/// `reduce_to_with_quote`, with the same gap-protection flags as
/// [`reduce_line_into_protected`].
pub fn reduce_to_with_quote_protected<W: std::io::Write>(
    s: &[u8],
    quote: &mut u8,
    protect_declaration_gap: bool,
    protect_comment_gap: bool,
    out: &mut W,
) -> std::io::Result<()> {
    let mut error = None;
    reduce_line_into_protected(
        s,
        quote,
        protect_declaration_gap,
        protect_comment_gap,
        &mut |byte| {
            if error.is_none() {
                if let Err(e) = out.write_all(&[byte]) {
                    error = Some(e);
                }
            }
        },
    );
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{reduce, reduce_line_into, reduce_line_into_protected};

    #[test]
    fn continued_strings_keep_their_internal_spaces() {
        let mut output = Vec::new();
        let mut quote = 0;
        reduce_line_into(
            b"call sub(\"hello              &",
            &mut quote,
            &mut |byte| output.push(byte),
        );
        reduce_line_into(b"world  \" ,  &", &mut quote, &mut |byte| output.push(byte));
        assert_eq!(
            output,
            b"call sub(\"hello              &world  \" , &".to_vec()
        );
        assert_eq!(quote, 0);
    }

    #[test]
    fn standalone_reduction_remains_quote_aware() {
        assert_eq!(reduce(b"x   =   \"a  b\"   +  y"), b"x = \"a  b\" + y");
    }

    fn protected(s: &[u8], protect_declaration_gap: bool, protect_comment_gap: bool) -> Vec<u8> {
        let mut output = Vec::new();
        let mut quote = 0;
        reduce_line_into_protected(
            s,
            &mut quote,
            protect_declaration_gap,
            protect_comment_gap,
            &mut |byte| output.push(byte),
        );
        output
    }

    #[test]
    fn declaration_gap_is_protected_only_when_asked() {
        let line: &[u8] = b"real(dl), intent(in)  ::   x";
        assert_eq!(
            protected(line, true, false),
            b"real(dl), intent(in)  :: x".to_vec(),
        );
        assert_eq!(
            protected(line, false, false),
            b"real(dl), intent(in) :: x".to_vec(),
        );
    }

    #[test]
    fn comment_gap_is_protected_only_when_asked() {
        let line: &[u8] = b"x  =  1   ! note";
        assert_eq!(protected(line, false, true), b"x = 1   ! note".to_vec());
        assert_eq!(protected(line, false, false), b"x = 1 ! note".to_vec());
    }
}
