//! Fortran 2023 multiple-subscript (`@`) tracking, shared by the two passes
//! that need it.
//!
//! A `@` designator owns the colons that belong to *its* triplet — `@lo:hi:step`,
//! `@::step` — while ordinary section-subscript colons in sibling or nested
//! items do not. Telling those apart is a depth question, and it has to survive
//! authored continuation lines, so both delimiter normalization (which decides
//! spacing) and post-layout declaration alignment (which must not mistake a
//! triplet `::` for a declaration separator) walk the same state machine.
//!
//! They used to walk two byte-identical copies of it, differing only in which
//! of the results each bothered to return. One copy drifting from the other
//! would mean the two passes disagreed about what a `::` *is*, so there is one
//! copy and it returns everything.

use super::{Token, TokenKind};

/// What one physical line's tokens did to the multiple-subscript state.
#[derive(Default)]
pub(crate) struct MultipleSubscriptScan {
    /// Token indices of the `@` prefixes seen on this line.
    pub(crate) prefixes: Vec<usize>,
    /// Token indices of the `:`/`::` that belong to an active `@` item.
    pub(crate) triplet_colons: Vec<usize>,
    /// Absolute delimiter depths with an `@` item still open at end of line.
    pub(crate) active_depths: Vec<usize>,
    /// Absolute delimiter depth at end of line.
    pub(crate) end_depth: usize,
}

/// The state a continuation line resumes from.
#[derive(Debug, Default, Clone)]
pub(crate) struct MultipleSubscriptState {
    pub(crate) open_depth: usize,
    pub(crate) active_depths: Vec<usize>,
}

impl MultipleSubscriptState {
    /// True while this line is in the middle of something a fresh scan would
    /// misread: an unclosed delimiter group, or an open `@` item.
    pub(crate) fn carrying(&self) -> bool {
        self.open_depth > 0 || !self.active_depths.is_empty()
    }
}

/// Scan one physical line using *absolute* delimiter depth, carrying active `@`
/// items from earlier continuation lines.
///
/// Absolute depth rather than a token-local counter is what makes a
/// continuation line that closes groups opened on an earlier physical line come
/// out right. A comma at an active depth ends that subscript item; closing a
/// group discards items opened deeper. Nested calls and sections therefore do
/// not leak their ordinary colons into the outer `@` triplet policy.
pub(crate) fn scan_multiple_subscripts(
    tokens: &[Token<'_>],
    open_depth: usize,
    continued_multiple_subscripts: &[usize],
) -> MultipleSubscriptScan {
    let mut scan = MultipleSubscriptScan {
        active_depths: continued_multiple_subscripts.to_vec(),
        ..MultipleSubscriptScan::default()
    };
    let mut depth = open_depth;

    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket => {
                depth += 1;
            }
            TokenKind::RParen | TokenKind::RBracket => {
                depth = depth.saturating_sub(1);
                scan.active_depths.retain(|active| *active <= depth);
            }
            TokenKind::Comma => {
                scan.active_depths.retain(|active| *active < depth);
            }
            TokenKind::Operator if token.text == b"@" => {
                scan.active_depths.retain(|active| *active != depth);
                scan.active_depths.push(depth);
                scan.prefixes.push(index);
            }
            TokenKind::Operator
                if matches!(token.text, b":" | b"::") && scan.active_depths.contains(&depth) =>
            {
                scan.triplet_colons.push(index);
            }
            _ => {}
        }
    }

    scan.end_depth = depth;
    scan
}
