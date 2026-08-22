//! Mapping a token of an assembled logical statement back to the physical
//! bytes it came from.
//!
//! A statement is reassembled from continuation pieces before any pass looks
//! at it, so a token's span is an offset into that reassembly, not into a
//! line. Both case passes need the same translation and the same rule for
//! writing a replacement back across a token the author split, so it lives
//! here rather than in either of them.

use crate::source::{tokens::Token, LogicalGroup, LogicalStatement};
use std::ops::Range;

/// The physical `(line, byte range)` pieces one token occupies, in order.
pub(super) fn source_spans(
    group: &LogicalGroup,
    statement: &LogicalStatement,
    token: &Token<'_>,
) -> Vec<(usize, Range<usize>)> {
    let start = statement.offset + token.span.start;
    let end = statement.offset + token.span.end;
    let mut spans = Vec::new();
    for piece in &group.pieces {
        let lo = start.max(piece.text.start);
        let hi = end.min(piece.text.end);
        if lo >= hi {
            continue;
        }
        let origin = piece.bytes.start as usize + (lo - piece.text.start);
        spans.push((piece.line, origin..origin + (hi - lo)));
    }
    spans
}

/// Distribute a canonical spelling across the spans its token occupies.
///
/// Every spelling the case passes produce names the same identifier, so the
/// replacement is the same length as the token and can be cut at the same
/// offsets the continuation cut the token at. A replacement of a different
/// length has no such correspondence and is refused; none is produced today.
/// Callers depend on that refusal: writing a longer or shorter spelling piece
/// by piece would corrupt the spans rather than fail.
pub(super) fn spread_replacement<'a>(
    spans: &'a [(usize, Range<usize>)],
    token: &Token<'_>,
    replacement: &'a [u8],
) -> Option<impl Iterator<Item = (usize, Range<usize>, &'a [u8])> + 'a> {
    (replacement.len() == token.text.len()).then(|| {
        let mut taken = 0;
        spans.iter().map(move |(line, span)| {
            let piece = &replacement[taken..taken + span.len()];
            taken += span.len();
            (*line, span.clone(), piece)
        })
    })
}
