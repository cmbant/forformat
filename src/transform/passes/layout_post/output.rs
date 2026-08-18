use crate::{config::FormatConfig, error::FormatError, transform::document::Document};

/// Step 20: remove trailing horizontal whitespace and normalize EOF newline.
pub fn output_whitespace(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
    let had_input = document.trailing_newline
        || document.lines.len() > 1
        || document.lines.first().is_some_and(|line| !line.is_empty());
    for line in &mut document.lines {
        let mut end = line.len();
        while end > 0 && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
            end -= 1;
        }
        line.truncate(end);
    }
    while document.lines.len() > 1 && document.lines.last().is_some_and(Vec::is_empty) {
        document.lines.pop();
    }
    document.trailing_newline = had_input;
    Ok(())
}
