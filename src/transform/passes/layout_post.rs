//! Steps 17-20: passes that run **after** the layout engine has placed every
//! line.
//!
//! The contract for this file is short and absolute: **nothing here may make a
//! line longer.** Wrapping has already happened, so a pass that pads would
//! invalidate a decision it cannot revisit.  Declaration alignment therefore
//! compresses and never pads — that is not an accident of the reference
//! implementation, it is why the pass is allowed to run here at all.

use crate::{config::FormatConfig, error::FormatError, transform::document::Document};

/// Step 17: compress the whitespace before a declaration's `::` so a block of
/// declarations lines up.  Compresses only.
///
/// Port target: `normalize_declaration_separator_alignment`.
pub fn declaration_separator_alignment(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = (document, config);
    Ok(())
}

/// Step 18: blank-line policy around program units and `CONTAINS`.
///
/// Port target: `normalize_program_unit_spacing`.
pub fn program_unit_spacing(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = (document, config);
    Ok(())
}

/// Step 19: cap runs of blank lines.
///
/// Port target: `limit_blank_lines`.
pub fn limit_blank_lines(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = (document, config);
    Ok(())
}

/// Step 20: trailing horizontal whitespace.
///
/// This one is implemented: it is unconditional, cannot lengthen a line, and
/// matches what indent-only mode already does, so full and indent-only output
/// agree on it (I2).
pub fn output_whitespace(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
    for line in &mut document.lines {
        let mut end = line.len();
        while end > 0 && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
            end -= 1;
        }
        line.truncate(end);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::output_whitespace;
    use crate::{config::FormatConfig, transform::document::Document};

    #[test]
    fn trailing_horizontal_whitespace_is_removed_from_every_line() {
        let mut document = Document::from_bytes(b"x = 1   \n\t\n  y = 2\t \n");
        output_whitespace(&mut document, &FormatConfig::default()).unwrap();
        assert_eq!(document.to_bytes(), b"x = 1\n\n  y = 2\n");
    }
}
