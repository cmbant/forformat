//! A real token stream over free-form Fortran source.
//!
//! This is the input to every full-mode normalization rule and to the wrapper.
//! It differs from `scanner::tokens` — the classifier's deliberately minimal
//! splitter — in three ways that matter for rewriting rather than recognizing:
//!
//! * tokens carry a [`TokenKind`], so a rule can say "space around binary `+`"
//!   without re-deriving what a `+` is attached to;
//! * `10abc` lexes as a number followed by a name instead of one blob;
//! * every token carries the bracket `depth` at which it appears, which is what
//!   break-point selection ranks candidates by.
//!
//! Protected regions come from [`crate::source::regions`], so string literals,
//! comments and Hollerith payloads are single opaque tokens and can never be
//! split by a transform (I3) or by a wrap (I5).

use super::regions::{LexState, RegionKind};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// An identifier or keyword: `[A-Za-z][A-Za-z0-9_]*`.
    Name,
    /// A numeric literal, without any `_kind` suffix.
    Number,
    /// A character literal including both delimiters.
    String,
    /// `nH` and its payload.
    Hollerith,
    /// `.and.`, `.eqv.`, `.myop.` — a dotted operator or logical constant.
    DotOp,
    /// Any other operator or punctuation: `+ - * / ** // = == /= < <= > >= => :: : % _`.
    Operator,
    Comma,
    Semicolon,
    /// A free-form continuation marker.
    Ampersand,
    LParen,
    RParen,
    LBracket,
    RBracket,
    /// From `!` to end of line.
    Comment,
    /// A byte that is none of the above, including non-ASCII input.
    Other,
}

impl TokenKind {
    /// True for the kinds whose bytes must survive formatting unchanged (I3).
    pub fn is_protected(self) -> bool {
        matches!(self, TokenKind::String | TokenKind::Hollerith)
    }

    pub fn opens_bracket(self) -> bool {
        matches!(self, TokenKind::LParen | TokenKind::LBracket)
    }

    pub fn closes_bracket(self) -> bool {
        matches!(self, TokenKind::RParen | TokenKind::RBracket)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub text: &'a [u8],
    pub span: Range<usize>,
    /// Bracket nesting depth at this token.  An opening bracket carries the
    /// depth outside it; its matching closing bracket carries the same value.
    pub depth: usize,
}

impl Token<'_> {
    /// Case-insensitive comparison against an ASCII spelling.
    pub fn is(&self, word: &[u8]) -> bool {
        self.text.eq_ignore_ascii_case(word)
    }

    pub fn is_name(&self, word: &[u8]) -> bool {
        self.kind == TokenKind::Name && self.is(word)
    }
}

/// Tokenize `s`, carrying lexical state in and out so a continuation group can
/// be tokenized one physical line at a time.
pub fn tokenize<'a>(s: &'a [u8], state: &mut LexState) -> Vec<Token<'a>> {
    let mut depth = 0usize;
    let mut out = Vec::new();
    let mut regions = Vec::new();
    state.scan(s, |region| regions.push(region));
    for region in regions {
        let range = region.range.clone();
        match region.kind {
            RegionKind::StringLiteral => out.push(Token {
                kind: TokenKind::String,
                text: &s[range.clone()],
                span: range,
                depth,
            }),
            RegionKind::Hollerith => out.push(Token {
                kind: TokenKind::Hollerith,
                text: &s[range.clone()],
                span: range,
                depth,
            }),
            RegionKind::Comment => out.push(Token {
                kind: TokenKind::Comment,
                text: &s[range.clone()],
                span: range,
                depth,
            }),
            RegionKind::Preprocessor => out.push(Token {
                kind: TokenKind::Other,
                text: &s[range.clone()],
                span: range,
                depth,
            }),
            RegionKind::Code => lex_code(s, range, &mut depth, &mut out),
        }
    }
    out
}

/// Tokenize a standalone slice.
pub fn tokens(s: &[u8]) -> Vec<Token<'_>> {
    tokenize(s, &mut LexState::default())
}

/// Whether writing `right` immediately after `left` preserves the token
/// boundary between them.
///
/// Asked with the lexer rather than a table of pairs, because the question is
/// exactly what the next run of the formatter will see, and that is whatever
/// the lexer says.
///
/// The question is boundary preservation, not token count. Counting is not
/// enough: `=` and `==` written together spell `===`, which is still two
/// tokens, but they are `==` and `=` -- a different pair. `*` and `**` are the
/// same trap one operator longer.
pub(crate) fn join_preserves_boundary(left: &[u8], right: &[u8]) -> bool {
    if left.is_empty() || right.is_empty() {
        return true;
    }
    let mut joined = Vec::with_capacity(left.len() + right.len());
    joined.extend_from_slice(left);
    joined.extend_from_slice(right);
    let tokens = tokenize(&joined, &mut LexState::default());
    tokens.len() == 2
        && tokens[0].span.start == 0
        && tokens[0].span.end == left.len()
        && tokens[1].span.end == joined.len()
}

