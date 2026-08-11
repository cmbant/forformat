//! Break-point selection and reflow.
//!
//! The division of labour matters more than any single heuristic:
//!
//! * the **wrapper** decides *where* a statement breaks;
//! * the **layout plan** decides *where the next physical line begins*.
//!
//! The reference Python formatter conflates the two — it hard-codes
//! `indent + 4` for continuations, which happens to agree with the CAMB
//! profile's `--indent_continuation=4` and would silently disagree the moment
//! anyone passed a different `-k`.  Here the continuation column always comes
//! from [`ContinuationLayout`], which the planner fills in.
//!
//! What *is* ported from Python is the ranking of candidate break points, which
//! encodes real taste about where a human would split an expression.

use crate::source::{
    regions::LexState,
    tokens::{tokenize, Token, TokenKind},
};

/// Operator tiers, loosest binding first.  A break on a looser operator is
/// preferred, because it separates larger pieces of meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BreakTier {
    Comma,
    Equivalence,
    Disjunction,
    Conjunction,
    Comparison,
    Concatenation,
    Additive,
    Multiplicative,
}

/// Where the next physical line of a wrapped statement starts.
///
/// `continuation` is whatever the layout plan says: the configured continuation
/// indent, or an active parenthesis alignment column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinuationLayout {
    pub first_indent: usize,
    pub continuation: usize,
}

/// Why a statement was left as it is.  Every long line the formatter declines to
/// wrap must be explainable, so the corpus check can separate "unwrappable by
/// design" from "wrapper bug".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decline {
    /// The statement already fits.
    Fits,
    /// No break point is safe: every candidate would split a token or leave the
    /// remainder still too long.
    NoSafeBreak,
    /// A character literal continues across physical lines (I5).
    ContinuedLiteral,
    /// Hollerith makes tokenization positional and reflow unsafe.
    Hollerith,
}

/// Reflow one statement body into physical lines.
///
/// `body` is the joined statement text without indentation, without
/// continuation markers, and without a trailing inline comment — the caller
/// detaches that above the statement first, because a comment cannot sit
/// between a continuation marker and the text it continues.
///
/// Each returned line except the last already carries its ` &` marker.  The
/// caller places line 0 at `layout.first_indent` and the rest at
/// `layout.continuation`.
pub fn wrap_body(
    body: &[u8],
    layout: ContinuationLayout,
    line_length: usize,
) -> Result<Vec<Vec<u8>>, Decline> {
    let mut state = LexState::default();
    let tokens = tokenize(body, &mut state);
    if state.in_literal() {
        return Err(Decline::ContinuedLiteral);
    }
    if tokens.iter().any(|t| t.kind == TokenKind::Hollerith) {
        return Err(Decline::Hollerith);
    }
    if layout.first_indent + body.len() <= line_length {
        return Err(Decline::Fits);
    }

    let mut out = Vec::new();
    let mut rest = body;
    let mut current = layout.first_indent;
    let mut first_break = true;
    while current + rest.len() > line_length {
        // Two columns are reserved for the ` &` this line will end with.
        let limit = line_length.saturating_sub(current + 2);
        let mut position = None;
        if first_break {
            if let Some(candidate) = assignment_wrap_position(rest, limit) {
                let remainder = trim_start(&rest[candidate..]);
                if layout.continuation + remainder.len() <= line_length {
                    position = Some(candidate);
                }
            }
        }
        let position = position.or_else(|| wrap_position(rest, limit));
        let Some(position) = position else {
            return Err(Decline::NoSafeBreak);
        };
        let mut line = trim_end(&rest[..position]).to_vec();
        line.extend_from_slice(b" &");
        out.push(line);
        rest = trim_start(&rest[position..]);
        current = layout.continuation;
        first_break = false;
    }
    out.push(rest.to_vec());
    Ok(out)
}

/// The best break position at or before `limit`, as an offset into `body`.
///
/// A statement's head — a declaration's attribute list up to `::`, or an
/// assignment's left-hand side up to `=` — is kept intact whenever the boundary
/// at its end is in reach, because that boundary is the one a human would use.
pub fn wrap_position(body: &[u8], limit: usize) -> Option<usize> {
    let tokens = tokenize(body, &mut LexState::default());
    let limit = scan_limit(&tokens, limit);
    let head_end = statement_head_end(&tokens, limit);
    if head_end > 0 {
        return operator_break_position(&tokens, head_end, limit).or(Some(head_end));
    }
    operator_break_position(&tokens, 0, limit)
        .or_else(|| assignment_break_position(&tokens, limit))
        .or_else(|| whitespace_break_position(body, &tokens, limit))
}

/// Stop scanning at an inline comment marker.
fn scan_limit(tokens: &[Token], limit: usize) -> usize {
    tokens
        .iter()
        .find(|token| token.kind == TokenKind::Comment)
        .map_or(limit, |token| limit.min(token.span.start))
}

