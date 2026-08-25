//! Break-point selection and reflow.
//!
//! The division of labour matters more than any single heuristic:
//!
//! * the **wrapper** decides *where* a statement breaks;
//! * the **layout plan** decides *where the next physical line begins*.
//!
//! A fixed `indent + 4` continuation would silently disagree the moment anyone
//! passed a different `-k`. The continuation column always comes from
//! [`ContinuationLayout`], which the planner fills in.
//!
//! The ranking of candidate break points encodes real taste about where a human
//! would split an expression.

use crate::format::continuation::ParenAlignmentState;
use crate::source::{
    regions::LexState,
    syntax::{line_start_syntax, LineStartSyntax},
    tokens::{tokenize, Token, TokenKind},
};

/// Operator tiers, loosest binding first.  A break on a looser operator is
/// preferred, because it separates larger pieces of meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BreakTier {
    /// The `::` of a nested type-spec, as in `allocate(T :: obj)` or
    /// `[integer :: 1, 2]`. Looser than a comma so the type-spec/object-list
    /// seam is preferred over a break further into the object list.
    TypeSpec,
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
/// wrap must be explainable, so diagnostics can separate "unwrappable by
/// design" from "wrapper bug".
///
/// Note that a lone over-long string literal is *not* always a decline: see
/// `literal_wrap_split`, which relaxes "no token is split" (I5) for exactly
/// that one case, at a whitespace boundary inside the literal's content.
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
    wrap_body_with_alignment(body, layout, line_length, false)
}

/// Variant used by full-mode reflow when parenthesis alignment is active.
/// The alignment state is advanced after each chosen break, so nested calls
/// can change the target for the following physical line.
pub fn wrap_body_with_alignment(
    body: &[u8],
    layout: ContinuationLayout,
    line_length: usize,
    align_paren: bool,
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
    let mut rest: Vec<u8> = body.to_vec();
    let mut current = layout.first_indent;
    let mut first_break = true;
    let mut paren_state = ParenAlignmentState::default();
    while current + rest.len() > line_length {
        // Two columns are reserved for the ` &` this line will end with.
        let limit = line_length.saturating_sub(current + 2);
        let mut position = None;
        if first_break {
            if let Some(candidate) = assignment_wrap_position(&rest, limit)
                .filter(|candidate| !continuation_head_is_line_start_syntax(&rest, *candidate))
            {
                let remainder = trim_start(&rest[candidate..]);
                let mut trial_state = paren_state.clone();
                let next = next_continuation(
                    &mut trial_state,
                    &rest[..candidate],
                    current,
                    layout.continuation,
                    align_paren,
                );
                if next + remainder.len() <= line_length {
                    position = Some(candidate);
                }
            }
        }
        let generic = position.or_else(|| wrap_position(&rest, limit));
        // A generic break that barely fills the line is a poor choice when a
        // literal split would use the space better — most often a short call
        // head (`write(*,'(A)') `) ahead of one over-long message literal,
        // where the only generic candidate is a comma buried in the head.
        let minimum_fill = (limit as f64 * crate::transform::vocab::MINIMUM_BREAK_FILL) as usize;
        let generic_fills = generic.is_some_and(|position| position >= minimum_fill);
        let (line_text, new_rest) = if generic_fills {
            let position = generic.expect("generic_fills implies Some");
            (
                trim_end(&rest[..position]).to_vec(),
                trim_start(&rest[position..]).to_vec(),
            )
        } else if let Some(split) = literal_wrap_split(&rest, limit) {
            split
        } else if let Some(position) = generic {
            (
                trim_end(&rest[..position]).to_vec(),
                trim_start(&rest[position..]).to_vec(),
            )
        } else {
            return Err(Decline::NoSafeBreak);
        };
        let mut line = line_text;
        line.extend_from_slice(b" &");
        out.push(line);
        current = next_continuation(
            &mut paren_state,
            out.last().expect("wrapped line was just pushed"),
            current,
            layout.continuation,
            align_paren,
        );
        rest = new_rest;
        first_break = false;
    }
    out.push(rest);
    Ok(out)
}