fn lex_code<'a>(s: &'a [u8], range: Range<usize>, depth: &mut usize, out: &mut Vec<Token<'a>>) {
    let mut i = range.start;
    let end = range.end;
    while i < end {
        let c = s[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let kind = if c.is_ascii_alphabetic() {
            i += 1;
            while i < end && (s[i].is_ascii_alphanumeric() || s[i] == b'_') {
                i += 1;
            }
            TokenKind::Name
        } else if c.is_ascii_digit() || (c == b'.' && s.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i = lex_number(s, i, end);
            TokenKind::Number
        } else if c == b'.' && s.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            let mut j = i + 1;
            while j < end && s[j].is_ascii_alphabetic() {
                j += 1;
            }
            if s.get(j) == Some(&b'.') {
                i = j + 1;
                TokenKind::DotOp
            } else {
                i += 1;
                TokenKind::Operator
            }
        } else {
            i += 1;
            match c {
                b',' => TokenKind::Comma,
                b';' => TokenKind::Semicolon,
                b'&' => TokenKind::Ampersand,
                b'(' => {
                    let token = TokenKind::LParen;
                    out.push(Token {
                        kind: token,
                        text: &s[start..i],
                        span: start..i,
                        depth: *depth,
                    });
                    *depth += 1;
                    continue;
                }
                b'[' => {
                    out.push(Token {
                        kind: TokenKind::LBracket,
                        text: &s[start..i],
                        span: start..i,
                        depth: *depth,
                    });
                    *depth += 1;
                    continue;
                }
                b')' | b']' => {
                    *depth = depth.saturating_sub(1);
                    out.push(Token {
                        kind: if c == b')' {
                            TokenKind::RParen
                        } else {
                            TokenKind::RBracket
                        },
                        text: &s[start..i],
                        span: start..i,
                        depth: *depth,
                    });
                    continue;
                }
                b'*' | b'/' | b'=' | b'<' | b'>' | b':' => {
                    if i < end && is_operator_pair(c, s[i]) {
                        i += 1;
                    }
                    TokenKind::Operator
                }
                b'+' | b'-' | b'%' | b'_' | b'$' | b'?' | b'@' | b'\\' | b'^' | b'~' | b'|'
                | b'.' => TokenKind::Operator,
                _ => TokenKind::Other,
            }
        };
        out.push(Token {
            kind,
            text: &s[start..i],
            span: start..i,
            depth: *depth,
        });
    }
}

fn is_operator_pair(first: u8, second: u8) -> bool {
    matches!(
        (first, second),
        (b'*', b'*')
            | (b'/', b'/')
            | (b'/', b'=')
            | (b'=', b'=')
            | (b'=', b'>')
            | (b'<', b'=')
            | (b'>', b'=')
            | (b':', b':')
    )
}

