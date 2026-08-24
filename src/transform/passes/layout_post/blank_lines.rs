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
    let mut statement_continuation = false;
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
            // A comment carries no statement of its own, so it neither opens a
            // continuation nor closes one: a comment sitting between a `&` and
            // the line it continues leaves the statement exactly as open as it
            // found it. Recomputing the flag from the comment instead would
            // clear it and expose the following blank to the cap, which is the
            // same mistake for the same statement. Preprocessor lines are
            // passed over above and keep the flag for the same reason.
            if let Some(continues) = statement_continues(line) {
                statement_continuation = continues;
            }
            limited.push(line.clone());
        } else if statement_continuation || blank_count < max_blank_lines {
            // A blank line between a `&` and the line it continues is part of
            // that statement, not a separator between two of them. Dropping it
            // changes the statement's physical shape *after* the wrapper has
            // measured it: `--max-blank-lines=0` turned a group the wrapper had
            // declined into one the next run wrapped (WRF `module_HYDRO_io.F90`).
            blank_count += 1;
            limited.push(line.clone());
        }
    }
    document.set_lines(limited);
    Ok(())
}

/// Whether `line` leaves a Fortran statement open, so that a blank line after
/// it belongs to that statement.
///
/// `None` for a comment-only line, which says nothing either way and leaves
/// the caller's answer standing.
fn statement_continues(line: &[u8]) -> Option<bool> {
    // A continuation ampersand is the last nonblank character of the statement
    // part, which a trailing comment may follow.
    let mut state = crate::source::regions::LexState::default();
    let code_end = state
        .regions(line)
        .into_iter()
        .find(|region| region.kind == crate::source::regions::RegionKind::Comment)
        .map_or(line.len(), |comment| comment.range.start);
    let code = line[..code_end].trim_ascii_end();
    if code.is_empty() {
        return None;
    }
    Some(code.last() == Some(&b'&'))
}