/// Split an over-long string literal into two `//`-concatenated pieces at a
/// whitespace boundary inside its content, when doing so uses the line
/// better than the best ordinary break.  This is what stands between a lone
/// over-long literal (or a short head followed by one) and
/// [`Decline::NoSafeBreak`]: it deliberately relaxes "no token is split"
/// (I5) for exactly this one case.
///
/// `rest` up to the literal's opening quote is kept as an unsplittable
/// prefix — the caller has already established nothing before it is a
/// better break. Returns `(first_line, new_rest)` rather than an offset into
/// `rest`, because the split grows the text by the inserted quote, space
/// and `//` bytes and so cannot be expressed as a plain slice of the
/// original.
///
/// The inserted `//` is spaced on both sides, matching how the operator-
/// spacing normalization pass would space a genuine one on the next run —
/// splitting it unspaced would make the very next pass respace it and,
/// with a whole extra column consumed, potentially re-wrap it differently,
/// breaking I1.
fn literal_wrap_split(rest: &[u8], limit: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    let tokens = tokenize(rest, &mut LexState::default());
    let token = tokens
        .iter()
        .find(|token| token.kind == TokenKind::String && token.span.end > limit)?;
    if token.span.start >= limit {
        return None;
    }
    let delim = *token.text.first()?;
    if token.text.len() < 2 || *token.text.last()? != delim {
        return None;
    }
    let content = &token.text[1..token.text.len() - 1];

    // Room left, after the kept prefix, for the opening quote, the content
    // kept on this line, the closing quote, and ` //` joining to the next
    // line.
    let max_split = limit
        .saturating_sub(token.span.start + 5)
        .min(content.len());

    // A doubled delimiter (`''`) is one escaped character; splitting between
    // its two bytes would turn it into two independent, unescaped quotes.
    let mut unsafe_after = vec![false; content.len() + 1];
    let mut i = 0;
    while i < content.len() {
        if content[i] == delim {
            unsafe_after[i + 1] = true;
            i += 2;
        } else {
            i += 1;
        }
    }

    let split = (1..=max_split)
        .rev()
        .find(|&position| content[position - 1] == b' ' && !unsafe_after[position])?;

    let mut line = rest[..token.span.start].to_vec();
    line.push(delim);
    line.extend_from_slice(&content[..split]);
    line.push(delim);
    line.extend_from_slice(b" //");

    let mut new_rest = Vec::with_capacity(content.len() - split + 2 + rest.len() - token.span.end);
    new_rest.push(delim);
    new_rest.extend_from_slice(&content[split..]);
    new_rest.push(delim);
    new_rest.extend_from_slice(&rest[token.span.end..]);

    Some((line, new_rest))
}

fn next_continuation(
    state: &mut ParenAlignmentState,
    line: &[u8],
    target: usize,
    fallback: usize,
    align_paren: bool,
) -> usize {
    if !align_paren {
        return fallback;
    }
    state.scan(line, target);
    state.current().unwrap_or(fallback)
}

/// The best break position at or before `limit`, as an offset into `body`.
///
/// A statement's head — a declaration's attribute list up to `::`, or an
/// assignment's left-hand side up to `=` — is kept intact whenever the boundary
/// at its end is in reach, because that boundary is the one a human would use.
pub fn wrap_position(body: &[u8], limit: usize) -> Option<usize> {
    let tokens = tokenize(body, &mut LexState::default());
    let limit = scan_limit(&tokens, limit);
    let safe = |position: &usize| !continuation_head_is_line_start_syntax(body, *position);
    let head_end = statement_head_end(&tokens, limit);
    if head_end > 0 {
        return operator_break_position(body, &tokens, head_end, limit)
            .or_else(|| Some(head_end).filter(safe));
    }
    operator_break_position(body, &tokens, 0, limit)
        .or_else(|| assignment_break_position(&tokens, limit).filter(safe))
        .or_else(|| whitespace_break_position(body, &tokens, limit).filter(safe))
}

