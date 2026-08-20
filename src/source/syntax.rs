//! Policy-free recognition helpers shared by formatter passes.
//!
//! Keep only source-shape questions here. Formatting choices belong in the
//! transform passes that consume these predicates.

use super::{Token, TokenKind};

/// The two free-form conditional-compilation prefix shapes the formatter
/// accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConditionalPrefixKind {
    /// An initial (or non-compact continued) line: `!$ ` / `!$\t`.
    InitialBlank,
    /// A continued line whose continuation marker immediately follows the
    /// sentinel: `!$&...`.
    CompactContinuation,
}

/// Parsed free-form conditional-compilation prefix.
///
/// `body_start` always points at the first byte that belongs to the Fortran
/// body. For a compact continuation that byte is the `&` itself, because it is
/// real Fortran continuation syntax rather than part of the sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConditionalPrefix {
    pub body_start: usize,
    pub kind: ConditionalPrefixKind,
}

/// Parse the free-form conditional-compilation sentinel at the start of a
/// physical line (after optional horizontal indentation).
///
/// An initial OpenMP conditional-compilation line uses `!$` followed by a
/// blank. A continued line may instead put the continuation marker directly
/// after the sentinel, as in `!$& index`. Joined spellings such as `!$OMP` and
/// `!$acc`, and bare `!$`, are not conditional-compilation code.
pub(crate) fn conditional_compilation_prefix(line: &[u8]) -> Option<ConditionalPrefix> {
    let start = line.iter().position(|byte| !matches!(byte, b' ' | b'\t'))?;
    if !line
        .get(start..)
        .is_some_and(|rest| rest.starts_with(b"!$"))
    {
        return None;
    }
    match line.get(start + 2) {
        Some(b' ' | b'\t') => Some(ConditionalPrefix {
            body_start: start + 3,
            kind: ConditionalPrefixKind::InitialBlank,
        }),
        Some(b'&') => Some(ConditionalPrefix {
            body_start: start + 2,
            kind: ConditionalPrefixKind::CompactContinuation,
        }),
        _ => None,
    }
}

/// Start of the Fortran body of a free-form conditional-compilation line.
///
/// Use [`conditional_compilation_prefix`] when the caller needs to distinguish
/// an ordinary sentinel blank from the compact `!$&` continuation spelling.
pub(crate) fn conditional_compilation_body_start(line: &[u8]) -> Option<usize> {
    conditional_compilation_prefix(line).map(|prefix| prefix.body_start)
}

