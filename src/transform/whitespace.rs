//! Redundant-whitespace reduction (`--ws-remred`).
//!
//! Runs of blanks between code tokens collapse to one space. Everything a
//! protected region owns — a character literal, a Hollerith payload, a trailing
//! comment — is copied byte for byte, so the reduction can never change what a
//! program says.
//!
//! Which bytes are protected is not decided here. This pass used to carry its
//! own quote state machine, a bare `quote: u8` that knew about delimiters and
//! nothing else; a Hollerith payload's blanks are positional data and it
//! collapsed them like any other run. It now walks the regions
//! [`crate::source::regions`] reports, so the answer is the same one every
//! other transform gets, and the state it carries across a continuation group is
//! a whole [`LexState`] rather than a delimiter byte.

use crate::source::{regions::RegionKind, LexState};

pub fn reduce(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut state = LexState::default();
    reduce_line_into_protected(s, &mut state, false, false, &mut |byte| out.push(byte));
    out
}

/// Reduce one physical line, carrying lexical state across free-form
/// continuation lines, with the option to leave two *kinds* of gap exactly as
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
/// will actually run afterward — see [`FormatMode::aligns_after_layout`].
/// Indent-only reaches this same reducer (`format::engine`, shared with full
/// mode) but never runs the post-layout passes, so protecting a gap there would
/// leave it merely un-collapsed rather than aligned, which is not what
/// `--ws-remred` means in indent-only and would break its byte-exact findent
/// contract.
///
/// [`FormatMode::aligns_after_layout`]: crate::config::FormatMode::aligns_after_layout
pub fn reduce_line_into_protected<F: FnMut(u8)>(
    s: &[u8],
    state: &mut LexState,
    protect_declaration_gap: bool,
    protect_comment_gap: bool,
    put: &mut F,
) {
    // A run of blanks is only emitted once something after it says what it is
    // separating, because that is what decides whether it collapses. A run with
    // nothing after it is line-terminal whitespace and is dropped.
    let mut space_start: Option<usize> = None;
    state.scan_line(s, |region| {
        let range = region.range;
        match region.kind {
            RegionKind::Code => {
                for index in range {
                    let byte = s[index];
                    if byte == b' ' || byte == b'\t' {
                        if space_start.is_none() {
                            space_start = Some(index);
                        }
                        continue;
                    }
                    let protect =
                        protect_declaration_gap && byte == b':' && s.get(index + 1) == Some(&b':');
                    flush_space(s, &mut space_start, index, protect, put);
                    put(byte);
                }
            }
            // The gap before a comment is the one an alignment pass may want.
            // The comment's own bytes are prose and are never touched.
            RegionKind::Comment => {
                flush_space(s, &mut space_start, range.start, protect_comment_gap, put);
                copy(s, range, put);
            }
            // Payload. The run of blanks *before* it is still ordinary
            // separation and still collapses; the bytes inside it are data.
            RegionKind::StringLiteral | RegionKind::Hollerith => {
                flush_space(s, &mut space_start, range.start, false, put);
                copy(s, range, put);
            }
            // Whole-line directives are a property of the line, so the byte
            // scanner never reports one; a caller that hands us one anyway gets
            // it back untouched rather than reduced.
            RegionKind::Preprocessor => copy(s, range, put),
        }
    });
}

fn copy<F: FnMut(u8)>(s: &[u8], range: std::ops::Range<usize>, put: &mut F) {
    for &byte in &s[range] {
        put(byte);
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

/// [`reduce_line_into_protected`] writing into a [`std::io::Write`].
pub fn reduce_to_protected<W: std::io::Write>(
    s: &[u8],
    state: &mut LexState,
    protect_declaration_gap: bool,
    protect_comment_gap: bool,
    out: &mut W,
) -> std::io::Result<()> {
    let mut error = None;
    reduce_line_into_protected(
        s,
        state,
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
    use super::{reduce, reduce_line_into_protected};
    use crate::source::LexState;

    fn reduce_lines(lines: &[&[u8]]) -> (Vec<u8>, LexState) {
        let mut output = Vec::new();
        let mut state = LexState::default();
        for line in lines {
            reduce_line_into_protected(line, &mut state, false, false, &mut |byte| {
                output.push(byte)
            });
        }
        (output, state)
    }

    #[test]
    fn continued_strings_keep_their_internal_spaces() {
        let (output, state) =
            reduce_lines(&[b"call sub(\"hello              &", b"world  \" ,  &"]);
        assert_eq!(
            output,
            b"call sub(\"hello              &world  \" , &".to_vec()
        );
        assert!(!state.in_literal());
    }

    #[test]
    fn standalone_reduction_remains_quote_aware() {
        assert_eq!(reduce(b"x   =   \"a  b\"   +  y"), b"x = \"a  b\" + y");
    }

    #[test]
    fn a_hollerith_payload_keeps_its_blanks() {
        // The payload length is positional: collapsing `a  b` to `a b` leaves a
        // `5H` that no longer has five characters, which is a different — and
        // invalid — program. The blanks *around* the constant are ordinary.
        assert_eq!(reduce(b"call s( 5Ha  b ,   x )"), b"call s( 5Ha  b , x )");
    }

    #[test]
    fn a_hollerith_payload_continued_onto_the_next_line_is_still_payload() {
        // Ten payload bytes, of which only five fit on the first line, so
        // `&e  f` on the second is data and its blanks are two of the ten. A
        // reducer carrying just a delimiter byte cannot know that: it starts
        // the second line believing it is in code and collapses them, which
        // leaves a `10H` holding nine characters.
        let (output, state) = reduce_lines(&[b"call s(10Habcd&", b"&e  fg,x)"]);
        assert_eq!(output, b"call s(10Habcd&&e  fg,x)".to_vec());
        assert!(!state.in_hollerith());
    }

    fn protected(s: &[u8], protect_declaration_gap: bool, protect_comment_gap: bool) -> Vec<u8> {
        let mut output = Vec::new();
        let mut state = LexState::default();
        reduce_line_into_protected(
            s,
            &mut state,
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