/// Lex one numeric literal.  A `_kind` suffix is deliberately left out so the
/// kind name is an ordinary [`TokenKind::Name`] that case normalization can
/// reach.
fn lex_number(s: &[u8], mut i: usize, end: usize) -> usize {
    while i < end && s[i].is_ascii_digit() {
        i += 1;
    }
    if s.get(i) == Some(&b'.') && i < end {
        // `1.0` and a bare `1.` are reals; `1.and.2` is an integer followed by a
        // dotted operator; `1.e5` is a real with an exponent.
        let next = s.get(i + 1).copied();
        let dotted_operator = next.is_some_and(|byte| byte.is_ascii_alphabetic())
            && exponent_at(s, i + 1, end).is_none();
        if !dotted_operator {
            i += 1;
            while i < end && s[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    if let Some(after) = exponent_at(s, i, end) {
        i = after;
    }
    i
}

/// The offset past an exponent part starting at `i`, when one is present.
fn exponent_at(s: &[u8], i: usize, end: usize) -> Option<usize> {
    if i >= end || !matches!(s[i], b'e' | b'E' | b'd' | b'D' | b'q' | b'Q') {
        return None;
    }
    let mut j = i + 1;
    if j < end && (s[j] == b'+' || s[j] == b'-') {
        j += 1;
    }
    if j >= end || !s[j].is_ascii_digit() {
        return None;
    }
    while j < end && s[j].is_ascii_digit() {
        j += 1;
    }
    Some(j)
}

#[cfg(test)]
mod tests {
    use super::{join_preserves_boundary, tokenize, tokens, TokenKind};
    use crate::source::regions::LexState;

    fn lexed(s: &[u8]) -> Vec<(TokenKind, String)> {
        tokens(s)
            .into_iter()
            .map(|token| (token.kind, String::from_utf8_lossy(token.text).into_owned()))
            .collect()
    }

    #[test]
    fn tokens_never_lose_or_reorder_source_bytes() {
        for source in [
            b"call sub(a, b(1), 'x')".as_slice(),
            b"if (x .eq. 1.0d0) then".as_slice(),
            b"real(kind=dp) :: a = 1.0_dp".as_slice(),
            b"x = 10abc".as_slice(),
            b"y = a // 'q' ! tail".as_slice(),
        ] {
            let mut end = 0;
            for token in tokens(source) {
                assert!(token.span.start >= end, "overlap in {source:?}");
                assert_eq!(token.text, &source[token.span.clone()]);
                end = token.span.end;
            }
            assert!(end <= source.len());
        }
    }

    #[test]
    fn a_digit_prefixed_identifier_is_two_tokens() {
        assert_eq!(
            lexed(b"10abc"),
            [
                (TokenKind::Number, "10".to_string()),
                (TokenKind::Name, "abc".to_string()),
            ]
        );
    }

    #[test]
    fn numeric_literals_keep_exponents_and_release_kind_names() {
        assert_eq!(
            lexed(b"1.0d0 1.e5 1. 1.and.2 .5 1.0_dp"),
            [
                (TokenKind::Number, "1.0d0".to_string()),
                (TokenKind::Number, "1.e5".to_string()),
                (TokenKind::Number, "1.".to_string()),
                (TokenKind::Number, "1".to_string()),
                (TokenKind::DotOp, ".and.".to_string()),
                (TokenKind::Number, "2".to_string()),
                (TokenKind::Number, ".5".to_string()),
                (TokenKind::Number, "1.0".to_string()),
                (TokenKind::Operator, "_".to_string()),
                (TokenKind::Name, "dp".to_string()),
            ]
        );
    }

    #[test]
    fn multi_character_operators_stay_whole() {
        assert_eq!(
            lexed(b"a**b//c==d/=e<=f>=g=>h::i%j"),
            [
                (TokenKind::Name, "a".to_string()),
                (TokenKind::Operator, "**".to_string()),
                (TokenKind::Name, "b".to_string()),
                (TokenKind::Operator, "//".to_string()),
                (TokenKind::Name, "c".to_string()),
                (TokenKind::Operator, "==".to_string()),
                (TokenKind::Name, "d".to_string()),
                (TokenKind::Operator, "/=".to_string()),
                (TokenKind::Name, "e".to_string()),
                (TokenKind::Operator, "<=".to_string()),
                (TokenKind::Name, "f".to_string()),
                (TokenKind::Operator, ">=".to_string()),
                (TokenKind::Name, "g".to_string()),
                (TokenKind::Operator, "=>".to_string()),
                (TokenKind::Name, "h".to_string()),
                (TokenKind::Operator, "::".to_string()),
                (TokenKind::Name, "i".to_string()),
                (TokenKind::Operator, "%".to_string()),
                (TokenKind::Name, "j".to_string()),
            ]
        );
    }

    #[test]
    fn joined_spellings_report_token_boundary_preservation() {
        assert!(join_preserves_boundary(b"+", b"-"));
        assert!(join_preserves_boundary(b"", b"="));
        assert!(join_preserves_boundary(b"=", b""));
        assert!(!join_preserves_boundary(b"/", b"/"));
        assert!(!join_preserves_boundary(b"=", b"=="));
        assert!(!join_preserves_boundary(b"*", b"**"));
    }

    #[test]
    fn bracket_depth_is_recorded_for_break_point_ranking() {
        let depths: Vec<(String, usize)> = tokens(b"call f(a, g(b, c), d)")
            .into_iter()
            .map(|token| {
                (
                    String::from_utf8_lossy(token.text).into_owned(),
                    token.depth,
                )
            })
            .collect();
        assert_eq!(depths[0], ("call".to_string(), 0));
        assert_eq!(depths[2], ("(".to_string(), 0));
        assert_eq!(depths[4], (",".to_string(), 1));
        assert_eq!(depths[6], ("(".to_string(), 1));
        assert_eq!(depths[8], (",".to_string(), 2));
        assert_eq!(depths.last().unwrap(), &(")".to_string(), 0));
    }

    #[test]
    fn protected_regions_are_single_tokens_across_lines() {
        let mut state = LexState::default();
        let first = tokenize(b"call sub('a, b &", &mut state);
        assert_eq!(first.last().unwrap().kind, TokenKind::String);
        assert!(state.in_literal());
        let second = tokenize(b"c', x)", &mut state);
        assert_eq!(second[0].kind, TokenKind::String);
        assert_eq!(second[0].text, b"c'");
        assert_eq!(
            lexed(b"x = 3Ha,b ! c"),
            [
                (TokenKind::Name, "x".to_string()),
                (TokenKind::Operator, "=".to_string()),
                (TokenKind::Hollerith, "3Ha,b".to_string()),
                (TokenKind::Comment, "! c".to_string()),
            ]
        );
    }
}
