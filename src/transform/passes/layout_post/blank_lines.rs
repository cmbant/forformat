use crate::{
    config::FormatConfig,
    error::FormatError,
    transform::{
        document::Document,
        passes::structure::{cpp_line_continues, is_preprocessor_line},
    },
};

/// Step 19: cap runs of blank lines without crossing preprocessor continuations.
pub fn limit_blank_lines(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let Some(max_blank_lines) = config.style.max_blank_lines else {
        return Ok(());
    };
    let mut limited = Vec::with_capacity(document.lines.len());
    let mut blank_count = 0;
    let mut cpp_continuation = false;
    for line in &document.lines {
        let cpp_line = cpp_continuation || is_preprocessor_line(line);
        if cpp_line {
            limited.push(line.clone());
            cpp_continuation = cpp_line_continues(line);
            blank_count = 0;
            continue;
        }
        cpp_continuation = false;
        if line.iter().any(|byte| !byte.is_ascii_whitespace()) {
            blank_count = 0;
            limited.push(line.clone());
        } else if blank_count < max_blank_lines {
            blank_count += 1;
            limited.push(line.clone());
        }
    }
    document.set_lines(limited);
    Ok(())
}