/// Number of leading tokens occupied by a declaration type head.
///
/// This is shared by declaration indexing and continuation-line formatting so
/// additions cannot drift between the two paths. It covers standard type heads
/// from older Fortran through Fortran 2023, the standard optional-blank
/// `DOUBLEPRECISION` spelling, and the widely supported `DOUBLE COMPLEX`
/// extension. Old-style kind forms such as `INTEGER*1` and `REAL*16` are
/// covered by their ordinary one-token type heads.
pub(crate) fn declaration_type_head_len(tokens: &[Token<'_>], first: usize) -> Option<usize> {
    let head = tokens.get(first)?;
    if head.kind != TokenKind::Name {
        return None;
    }
    if head.is_name(b"double") {
        return tokens.get(first + 1).and_then(|next| {
            (next.is_name(b"precision") || next.is_name(b"complex")).then_some(2)
        });
    }
    matches!(
        head.text.to_ascii_lowercase().as_slice(),
        b"integer"
            | b"real"
            | b"complex"
            | b"logical"
            | b"character"
            | b"type"
            | b"class"
            | b"typeof"
            | b"classof"
            | b"doubleprecision"
    )
    .then_some(1)
}

/// Whether `tokens[index]` is the leading `END` of a block-end statement.
pub(crate) fn is_end_construct_keyword(tokens: &[Token<'_>], index: usize) -> bool {
    if !tokens[index].is_name(b"end") {
        return false;
    }
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    if index != first {
        return false;
    }
    match tokens.get(first + 1) {
        None => true,
        Some(next) => matches!(
            next.text.to_ascii_lowercase().as_slice(),
            b"do"
                | b"if"
                | b"where"
                | b"forall"
                | b"select"
                | b"associate"
                | b"block"
                | b"critical"
                | b"type"
                | b"interface"
                | b"enum"
                | b"enumeration"
                | b"function"
                | b"subroutine"
                | b"program"
                | b"module"
                | b"submodule"
                | b"procedure"
                | b"blockdata"
                | b"team"
                | b"structure"
                | b"union"
                | b"map"
        ),
    }
}

/// Whether a `!` comment is a directive/sentinel rather than ordinary prose.
pub(crate) fn is_directive_comment(comment: &[u8]) -> bool {
    if comment.len() < 2 || comment[0] != b'!' {
        return false;
    }
    if comment[1] == b'$' {
        return true;
    }
    [b"dir$".as_slice(), b"dec$", b"gcc$"].iter().any(|prefix| {
        comment[1..].len() >= prefix.len()
            && comment[1..1 + prefix.len()].eq_ignore_ascii_case(prefix)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        conditional_compilation_body_start, conditional_compilation_prefix,
        declaration_type_head_len, is_directive_comment, is_end_construct_keyword,
        ConditionalPrefix, ConditionalPrefixKind,
    };
    use crate::source::tokens::tokens;

    #[test]
    fn conditional_compilation_parses_initial_and_compact_prefixes() {
        for (line, expected) in [
            (
                b"!$ x".as_slice(),
                Some(ConditionalPrefix {
                    body_start: 3,
                    kind: ConditionalPrefixKind::InitialBlank,
                }),
            ),
            (
                b"  !$\tx",
                Some(ConditionalPrefix {
                    body_start: 5,
                    kind: ConditionalPrefixKind::InitialBlank,
                }),
            ),
            (
                b"!$  x",
                Some(ConditionalPrefix {
                    body_start: 3,
                    kind: ConditionalPrefixKind::InitialBlank,
                }),
            ),
            (
                b"!$& x",
                Some(ConditionalPrefix {
                    body_start: 2,
                    kind: ConditionalPrefixKind::CompactContinuation,
                }),
            ),
            (
                b"  !$&x",
                Some(ConditionalPrefix {
                    body_start: 4,
                    kind: ConditionalPrefixKind::CompactContinuation,
                }),
            ),
            (b"!$", None),
            (b"!$OMP parallel", None),
            (b"!$acc parallel", None),
            (b"! ordinary", None),
        ] {
            assert_eq!(conditional_compilation_prefix(line), expected, "{line:?}");
            assert_eq!(
                conditional_compilation_body_start(line),
                expected.map(|prefix| prefix.body_start),
                "{line:?}"
            );
        }
    }

    #[test]
    fn declaration_type_heads_cover_standard_history_and_safe_extensions() {
        for (source, expected) in [
            (b"INTEGER*1 i".as_slice(), 1),
            (b"REAL*16 x", 1),
            (b"COMPLEX*16 z", 1),
            (b"LOGICAL*1 flag", 1),
            (b"CHARACTER*20 text", 1),
            (b"DOUBLE PRECISION x", 2),
            (b"DOUBLEPRECISION x", 1),
            (b"TYPEOF(x) y", 1),
            (b"CLASSOF(x) y", 1),
            (b"DOUBLE COMPLEX z", 2),
        ] {
            let tokens = tokens(source);
            assert_eq!(declaration_type_head_len(&tokens, 0), Some(expected));
        }

        // BYTE is a common extension too, but it is also an ordinary and
        // plausible identifier; keep the shape recognizer conservative.
        let byte_assignment = tokens(b"BYTE = value");
        assert_eq!(declaration_type_head_len(&byte_assignment, 0), None);
    }

    #[test]
    fn end_construct_recognition_is_shape_only() {
        let end_do = tokens(b"END DO loop");
        assert!(is_end_construct_keyword(&end_do, 0));

        let end_enumeration = tokens(b"END ENUMERATION TYPE colour");
        assert!(is_end_construct_keyword(&end_enumeration, 0));

        let expression = tokens(b"x = end + 1");
        assert!(!is_end_construct_keyword(&expression, 2));
    }

    #[test]
    fn directive_comment_recognition_covers_supported_sentinels() {
        for comment in [
            b"!$omp parallel".as_slice(),
            b"!DIR$ vector",
            b"!dec$ attrs",
            b"!GCC$ x",
        ] {
            assert!(is_directive_comment(comment));
        }
        assert!(!is_directive_comment(b"! ordinary comment"));
    }
}