/// The tiered operator search.  Candidates are ranked by, in order: filling the
/// line at all, shallowest bracket depth, loosest operator, and then the
/// rightmost position.
fn operator_break_position(tokens: &[Token], start: usize, limit: usize) -> Option<usize> {
    let minimum_fill = (limit as f64 * crate::transform::vocab::MINIMUM_BREAK_FILL) as usize;
    let mut best: Option<(bool, usize, BreakTier, usize)> = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.span.start < start {
            continue;
        }
        let position = token.span.end;
        if position > limit {
            break;
        }
        let Some(tier) = break_tier(tokens, index) else {
            continue;
        };
        let candidate = (position < minimum_fill, token.depth, tier, position);
        let better = match &best {
            None => true,
            Some(current) => {
                (candidate.0, candidate.1, candidate.2) < (current.0, current.1, current.2)
                    || ((candidate.0, candidate.1, candidate.2)
                        == (current.0, current.1, current.2)
                        && candidate.3 > current.3)
            }
        };
        if better {
            best = Some(candidate);
        }
    }
    best.map(|candidate| candidate.3)
}

/// The tier of a token used as a break operator, if it is one.
fn break_tier(tokens: &[Token], index: usize) -> Option<BreakTier> {
    let token = &tokens[index];
    match token.kind {
        TokenKind::Comma => Some(BreakTier::Comma),
        TokenKind::DotOp => {
            if token.is(b".eqv.") || token.is(b".neqv.") {
                Some(BreakTier::Equivalence)
            } else if token.is(b".or.") {
                Some(BreakTier::Disjunction)
            } else if token.is(b".and.") {
                Some(BreakTier::Conjunction)
            } else {
                None
            }
        }
        TokenKind::Operator => match token.text {
            b"==" | b"/=" | b"<=" | b">=" | b"<" | b">" => Some(BreakTier::Comparison),
            b"//" => Some(BreakTier::Concatenation),
            // A `+` or `-` is a break candidate only when it is spelled as a
            // binary operator with space on both sides; otherwise it may be a
            // sign, and breaking after it would strand the sign.
            b"+" | b"-" => spaced_binary(tokens, index).then_some(BreakTier::Additive),
            b"**" | b"*" => Some(BreakTier::Multiplicative),
            // `/)` closes an array constructor and `/=` is a comparison; both
            // are excluded by the tokenizer, but a `/` immediately before `)`
            // is still the tail of a legacy constructor.
            b"/" => (!next_is(tokens, index, b")")).then_some(BreakTier::Multiplicative),
            _ => None,
        },
        _ => None,
    }
}

fn next_is(tokens: &[Token], index: usize, text: &[u8]) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.text == text && token.span.start == tokens[index].span.end)
}

/// True when the operator has whitespace on both sides in the source.
fn spaced_binary(tokens: &[Token], index: usize) -> bool {
    let token = &tokens[index];
    let before = tokens
        .get(index.wrapping_sub(1))
        .is_none_or(|previous| previous.span.end < token.span.start);
    let after = tokens
        .get(index + 1)
        .is_none_or(|next| next.span.start > token.span.end);
    index > 0 && before && after
}

/// The end of the part of a statement a break must not split.
fn statement_head_end(tokens: &[Token], limit: usize) -> usize {
    for token in tokens {
        if token.span.end > limit {
            break;
        }
        if token.depth == 0 && token.kind == TokenKind::Operator && token.text == b"::" {
            return token.span.end;
        }
    }
    for (index, token) in tokens.iter().enumerate() {
        if token.span.end > limit {
            break;
        }
        if token.depth == 0 && is_assignment(token) && !is_named_argument(tokens, index) {
            return token.span.end;
        }
    }
    0
}

/// The last top-level assignment break before `limit`.
fn assignment_break_position(tokens: &[Token], limit: usize) -> Option<usize> {
    let mut position = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.span.end > limit {
            break;
        }
        if is_assignment(token) && !is_named_argument(tokens, index) {
            position = Some(token.span.end);
        }
    }
    position
}

/// The first assignment break, preferred for the first break of a statement so
/// a long right-hand side starts on its own line.
fn assignment_wrap_position(body: &[u8], limit: usize) -> Option<usize> {
    let tokens = tokenize(body, &mut LexState::default());
    for (index, token) in tokens.iter().enumerate() {
        if token.span.start >= limit {
            break;
        }
        if token.kind == TokenKind::Comment {
            return None;
        }
        if token.text == b"=" && !is_named_argument(&tokens, index) {
            return Some(token.span.end);
        }
    }
    None
}

/// The last whitespace run before `limit`, outside protected regions: the
/// fallback when no operator break exists.
fn whitespace_break_position(body: &[u8], tokens: &[Token], limit: usize) -> Option<usize> {
    let mut best = None;
    for pair in tokens.windows(2) {
        if pair[1].span.start > limit {
            break;
        }
        if pair[1].span.start > pair[0].span.end {
            best = Some(pair[1].span.start);
        }
    }
    let _ = body;
    best
}

