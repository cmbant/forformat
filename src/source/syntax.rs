//! Policy-free recognition helpers shared by formatter passes.
//!
//! Keep only source-shape questions here. Formatting choices belong in the
//! transform passes that consume these predicates.

use super::{Token, TokenKind};

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
    use super::{is_directive_comment, is_end_construct_keyword};
    use crate::source::tokens::tokens;

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
