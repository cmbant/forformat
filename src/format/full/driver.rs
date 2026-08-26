//! Full-mode orchestration.
//!
//! ```text
//! bytes ─► document ─► normalization (steps 1-15)
//!                   ─► reflow (step 16)
//!                   ─► layout convergence
//!                   ─► post-layout passes (steps 17-20)
//!                   ─► bytes
//! ```
//!
//! This module deliberately owns only the pipeline order. Layout convergence
//! lives in [`super::layout`], and all wrapping mechanics and convergence live
//! in [`super::reflow`]. Keeping those responsibilities separate makes the
//! invariant-bearing loops readable without turning the driver into their
//! permanent home.

use super::{engine, layout, reflow};
use crate::{
    analysis::{analyze_file, ProjectContext},
    config::FormatConfig,
    error::FormatError,
    transform::{document::Document, pipeline},
    FormatMeta, FormatResult,
};

#[cfg(test)]
use super::reflow::{copy_group_without_final_comment, fixed_point_progress, FixedPointProgress};

/// Format one buffer with project context.
pub fn format_with_context(
    source: &[u8],
    project: &ProjectContext,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    if !config.mode.normalizes() {
        return engine::format(source, config);
    }
    let local = analyze_file(source)?;
    format_with_context_and_local(source, project, &local, config)
}

/// Format one buffer using declaration facts already extracted from `source`.
///
/// The file workflow analyzes project members before formatting so it can both
/// build the project tables and retain each target's local precedence facts.
/// Reusing those facts here avoids parsing every full-mode target a second time.
pub(crate) fn format_with_context_and_local(
    source: &[u8],
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    if !config.mode.normalizes() {
        return engine::format(source, config);
    }
    let (document, meta) = format_document_with_context_and_local(source, project, local, config)?;
    Ok(FormatResult {
        bytes: document.to_bytes(),
        meta,
    })
}

/// Format one buffer with project context directly into a writer.
pub(crate) fn format_to_with_context<W: std::io::Write>(
    source: &[u8],
    project: &ProjectContext,
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    if !config.mode.normalizes() {
        return engine::format_to(source, config, out);
    }
    let local = analyze_file(source)?;
    let (document, meta) = format_document_with_context_and_local(source, project, &local, config)?;
    document.write_to(out)?;
    Ok(meta)
}

fn format_document_with_context_and_local(
    source: &[u8],
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<(Document, FormatMeta), FormatError> {
    let mut document = Document::from_bytes(source);
    // `--start-indent=auto` has to be answered while the authored indentation
    // is still there. Every stage below then reads one fixed base, so the
    // wrapper measures the columns the engine will really emit.
    let resolved = resolve_start_indent(&document, config)?;
    let config = resolved.as_ref().unwrap_or(config);
    pipeline::normalize(&mut document, project, local, config)?;

    if !config.mode.lays_out() {
        // The no-layout modes skip structural layout, but trailing whitespace
        // is invisible in every mode and is still normalized.
        crate::transform::passes::layout_post::trim_trailing_horizontal(&mut document);
        return Ok((document, FormatMeta::default()));
    }

    if config.wrap.enabled {
        return reflow::format_wrapped(&mut document, project, local, config);
    }

    layout::lay_out(&document, config)
}

/// Freeze `--start-indent=auto` into a plain `start_indent`, or `None` when the
/// option is off and the caller's own config already answers every question.
fn resolve_start_indent(
    document: &Document,
    config: &FormatConfig,
) -> Result<Option<FormatConfig>, FormatError> {
    if !config.auto_start_indent {
        return Ok(None);
    }
    let analysis = document.analyze()?;
    let mut resolved = config.clone();
    resolved.start_indent = crate::format::planner::resolve_auto_start_indent(
        &analysis.buffer,
        &analysis.groups,
        config,
    );
    resolved.auto_start_indent = false;
    Ok(Some(resolved))
}

#[cfg(test)]
mod tests;
