//! Public full-formatting facade.
//!
//! Most modes run through the established full-mode driver.  The composed
//! `CanonicalizeAndIndent` mode is deliberately routed here instead: it runs
//! canonicalization first and then feeds those bytes to the indent-only engine,
//! without entering the driver's wrapping or post-layout presentation passes.

use super::{engine, planner, wrapping};
use crate::{
    analysis::{analyze_file, FileFacts, ProjectContext},
    config::{FormatConfig, FormatMode},
    error::FormatError,
    FormatMeta, FormatResult,
};

mod driver;

// Keep the existing public API while the driver itself stays an implementation
// detail. Repository-internal call sites do not use these helpers, but removing
// public exports is a separate compatibility decision from this module cleanup.
pub use driver::{reflow, reflow_with_context};

pub fn format_with_context(
    source: &[u8],
    project: &ProjectContext,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    let target_override = config.target_standard_override();
    let config = target_override.as_ref().unwrap_or(config);
    if config.mode != FormatMode::CanonicalizeAndIndent {
        return driver::format_with_context(source, project, config);
    }
    let local = analyze_file(source)?;
    canonicalize_and_indent_with_local(source, project, &local, config)
}

pub(crate) fn format_with_context_and_local(
    source: &[u8],
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    let target_override = config.target_standard_override();
    let config = target_override.as_ref().unwrap_or(config);
    if config.mode != FormatMode::CanonicalizeAndIndent {
        return driver::format_with_context_and_local(source, project, local, config);
    }
    canonicalize_and_indent_with_local(source, project, local, config)
}

pub(crate) fn format_to_with_context<W: std::io::Write>(
    source: &[u8],
    project: &ProjectContext,
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    let target_override = config.target_standard_override();
    let config = target_override.as_ref().unwrap_or(config);
    if config.mode != FormatMode::CanonicalizeAndIndent {
        return driver::format_to_with_context(source, project, config, out);
    }
    let local = analyze_file(source)?;
    let canonicalized = canonicalize_with_local(source, project, &local, config)?;
    let mut indent = config.clone();
    indent.mode = FormatMode::IndentOnly;
    engine::format_to(&canonicalized.bytes, &indent, out)
}

fn canonicalize_and_indent_with_local(
    source: &[u8],
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    let canonicalized = canonicalize_with_local(source, project, local, config)?;
    let mut indent = config.clone();
    indent.mode = FormatMode::IndentOnly;
    engine::format(&canonicalized.bytes, &indent)
}

fn canonicalize_with_local(
    source: &[u8],
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    let mut canonical = config.clone();
    canonical.mode = FormatMode::CanonicalizeOnly;
    driver::format_with_context_and_local(source, project, local, &canonical)
}
