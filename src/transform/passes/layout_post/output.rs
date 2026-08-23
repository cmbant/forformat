use crate::{
    config::FormatConfig, error::FormatError, source::regions::StreamLexStates,
    transform::document::Document,
};

/// Step 20: remove trailing horizontal whitespace, truncate overlong visual
/// separator comments when wrapping is enabled, and normalize EOF newline.
pub fn output_whitespace(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let had_input = document.trailing_newline
        || document.lines.len() > 1
        || document.lines.first().is_some_and(|line| !line.is_empty());
    trim_trailing_horizontal(document);
    if config.wrap.enabled {
        for line in &mut document.lines {
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
/// column, shorten the line to the wrap budget only when the comment prefix and
/// at least one separator byte fit. Ordinary and inline comments are
/// deliberately untouched.
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
    // Preserve indentation, the `!` marker, any spacing after it, and at least
    // one separator byte when the configured budget is extremely small.
    if separator_start >= line_length {
        return;
    }
    let separator = line[separator_start];
    let body = &line[separator_start..];
    if body.len() >= 3 && body.iter().all(|byte| *byte == separator) {
        line.truncate(line_length);
    }
}

/// Step 20's trailing-whitespace half, which every mode owes.
///
/// Interior whitespace is a formatting choice an author can mean — a column of
/// aligned assignments, a hand-spaced expression — so the modes that preserve
/// presentation preserve it. Whitespace at end of line is not that: it is
/// invisible, and no mode has a reason to keep it. This half is therefore split
/// out from the blank-line and EOF-newline policy above, which *is* layout and
/// which normalize-only deliberately does not own.
///
/// Blanks that belong to a character literal or a Hollerith payload are not
/// trailing whitespace at all; see
/// [`protected_trailing_floor`](crate::source::regions::protected_trailing_floor).
pub fn trim_trailing_horizontal(document: &mut Document) {
    let mut streams = StreamLexStates::default();
    for line in &mut document.lines {
        let floor = streams.protected_trailing_floor(line);
        let mut end = line.len();
        while end > floor && matches!(line[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        line.truncate(end);
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
    fn separator_comments_keep_prefix_when_budget_is_too_small() {
        let source = b"    ! --------------------\n";
        for line_length in [0, 1, 4, 5, 6] {
            let mut config = FormatConfig::default();
            config.wrap.line_length = line_length;
            let mut document = Document::from_bytes(source);

            output_whitespace(&mut document, &config).unwrap();

            assert_eq!(document.to_bytes(), source, "line_length={line_length}");
        }

        let mut config = FormatConfig::default();
        config.wrap.line_length = 7;
        let mut document = Document::from_bytes(source);
        output_whitespace(&mut document, &config).unwrap();
        assert_eq!(document.to_bytes(), b"    ! -\n");
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
