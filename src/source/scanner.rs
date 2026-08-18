//! The classifier's minimal splitter.
//!
//! This is deliberately *not* the full-mode token stream: the recognizer chain
//! in `classify::recognizers` encodes hundreds of findent 4.3.7 edge cases
//! against exactly these token boundaries, so it keeps its own conservative
//! lexer.  Rewriting transforms use [`crate::source::tokens`] instead.  Both
//! share one protected-region walker, which is the duplication that mattered.

use super::regions::{comment_start, LexState, RegionKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub text: &'a [u8],
    pub start: usize,
    pub end: usize,
}

/// Lazily iterate classifier tokens without materializing a token vector.
pub fn iter_tokens(s: &[u8]) -> impl Iterator<Item = Token<'_>> + '_ {
    let mut i = 0;
    let end = comment_start(s).unwrap_or(s.len());
    std::iter::from_fn(move || {
        while i < end && s[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            return None;
        }
        let start = i;
        let c = s[i];
        if c == b'\'' || c == b'"' {
            let q = c;
            i += 1;
            while i < end {
                if s[i] == q {
                    if s.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if c.is_ascii_alphanumeric() || c == b'_' {
            i += 1;
            while i < end && (s[i].is_ascii_alphanumeric() || s[i] == b'_') {
                i += 1;
            }
        } else if c == b'.' && s.get(i + 1).is_some_and(|x| x.is_ascii_alphabetic()) {
            i += 1;
            while i < end && s[i] != b'.' {
                i += 1;
            }
            if i < end {
                i += 1;
            }
        } else {
            i += 1;
        }
        Some(Token {
            text: &s[start..i],
            start,
            end: i,
        })
    })
}

pub fn tokens(s: &[u8]) -> Vec<Token<'_>> {
    iter_tokens(s).collect()
}

/// Split a joined logical line at semicolons that are real statement
/// separators.  A trailing comment is never split and stays attached to the
/// last statement, matching findent.
pub fn split_statements(s: &[u8]) -> Vec<&[u8]> {
    split_statement_ranges(s)
        .into_iter()
        .map(|range| &s[range])
        .collect()
}

/// The same split, reported as ranges so a caller can keep provenance.
pub fn split_statement_ranges(s: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut separators = Vec::new();
    for region in LexState::default().regions(s) {
        match region.kind {
            RegionKind::Comment => break,
            RegionKind::Code => {
                for (offset, byte) in s[region.range.clone()].iter().enumerate() {
                    if *byte == b';' {
                        separators.push(region.range.start + offset);
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(separators.len() + 1);
    let mut start = 0;
    for index in separators {
        out.push(start..index);
        start = index + 1;
    }
    out.push(start..s.len());
    out
}

#[cfg(test)]
mod tests {
    use super::{split_statements, tokens};

    #[test]
    fn statement_splitting_respects_strings_and_hollerith() {
        let parts = split_statements(b"a='x;y'; b=3H;!;; c=1");
        assert_eq!(
            parts,
            [b"a='x;y'".as_slice(), b" b=3H;!;".as_slice(), b" c=1"]
        );
    }

    #[test]
    fn tokens_keep_dot_operators_as_one_opaque_token() {
        let values: Vec<&[u8]> = tokens(b"if (x .EQ. y) then")
            .into_iter()
            .map(|token| token.text)
            .collect();
        assert!(values
            .iter()
            .any(|token| token.eq_ignore_ascii_case(b".EQ.")));
    }
}