/// Whether breaking `body` at `position` would open the continuation with
/// syntax that only means something at the start of a physical line.
///
/// A stray `&` is the one that bites. Written inside a statement it is a byte
/// the formatter carries through; first on a continuation line it is the
/// optional leading marker, and the next pass consumes it. `program bf=&,(...`
/// wrapped to `program bf = &` / `&, (...`, and the run after that spelled the
/// second line `   , (...` -- an I1 break that *deletes* a byte, which is worse
/// than the ones that only move blanks around.
///
/// The test is [`line_start_syntax`], the same one the continuation pass reads,
/// so the wrapper cannot come to disagree with the reader about what it has
/// just written. `Blank` is not a refusal: a break that leaves nothing after it
/// is impossible here, and refusing on it would only ever be a false positive.
///
/// This is [`continuation_head_would_gain_relational_space`] one level up --
/// same reason, that the next pass would rewrite what this break produced, and
/// the same remedy, which is to break somewhere else.
///
/// [`wrap_position`]'s other caller reflows a directive, where each physical
/// line opens with `!$omp ` rather than at column zero. The test still holds
/// there, and for the same reason: a `&` right after the sentinel is that
/// stream's own continuation marker, which is the doubled-marker case in
/// `tests/line_start_promotion.rs`.
///
/// Refusing every candidate ends in [`Decline::NoSafeBreak`] and an over-long
/// line. That is the right way to be wrong here: a line nobody wrapped is a
/// fixed point, and a line wrapped onto a marker loses a byte.
fn continuation_head_is_line_start_syntax(body: &[u8], position: usize) -> bool {
    !matches!(
        line_start_syntax(&body[position..]),
        LineStartSyntax::Ordinary | LineStartSyntax::Blank
    )
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
fn operator_break_position(
    body: &[u8],
    tokens: &[Token],
    start: usize,
    limit: usize,
) -> Option<usize> {
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
        if continuation_head_would_gain_relational_space(tokens, index) {
            continue;
        }
        // Rejected here rather than around the result, so the search falls
        // through to the next-best candidate instead of giving up on the
        // whole tier.
        if continuation_head_is_line_start_syntax(body, position) {
            continue;
        }
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

/// A break can turn the next token into the first token of a continuation.
/// Single `<` and `>` tokens are intentionally not spaced while they touch
/// another angle operator, but at the start of the new line that left-hand
/// neighbour is gone. Refuse the one boundary that would expose a lone angle
/// operator directly against the following token, because the next full-mode
/// pass would then insert a space and violate I1.
fn continuation_head_would_gain_relational_space(tokens: &[Token], index: usize) -> bool {
    let Some(operator) = tokens.get(index + 1) else {
        return false;
    };
    if !matches!(operator.text, b"<" | b">") {
        return false;
    }
    let Some(following) = tokens.get(index + 2) else {
        return false;
    };
    following.span.start == operator.span.end
        && !matches!(following.text.first(), Some(b'<' | b'>'))
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
            // The statement-level `::` (a declaration's attribute/entity
            // seam) is handled separately by `statement_head_end`, which
            // runs before this search and is never revisited here. Nested
            // type-spec separators are useful breaks, but Fortran 2023 also
            // uses `::` inside compact `@` multiple-subscript triplets, where
            // splitting would break the designator's punctuation policy.
            b"::" => {
                (!is_multiple_subscript_double_colon(tokens, index)).then_some(BreakTier::TypeSpec)
            }
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

/// True when `tokens[index]` is the two adjacent triplet colons of a Fortran
/// 2023 multiple-subscript item. At one delimiter depth, a comma ends the
/// current subscript item; tokens at deeper depths belong to nested expressions.
fn is_multiple_subscript_double_colon(tokens: &[Token], index: usize) -> bool {
    let depth = tokens[index].depth;
    for token in tokens[..index].iter().rev() {
        if token.depth > depth {
            continue;
        }
        if token.depth < depth || token.kind == TokenKind::Comma {
            return false;
        }
        if token.kind == TokenKind::Operator && token.text == b"@" {
            return true;
        }
    }
    false
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
    use super::{wrap_body, wrap_body_with_alignment, wrap_position, ContinuationLayout, Decline};

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
    fn multiple_subscript_double_colons_are_not_type_spec_breaks() {
        for body in [
            b"call f(@::stride, another_argument)".as_slice(),
            b"call f(@lo::stride, another_argument)".as_slice(),
        ] {
            let position = wrap_position(body, body.len()).unwrap();
            assert_eq!(&body[position - 1..position], b",");
        }

        // Keep the existing nested type-spec preference for real type specs.
        let body = b"allocate(widget :: object, stat=i)";
        let position = wrap_position(body, body.len()).unwrap();
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

    #[test]
    fn an_over_long_literal_is_split_at_a_space_boundary_via_concatenation() {
        let body = b"write(*,'(A)') 'Turbine Number  Output Turbine Number      X          Y          Z   OpenFAST Time Step  OpenFAST SubCycles  OpenFAST Input File'";
        let layout = ContinuationLayout {
            first_indent: 3,
            continuation: 6,
        };
        let lines = wrap_body(body, layout, 100).unwrap();
        assert!(lines.len() > 1, "expected a split: {:?}", shown(&lines));
        for (index, line) in lines.iter().enumerate() {
            let indent = if index == 0 {
                layout.first_indent
            } else {
                layout.continuation
            };
            assert!(
                indent + line.len() <= 100,
                "line {index} overruns: {:?}",
                shown(&lines)
            );
        }
        assert!(
            lines[0].ends_with(b"' // &"),
            "not canonically spaced: {:?}",
            shown(&lines)
        );
        assert!(
            lines[1].starts_with(b"'"),
            "second half should start directly at its own quote: {:?}",
            shown(&lines)
        );
        // The literal's content, rejoined across the split, is byte-identical
        // to the original — the split only ever inserts delimiters and the
        // concatenation operator around an existing space.
        let head = lines[0].strip_suffix(b" // &").expect("checked above");
        let open = head.iter().position(|b| *b == b'\'').unwrap();
        let close = head.len() - 1;
        let mut content = Vec::new();
        content.extend_from_slice(&head[open + 1..close]);
        content.extend_from_slice(&lines[1][1..lines[1].len() - 1]);
        assert!(String::from_utf8_lossy(&content).contains("Turbine Number  Output Turbine Number"));
        assert!(String::from_utf8_lossy(&content).ends_with("OpenFAST Input File"));
    }

    #[test]
    fn literal_split_never_cuts_a_doubled_quote() {
        // The doubled `''` sits exactly where a naive space-search might
        // otherwise land a split; the escape must survive intact regardless.
        let body = b"call sub('a very long line that keeps going and going and going don''t you know it does')";
        let layout = ContinuationLayout {
            first_indent: 0,
            continuation: 4,
        };
        let lines = wrap_body(body, layout, 40).unwrap();
        let joined = shown(&lines).join("");
        assert!(
            !joined.contains("don' // 't"),
            "split inside the doubled quote: {joined}"
        );
        assert!(joined.contains("don''t"), "escape lost: {joined}");
    }

    #[test]
    fn literal_split_is_preferred_over_a_poorly_filling_generic_break() {
        // The only ordinary break candidate here is the comma inside
        // `write(*,...)`, which barely fills the line; splitting the message
        // literal instead keeps the call head whole.
        let body = b"write(*,'(A)') 'Turbine Number  Output Turbine Number      X          Y          Z   OpenFAST Time Step  OpenFAST SubCycles  OpenFAST Input File'";
        let layout = ContinuationLayout {
            first_indent: 3,
            continuation: 6,
        };
        let lines = wrap_body(body, layout, 100).unwrap();
        assert!(
            shown(&lines)[0].starts_with("write(*, '(A)') '")
                || shown(&lines)[0].starts_with("write(*,'(A)') '"),
            "call head was needlessly split: {:?}",
            shown(&lines)
        );
    }

    #[test]
    fn a_single_unbreakable_word_literal_still_declines() {
        // No space exists to split at, so this remains an honest decline
        // rather than an arbitrary mid-word cut.
        let layout = ContinuationLayout {
            first_indent: 0,
            continuation: 4,
        };
        assert_eq!(
            wrap_body(
                b"call sub('averyveryverylongsinglewordwithnospaceatallanywhereinit')",
                layout,
                20
            ),
            Err(Decline::NoSafeBreak)
        );
    }

    #[test]
    fn aligned_wrapping_uses_the_nested_parenthesis_target_per_break() {
        let body = b"call outer(first, inner(alpha, beta, gamma), last_value)";
        let lines = wrap_body_with_alignment(
            body,
            ContinuationLayout {
                first_indent: 2,
                continuation: 6,
            },
            32,
            true,
        )
        .unwrap();
        assert!(lines.len() > 1);
        for line in &lines[..lines.len() - 1] {
            assert!(line.ends_with(b" &"));
        }
    }
}
