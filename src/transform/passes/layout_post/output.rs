use crate::{
    config::FormatConfig, error::FormatError, source::regions::StreamLexStates,
    transform::document::Document,
};

/// Step 20: remove trailing horizontal whitespace and normalize EOF newline.
pub fn output_whitespace(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
    let had_input = document.trailing_newline
        || document.lines.len() > 1
        || document.lines.first().is_some_and(|line| !line.is_empty());
    trim_trailing_horizontal(document);
    while document.lines.len() > 1 && document.lines.last().is_some_and(Vec::is_empty) {
        document.lines.pop();
    }
    document.trailing_newline = had_input;
    Ok(())
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
