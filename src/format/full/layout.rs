//! Full-mode layout and layout measurement.
//!
//! The layout engine remains the sole owner of output columns. This module
//! centralizes the two places full formatting asks it to do work: final layout
//! convergence, and the narrower column measurement the wrapper uses while it
//! is still choosing breaks.

use crate::{
    config::FormatConfig,
    error::FormatError,
    format::engine,
    transform::{document::Document, pipeline},
    FormatMeta,
};

/// Run the layout engine and the post-layout passes over normalized text.
///
/// Step 17 can change a line's width after the engine has chosen continuation
/// columns. When that happens, one more engine pass makes those columns agree
/// with the bytes that will be emitted. The second post-layout pass is a fixed
/// point of the first, so two rounds are sufficient.
pub(super) fn lay_out(
    document: &Document,
    config: &FormatConfig,
) -> Result<(Document, FormatMeta), FormatError> {
    let mut source = document.to_lf_bytes();
    let mut rounds = 2;
    loop {
        let laid_out = engine::format(&source, config)?;
        let mut output = Document::from_bytes(&laid_out.bytes);
        output.newline = document.newline;
        output.trailing_newline = document.trailing_newline;
        let widths_changed = pipeline::post_layout(&mut output, config)?;
        rounds -= 1;
        if !widths_changed || rounds == 0 {
            return Ok((output, laid_out.meta));
        }
        source = output.to_lf_bytes();
    }
}

/// Lay out one wrapper measurement and apply exactly the alignment passes that
/// can change the widths the wrapper observes.
///
/// Declaration alignment may grow or shrink a line. Trailing-comment alignment
/// only shrinks a gap, but measuring it here prevents the wrapper from budgeting
/// against whitespace that final emission will remove. Later post-layout passes
/// may change line structure, so they deliberately remain part of [`lay_out`]
/// rather than this one-to-one group measurement.
pub(super) fn measure(source: &[u8], config: &FormatConfig) -> Result<Document, FormatError> {
    let mut laid_out = Document::from_bytes(&engine::format(source, config)?.bytes);
    crate::transform::passes::layout_post::declaration_separator_alignment(&mut laid_out, config)?;
    crate::transform::passes::layout_post::trailing_comment_alignment(&mut laid_out, config)?;
    Ok(laid_out)
}
