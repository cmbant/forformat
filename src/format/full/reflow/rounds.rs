//! Fixed-point convergence for wrapping and rewrap settlement.

use super::{reflow_and_lay_out, ReflowResult, ReflowScope};
use crate::{
    analysis::{FileFacts, ProjectContext},
    config::FormatConfig,
    error::FormatError,
    transform::{document::Document, pipeline},
    FormatMeta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::format::full) enum FixedPointProgress {
    New,
    Stable,
    Cycle,
}

/// Classify one deterministic transition result against the states already seen.
///
/// Repeating the immediately preceding state proves a fixed point. Repeating an
/// older state instead closes a cycle, so another iteration would only revisit
/// states we have already processed.
pub(in crate::format::full) fn fixed_point_progress<T: PartialEq>(
    history: &[T],
    candidate: &T,
) -> FixedPointProgress {
    if history.last().is_some_and(|previous| previous == candidate) {
        FixedPointProgress::Stable
    } else if history.contains(candidate) {
        FixedPointProgress::Cycle
    } else {
        FixedPointProgress::New
    }
}

fn convergence_cycle(stage: &str) -> FormatError {
    FormatError::Unsupported(format!(
        "{stage} entered a cycle before reaching a fixed point"
    ))
}

/// Finish `--rewrap` by feeding only wrapper-generated continuation seams back
/// through the existing reflow/layout path until preparation reaches a fixed
/// point.
pub(super) fn settle_rewrap(
    mut output: Document,
    mut meta: FormatMeta,
    settlement_input: Vec<u8>,
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<(Document, FormatMeta), FormatError> {
    let mut settlement_inputs = vec![settlement_input];
    loop {
        let source = output.to_bytes();
        let mut candidate = Document::from_bytes(&source);
        pipeline::prepare_rewrap_settlement(&mut candidate, project, local, config)?;
        let prepared = candidate.to_bytes();

        // No wrapper-generated continuation was joined, so there is no new
        // normalization evidence for another internal invocation.
        if prepared == source {
            return Ok((output, meta));
        }

        match fixed_point_progress(&settlement_inputs, &prepared) {
            FixedPointProgress::Stable => return Ok((output, meta)),
            FixedPointProgress::Cycle => return Err(convergence_cycle("rewrap settlement")),
            FixedPointProgress::New => settlement_inputs.push(prepared),
        }
        (output, meta) = reflow_and_lay_out(&mut candidate, project, local, config)?;
    }
}

/// One wrapper round's visible state. `needs_reflow` is deliberately not stored:
/// those flags only grow, and the history is cleared whenever one grows. Within
/// one history segment the flags are therefore fixed, so repeating these bytes
/// and group spans repeats the complete deterministic transition state.
#[derive(PartialEq, Eq)]
struct ReflowRoundState {
    lines: Vec<Vec<u8>>,
    spans: Vec<std::ops::Range<usize>>,
}

impl ReflowScope<'_> {
    /// Run the rounds and return the document's new lines with the declines to
    /// report.
    ///
    /// *Whether* a statement overruns is asked afresh of every round's layout,
    /// but the answer only ever accumulates. Both halves are load-bearing.
    ///
    /// Asking every round finds a statement that fits until some *other* group
    /// is wrapped and declaration alignment widens it. Never retracting stops a
    /// wrapped group from measuring its own new short lines, unwrapping itself,
    /// and oscillating. Once discovery stops, the transition is deterministic:
    /// an adjacent repeat is the fixed point and any older repeat is a cycle.
    pub(super) fn reflow(&self) -> Result<(Vec<Vec<u8>>, ReflowResult), FormatError> {
        let analysis = self.analysis();
        let config = self.config();
        let mut spans: Vec<std::ops::Range<usize>> = analysis
            .groups
            .iter()
            .map(|group| group.lines.clone())
            .collect();
        let mut needs_reflow = vec![false; analysis.groups.len()];
        let mut measured = self.document.to_lf_bytes();
        let mut round_history: Vec<ReflowRoundState> = Vec::new();

        loop {
            let laid_out = super::super::layout::measure(&measured, config)?;
            let mut discovered = false;
            for (ordinal, span) in spans.iter().enumerate() {
                if !self.wrappable[ordinal] || needs_reflow[ordinal] {
                    continue;
                }
                if span.clone().any(|line| {
                    laid_out.lines.get(line).map_or(0, Vec::len) > config.wrap.line_length
                }) {
                    needs_reflow[ordinal] = true;
                    discovered = true;
                }
            }

            let mut lines: Vec<Vec<u8>> = Vec::with_capacity(self.document.lines.len());
            let mut declined = Vec::new();
            let mut next_spans = Vec::with_capacity(analysis.groups.len());
            for (ordinal, ((group, plan), span)) in analysis
                .groups
                .iter()
                .zip(&self.plans)
                .zip(&spans)
                .enumerate()
            {
                let start = lines.len();
                let (out, decline) =
                    self.emit_group(group, plan, span, &laid_out, needs_reflow[ordinal]);
                lines.extend(out);
                if let Some(decline) = decline {
                    declined.push(decline);
                }
                next_spans.push(start..lines.len());
            }

            // States collected before a new sticky wrap decision are no longer
            // comparable: the transition function has gained another fixed
            // `needs_reflow` input.
            if discovered {
                round_history.clear();
            }
            let state = ReflowRoundState {
                lines: lines.clone(),
                spans: next_spans.clone(),
            };
            match fixed_point_progress(&round_history, &state) {
                FixedPointProgress::Stable => return Ok((lines, declined)),
                FixedPointProgress::Cycle => return Err(convergence_cycle("wrapping")),
                FixedPointProgress::New => round_history.push(state),
            }

            let mut probe = self.document.clone();
            probe.set_lines(lines);
            measured = probe.to_lf_bytes();
            spans = next_spans;
        }
    }
}
