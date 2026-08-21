//! Policy-free recognition helpers shared by formatter passes.
//!
//! Keep only source-shape questions here. Formatting choices belong in the
//! transform passes that consume these predicates.

use super::{Token, TokenKind};

/// Which semantic Fortran source stream a physical line belongs to.
///
/// Conditional-compilation source is independent of ordinary source for
/// continuation and protected-region state, even though both use the same
/// Fortran syntax when active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SourceStream {
    #[default]
    Ordinary,
    Conditional,
}

impl SourceStream {
    pub(crate) fn is_conditional(self) -> bool {
        matches!(self, Self::Conditional)
    }
}

/// The two free-form conditional-compilation prefix shapes the source-shape
/// recognizer reports. Whether a compact prefix belongs to the conditional
/// stream is contextual and is decided by `SourceBuffer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConditionalPrefixKind {
    /// A sentinel separated from its body by a horizontal blank: `!$ ` / `!$\t`.
    BlankSeparated,
    /// A continued line whose continuation marker immediately follows the
    /// sentinel: `!$&...`.
    CompactContinuation,
}

/// Parsed free-form conditional-compilation prefix.
///
/// `body_start` is the offset just past the sentinel, which is where the
/// Fortran body *begins*, not necessarily where its first nonblank byte is:
/// `!$  x` reports the second space, because only the first one is the
/// sentinel's separator and the rest is the body's own leading indentation. A
/// caller that wants the first significant byte trims from here.
///
/// For a compact continuation the offset lands on the `&` itself, because that
/// is real Fortran continuation syntax rather than part of the sentinel.
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
/// after the sentinel, as in `!$& index`. This function recognizes that compact
/// *shape* without deciding whether a continuation is actually open; semantic
/// stream classification belongs to `SourceBuffer`. Joined spellings such as
/// `!$OMP` and `!$acc`, and bare `!$`, are not conditional-compilation code.
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
            kind: ConditionalPrefixKind::BlankSeparated,
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
/// a blank-separated sentinel from the compact `!$&` continuation spelling.
pub(crate) fn conditional_compilation_body_start(line: &[u8]) -> Option<usize> {
    conditional_compilation_prefix(line).map(|prefix| prefix.body_start)
}

/// Which reserved free-form OpenMP directive sentinel introduced a line.
///
/// This says which sentinel was parsed, never how to spell it: the sentinel
/// word is a keyword and follows `--keyword-case`, so a caller that re-emits
/// one copies the spelling the document already settled on rather than a
/// canonical constant. A constant here is what previously made a wrapped
/// directive disagree with the normalized one and cost the fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMpDirectiveSentinel {
    Omp,
    Ompx,
}

/// Parsed free-form OpenMP directive prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenMpDirectivePrefix {
    /// First byte after the sentinel word itself.
    pub sentinel_end: usize,
    /// First nonblank byte of the directive body, or the line length.
    pub body_start: usize,
    pub sentinel: OpenMpDirectiveSentinel,
}

/// Parse either reserved free-form OpenMP directive sentinel, `!$omp` or
/// `!$ompx`, after optional horizontal indentation.
///
/// Initial directive lines require whitespace after the sentinel; continued
/// lines may put `&` directly after it. This recognizer therefore accepts a
/// boundary of end-of-line, horizontal whitespace, or `&` and leaves the
/// caller to decide whether a particular physical line is a valid initial or
/// continuation directive in context.
pub(crate) fn openmp_directive_prefix(line: &[u8]) -> Option<OpenMpDirectivePrefix> {
    let start = line.iter().position(|byte| !matches!(byte, b' ' | b'\t'))?;
    let rest = line.get(start..)?;
    if rest.get(..2).is_none_or(|prefix| prefix != b"!$") {
        return None;
    }

    let (sentinel, sentinel_len) = if rest
        .get(2..6)
        .is_some_and(|bytes| bytes.eq_ignore_ascii_case(b"ompx"))
    {
        (OpenMpDirectiveSentinel::Ompx, 6)
    } else if rest
        .get(2..5)
        .is_some_and(|bytes| bytes.eq_ignore_ascii_case(b"omp"))
    {
        (OpenMpDirectiveSentinel::Omp, 5)
    } else {
        return None;
    };

    let boundary = rest.get(sentinel_len);
    if !boundary.is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'&')) {
        return None;
    }

    let sentinel_end = start + sentinel_len;
    let mut body_start = sentinel_end;
    while body_start < line.len() && matches!(line[body_start], b' ' | b'\t') {
        body_start += 1;
    }
    Some(OpenMpDirectivePrefix {
        sentinel_end,
        body_start,
        sentinel,
    })
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
        openmp_directive_prefix, ConditionalPrefix, ConditionalPrefixKind, OpenMpDirectivePrefix,
        OpenMpDirectiveSentinel,
    };
    use crate::source::tokens::tokens;

    #[test]
    fn conditional_compilation_parses_initial_and_compact_prefixes() {
        for (line, expected) in [
            (
                b"!$ x".as_slice(),
                Some(ConditionalPrefix {
                    body_start: 3,
                    kind: ConditionalPrefixKind::BlankSeparated,
                }),
            ),
            (
                b"  !$\tx",
                Some(ConditionalPrefix {
                    body_start: 5,
                    kind: ConditionalPrefixKind::BlankSeparated,
                }),
            ),
            (
                b"!$  x",
                Some(ConditionalPrefix {
                    body_start: 3,
                    kind: ConditionalPrefixKind::BlankSeparated,
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
            (b"!$OMPX vendor", None),
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
    fn openmp_directive_prefixes_cover_omp_and_ompx() {
        for (line, expected) in [
            (
                b"!$omp parallel".as_slice(),
                Some(OpenMpDirectivePrefix {
                    sentinel_end: 5,
                    body_start: 6,
                    sentinel: OpenMpDirectiveSentinel::Omp,
                }),
            ),
            (
                b"  !$OMP&do",
                Some(OpenMpDirectivePrefix {
                    sentinel_end: 7,
                    body_start: 7,
                    sentinel: OpenMpDirectiveSentinel::Omp,
                }),
            ),
            (
                b"!$ompx vendor",
                Some(OpenMpDirectivePrefix {
                    sentinel_end: 6,
                    body_start: 7,
                    sentinel: OpenMpDirectiveSentinel::Ompx,
                }),
            ),
            (
                b"\t!$OMPX&vendor",
                Some(OpenMpDirectivePrefix {
                    sentinel_end: 7,
                    body_start: 7,
                    sentinel: OpenMpDirectiveSentinel::Ompx,
                }),
            ),
            (b"!$ompxx vendor", None),
            (b"!$ompish vendor", None),
            (b"!$ x", None),
        ] {
            assert_eq!(openmp_directive_prefix(line), expected, "{line:?}");
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
            b"!$ompx vendor",
            b"!DIR$ vector",
            b"!dec$ attrs",
            b"!GCC$ x",
        ] {
            assert!(is_directive_comment(comment));
        }
        assert!(!is_directive_comment(b"! ordinary comment"));
    }
}