fn is_assignment(token: &Token) -> bool {
    token.kind == TokenKind::Operator && (token.text == b"=" || token.text == b"=>")
}

/// True when this `=` names an argument or a specifier rather than assigning.
/// Breaking there would separate a keyword from its value (I5).
fn is_named_argument(tokens: &[Token], index: usize) -> bool {
    if tokens[index].depth == 0 {
        return false;
    }
    let Some(name) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return false;
    };
    if name.kind != TokenKind::Name {
        return false;
    }
    index
        .checked_sub(2)
        .and_then(|i| tokens.get(i))
        .is_some_and(|before| matches!(before.kind, TokenKind::LParen | TokenKind::Comma))
}

fn trim_start(mut s: &[u8]) -> &[u8] {
    while s.first().is_some_and(u8::is_ascii_whitespace) {
        s = &s[1..];
    }
    s
}

fn trim_end(mut s: &[u8]) -> &[u8] {
    while s.last().is_some_and(u8::is_ascii_whitespace) {
        s = &s[..s.len() - 1];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{wrap_body, wrap_position, ContinuationLayout, Decline};

    fn shown(lines: &[Vec<u8>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect()
    }

    #[test]
    fn a_break_lands_on_the_shallowest_loosest_operator() {
        // Both outer commas are shallower than anything inside `g(...)`; the
        // rightmost of the equally good candidates wins, and the nested call is
        // left whole.
        let body = b"call f(alpha, g(beta + gamma, delta), epsilon)";
        let position = wrap_position(body, 40).unwrap();
        assert_eq!(&body[position - 1..position], b",");
        assert_eq!(&body[..position], b"call f(alpha, g(beta + gamma, delta),");
    }

    #[test]
    fn a_declaration_head_is_not_split() {
        let body = b"real(dl), allocatable, intent(inout) :: values(:), weights(:)";
        let position = wrap_position(body, 45).unwrap();
        assert_eq!(&body[position - 2..position], b"::");
    }

    #[test]
    fn a_named_argument_is_never_mistaken_for_an_assignment() {
        let body = b"call sub(unit=lun, status='old', form='formatted', action='read')";
        let position = wrap_position(body, 30).unwrap();
        assert_eq!(&body[position - 1..position], b",");
    }

    #[test]
    fn wrapping_uses_the_layout_continuation_column_not_a_literal() {
        let body = b"total = alpha + beta + gamma + delta + epsilon + zeta + eta + theta";
        let narrow = wrap_body(
            body,
            ContinuationLayout {
                first_indent: 4,
                continuation: 8,
            },
            40,
        )
        .unwrap();
        for (index, line) in narrow.iter().enumerate() {
            let indent = if index == 0 { 4 } else { 8 };
            assert!(
                indent + line.len() <= 40,
                "line {index} overruns: {:?}",
                shown(&narrow)
            );
        }
        assert!(narrow.len() > 1);
        for line in &narrow[..narrow.len() - 1] {
            assert!(
                line.ends_with(b" &"),
                "missing marker in {:?}",
                shown(&narrow)
            );
        }
        assert!(!narrow.last().unwrap().ends_with(b"&"));
    }

    #[test]
    fn reflow_preserves_every_code_byte() {
        let body = b"result = first_term * second_term + third_term / fourth_term - fifth_term";
        let lines = wrap_body(
            body,
            ContinuationLayout {
                first_indent: 6,
                continuation: 10,
            },
            48,
        )
        .unwrap();
        let mut rejoined = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let text = if index + 1 < lines.len() {
                &line[..line.len() - 2]
            } else {
                &line[..]
            };
            if index > 0 {
                rejoined.push(b' ');
            }
            rejoined.extend_from_slice(text.strip_suffix(b" ").unwrap_or(text));
        }
        assert_eq!(rejoined, body);
    }

    #[test]
    fn unsafe_statements_are_declined_with_a_reason() {
        let layout = ContinuationLayout {
            first_indent: 0,
            continuation: 4,
        };
        assert_eq!(wrap_body(b"x = 1", layout, 80), Err(Decline::Fits));
        assert_eq!(
            wrap_body(b"call sub('unterminated", layout, 10),
            Err(Decline::ContinuedLiteral)
        );
        assert_eq!(
            wrap_body(b"call sub(3Habc, more_arguments_here)", layout, 10),
            Err(Decline::Hollerith)
        );
        assert_eq!(
            wrap_body(b"averyveryverylongsingleidentifiername", layout, 10),
            Err(Decline::NoSafeBreak)
        );
    }

    #[test]
    fn a_string_literal_is_never_split() {
        let body = b"write(*,*) 'a very long message that must not be split at all', value";
        let position = wrap_position(body, 40).unwrap();
        let literal_start = body.iter().position(|b| *b == b'\'').unwrap();
        assert!(
            position <= literal_start
                || position
                    > body[literal_start + 1..]
                        .iter()
                        .position(|b| *b == b'\'')
                        .unwrap()
                        + literal_start,
            "break at {position} falls inside the literal"
        );
    }
}
