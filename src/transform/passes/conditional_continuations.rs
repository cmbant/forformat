//! Contextual normalization of compact conditional-compilation continuations.
//!
//! In free-form OpenMP conditional compilation, `!$&...` is a continuation
//! spelling, not an unconditional sentinel. Canonicalize it before the first
//! semantic analysis so every later pass can keep using the stable `!$ ` code
//! sentinel. The ordinary continuation pass still decides whether the leading
//! `&` is redundant or required by a continued character literal or split
//! lexical token.

use crate::{
    source::{regions::LexState, RegionKind},
    transform::{document::Document, pipeline::Changed},
};

/// Rewrite a contextually valid `!$&...` line as `!$ &...`.
///
/// A compact prefix is accepted only while the conditional-compilation stream
/// has an open continuation. Standalone `!$&...` therefore stays comment-like
/// and byte-for-byte unchanged. This pass intentionally leaves the leading `&`
/// in the Fortran body; [`super::continuations::normalize_continuations`] owns
/// removing redundant markers while preserving lexical/protected ones.
pub fn run(document: &mut Document) -> Changed {
    let original = document.lines.clone();
    let mut normalized = Vec::with_capacity(original.len());
    let mut state = LexState::default();
    let mut continuation = false;

    for original_line in &original {
        let mut line = original_line.clone();
        match conditional_prefix(original_line) {
            Some(ConditionalPrefix::BlankSeparated { body_start }) => {
                continuation =
                    advance_conditional_state(&mut state, &line[body_start..], continuation);
            }
            Some(ConditionalPrefix::Compact { sentinel_end }) if continuation => {
                line.insert(sentinel_end, b' ');
                continuation =
                    advance_conditional_state(&mut state, &line[sentinel_end + 1..], continuation);
            }
            Some(ConditionalPrefix::Compact { .. }) | None => {}
        }
        normalized.push(line);
    }

    if normalized == original {
        Changed::No
    } else {
        document.set_lines(normalized);
        Changed::Text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionalPrefix {
    BlankSeparated { body_start: usize },
    Compact { sentinel_end: usize },
}

fn conditional_prefix(line: &[u8]) -> Option<ConditionalPrefix> {
    let start = line
        .iter()
        .position(|byte| !matches!(*byte, b' ' | b'\t'))?;
    let rest = &line[start..];
    if rest.starts_with(b"!$ ") {
        Some(ConditionalPrefix::BlankSeparated {
            body_start: start + 3,
        })
    } else if rest.starts_with(b"!$&") {
        Some(ConditionalPrefix::Compact {
            sentinel_end: start + 2,
        })
    } else {
        None
    }
}

fn advance_conditional_state(
    state: &mut LexState,
    body: &[u8],
    incoming_continuation: bool,
) -> bool {
    // A continued character literal requires a leading `&`. For malformed
    // editor buffers, do not let a same-stream line without it consume or close
    // the protected state; keep looking for a valid continuation line instead.
    if state.in_literal() && !starts_with_continuation_marker(body) {
        return incoming_continuation;
    }

    let mut comment = None;
    state.scan(body, |region| {
        if comment.is_none() && region.kind == RegionKind::Comment {
            comment = Some(region.range.start);
        }
    });
    let continuation = ends_with_continuation_before(body, comment);
    if !continuation {
        *state = LexState::default();
    }
    continuation
}

fn starts_with_continuation_marker(body: &[u8]) -> bool {
    body.iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b'&')
}

fn ends_with_continuation_before(body: &[u8], comment: Option<usize>) -> bool {
    let mut end = comment.unwrap_or(body.len());
    while end > 0 && body[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end > 0 && body[end - 1] == b'&'
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::transform::{document::Document, pipeline::Changed};

    #[test]
    fn compact_prefix_is_canonicalized_only_for_an_open_conditional_continuation() {
        let mut document =
            Document::from_bytes(b"!$& standalone\n!$ x = a &\n!$& b\n!$ x = 1\n!$& closed\n");
        assert_eq!(run(&mut document), Changed::Text);
        assert_eq!(
            document.lines,
            vec![
                b"!$& standalone".to_vec(),
                b"!$ x = a &".to_vec(),
                b"!$ & b".to_vec(),
                b"!$ x = 1".to_vec(),
                b"!$& closed".to_vec(),
            ]
        );
    }

    #[test]
    fn compact_prefix_keeps_required_leading_marker_for_the_continuation_pass() {
        let mut document =
            Document::from_bytes(b"!$ sub&\n!$&routine sub\n!$ text = 'ab &\n!$&cd'\n");
        assert_eq!(run(&mut document), Changed::Text);
        assert_eq!(document.lines[1], b"!$ &routine sub".to_vec());
        assert_eq!(document.lines[3], b"!$ &cd'".to_vec());
    }

    #[test]
    fn conditional_state_steps_over_other_streams_and_comment_lines() {
        let mut document = Document::from_bytes(b"!$ x = a &\ny = 2\n! ordinary comment\n!$& b\n");
        assert_eq!(run(&mut document), Changed::Text);
        assert_eq!(document.lines[3], b"!$ & b".to_vec());
    }

    #[test]
    fn comment_ampersands_do_not_open_compact_continuations() {
        let mut document =
            Document::from_bytes(b"!$ x = 1 ! not a continuation &\n!$& untouched\n");
        let original = document.lines.clone();
        assert_eq!(run(&mut document), Changed::No);
        assert_eq!(document.lines, original);
    }

    #[test]
    fn malformed_literal_continuation_does_not_consume_protected_state() {
        let mut document =
            Document::from_bytes(b"!$ text = 'ab &\n!$ malformed without leading marker\n!$&cd'\n");
        assert_eq!(run(&mut document), Changed::Text);
        assert_eq!(
            document.lines[2],
            b"!$ &cd'".to_vec(),
            "the compact line should still see the open literal continuation"
        );
    }
}
