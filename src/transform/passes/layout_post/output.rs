use crate::{config::FormatConfig, error::FormatError, transform::document::Document};

/// Step 20: remove trailing horizontal whitespace, truncate overlong separator
/// comments when wrapping is enabled, and normalize the EOF newline.
pub fn output_whitespace(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let had_input = document.trailing_newline
        || document.lines.len() > 1
        || document.lines.first().is_some_and(|line| !line.is_empty());
    for line in &mut document.lines {
        let mut end = line.len();
        while end > 0 && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
            end -= 1;
        }
        line.truncate(end);
        if config.wrap.enabled {
            truncate_comment_separator(line, config.wrap.line_length);
        }
    }
    while document.lines.len() > 1 && document.lines.last().is_some_and(Vec::is_empty) {
        document.lines.pop();
    }
    document.trailing_newline = had_input;
    Ok(())
}

/// A full-line comment whose body is one repeated non-whitespace byte is a
/// visual separator, not prose to reflow. Once layout has chosen its final
/// column, shorten only the repeated run so the emitted line fits the wrap
/// budget. Ordinary comments and inline comments are deliberately untouched.
fn truncate_comment_separator(line: &mut Vec<u8>, line_length: usize) {
    if line.len() <= line_length {
        return;
    }
    let Some(comment_start) = line.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return;
    };
    if line[comment_start] != b'!' {
        return;
    }
    let Some(relative_start) = line[comment_start + 1..]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
    else {
        return;
    };
    let separator_start = comment_start + 1 + relative_start;
    // Keep at least one separator byte as well as the comment marker/prefix.
    if separator_start >= line_length {
        return;
    }
    let separator = line[separator_start];
    let body = &line[separator_start..];
    if body.len() >= 3 && body.iter().all(|byte| *byte == separator) {
        line.truncate(line_length);
    }
}

#[cfg(test)]
mod tests {
    use super::output_whitespace;
    use crate::{config::FormatConfig, transform::document::Document};

    #[test]
    fn wrapped_separator_comments_are_truncated_to_the_line_length() {
        let mut config = FormatConfig::default();
        config.wrap.line_length = 12;
        let mut document = Document::from_bytes(
            b"! --------------------\n!CCCCCCCCCCCCCCCC\n!   ################\n    ! --------------------\n! ordinary words are not separators\n",
        );

        output_whitespace(&mut document, &config).unwrap();

        assert_eq!(
            document.to_bytes(),
            b"! ----------\n!CCCCCCCCCCC\n!   ########\n    ! ------\n! ordinary words are not separators\n"
        );
    }

    #[test]
    fn separator_comments_are_unchanged_when_wrapping_is_disabled() {
        let mut config = FormatConfig::default();
        config.wrap.enabled = false;
        config.wrap.line_length = 12;
        let source = b"! --------------------\n";
        let mut document = Document::from_bytes(source);

        output_whitespace(&mut document, &config).unwrap();

        assert_eq!(document.to_bytes(), source);
    }
}
