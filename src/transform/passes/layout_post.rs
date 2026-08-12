//! Steps 17-20: passes that run **after** the layout engine has placed every
//! line.
//!
//! The contract for this file is short and absolute: **nothing here may make a
//! line longer.** Wrapping has already happened, so a pass that pads would
//! invalidate a decision it cannot revisit.  Declaration alignment therefore
//! compresses and never pads — that is not an accident of the reference
//! implementation, it is why the pass is allowed to run here at all.

use crate::{
    classify::{classify, StatementKind},
    config::FormatConfig,
    error::FormatError,
    source::{regions::comment_start, scanner},
    transform::{
        document::Document,
        passes::structure::{cpp_line_continues, is_preprocessor_line},
    },
};

/// Step 17: compress the whitespace before a declaration's `::` so a block of
/// declarations lines up.  Compresses only.
///
/// Port target: `normalize_declaration_separator_alignment`.
pub fn declaration_separator_alignment(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
    let mut lines = document.lines.clone();
    loop {
        let mut cpp_lines = Vec::with_capacity(lines.len());
        let mut cpp_continuation = false;
        for line in &lines {
            let cpp_line = cpp_continuation || is_preprocessor_line(line);
            cpp_lines.push(cpp_line);
            cpp_continuation = cpp_line && cpp_line_continues(line);
        }

        let separators = lines
            .iter()
            .zip(&cpp_lines)
            .map(|(line, cpp)| {
                if *cpp {
                    None
                } else {
                    declaration_separator_info(line)
                }
            })
            .collect::<Vec<_>>();
        let original = lines.clone();
        let mut updated = original.clone();
        let mut index = 0;
        while index < original.len() {
            if separators[index].is_none() {
                index += 1;
                continue;
            }

            let mut block_indices = Vec::new();
            while index < original.len() {
                if separators[index].is_some() {
                    block_indices.push(index);
                    index += 1;
                    continue;
                }
                if !cpp_lines[index] && original[index].trim_ascii_start().starts_with(b"!") {
                    index += 1;
                    continue;
                }
                break;
            }

            let mut block = Vec::new();
            let mut separator_column = 0;
            for line_index in block_indices {
                let separator = separators[line_index]
                    .expect("separator index collected only for declaration lines");
                let minimum_column = separator.0 - separator.1 + 1;
                let proposed_column = separator_column.max(minimum_column);
                if !block.is_empty()
                    && (separator.0 < proposed_column
                        || block
                            .iter()
                            .any(|(_, (column, _, _))| *column < proposed_column))
                {
                    normalize_declaration_block(&original, &mut updated, &block);
                    block.clear();
                    separator_column = 0;
                }
                let proposed_column = separator_column.max(minimum_column);
                block.push((line_index, separator));
                separator_column = proposed_column;
            }
            normalize_declaration_block(&original, &mut updated, &block);
        }
        if updated == lines {
            break;
        }
        lines = updated;
    }
    document.set_lines(lines);
    Ok(())
}

/// Step 18: blank-line policy around program units and `CONTAINS`.
///
/// Port target: `normalize_program_unit_spacing`.
pub fn program_unit_spacing(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
    let mut normalized = Vec::with_capacity(document.lines.len());
    let mut unit_depth = 0usize;
    let mut type_depth = 0usize;
    let mut interface_depth = 0usize;
    let mut add_blank_before_next = false;
    let mut cpp_continuation = false;

    for line in &document.lines {
        let cpp_line = cpp_continuation || is_preprocessor_line(line);
        if cpp_line {
            if add_blank_before_next && !cpp_continuation {
                if normalized
                    .last()
                    .is_some_and(|previous: &Vec<u8>| !previous.iter().all(u8::is_ascii_whitespace))
                {
                    normalized.push(Vec::new());
                }
                add_blank_before_next = false;
            }
            normalized.push(line.clone());
            cpp_continuation = cpp_line_continues(line);
            continue;
        }
        cpp_continuation = false;

        let code = code_context(line);
        let is_blank = line.iter().all(u8::is_ascii_whitespace);
        if add_blank_before_next {
            if is_blank {
                continue;
            }
            normalized.push(Vec::new());
            add_blank_before_next = false;
        }

        if is_blank
            && (unit_depth > 0 || interface_depth > 0)
            && normalized
                .last()
                .is_some_and(|previous: &Vec<u8>| previous.iter().all(u8::is_ascii_whitespace))
        {
            continue;
        }

        if is_type_definition_end(code) {
            type_depth = type_depth.saturating_sub(1);
        } else if is_type_definition_start(code) {
            type_depth += 1;
        }

        let is_end = interface_depth == 0 && is_program_unit_end(code);
        let is_header = !is_end && (scope_header(code) || is_module_declaration(code));
        if interface_depth == 0 && is_header {
            unit_depth += 1;
        }

        let is_contains = unit_depth > 0 && type_depth == 0 && is_contains_statement(code);
        if is_contains || is_end {
            while normalized
                .last()
                .is_some_and(|previous: &Vec<u8>| previous.iter().all(u8::is_ascii_whitespace))
            {
                normalized.pop();
            }
            if !normalized.is_empty() {
                normalized.push(Vec::new());
            }
        }

        normalized.push(line.clone());
        if is_contains {
            add_blank_before_next = true;
        }
        if is_end {
            unit_depth = unit_depth.saturating_sub(1);
        }
        if is_interface_end(code) {
            interface_depth = interface_depth.saturating_sub(1);
        } else if is_interface_start(code) {
            interface_depth += 1;
        }
    }
    document.set_lines(normalized);
    Ok(())
}

/// Step 19: cap runs of blank lines.
///
/// Port target: `limit_blank_lines`.
pub fn limit_blank_lines(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
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
        } else if blank_count < 2 {
            blank_count += 1;
            limited.push(line.clone());
        }
    }
    document.set_lines(limited);
    Ok(())
}

fn declaration_separator_info(line: &[u8]) -> Option<(usize, usize, usize)> {
    let mut quote = 0;
    let mut index = 0;
    while index < line.len() {
        let byte = line[index];
        if quote != 0 {
            if byte == quote {
                if line.get(index + 1) == Some(&quote) {
                    index += 2;
                    continue;
                }
                quote = 0;
            }
            index += 1;
        } else if byte == b'\'' || byte == b'"' {
            quote = byte;
            index += 1;
        } else if byte == b'!' {
            return None;
        } else if line.get(index..index + 2) == Some(b"::") {
            let mut before = index;
            while before > 0 && matches!(line[before - 1], b' ' | b'\t') {
                before -= 1;
            }
            let mut after = index + 2;
            while after < line.len() && matches!(line[after], b' ' | b'\t') {
                after += 1;
            }
            return Some((index, index - before, after - index - 2));
        } else {
            index += 1;
        }
    }
    None
}

fn normalize_declaration_block(
    original: &[Vec<u8>],
    updated: &mut [Vec<u8>],
    block: &[(usize, (usize, usize, usize))],
) {
    if block.is_empty() {
        return;
    }
    let separator_column = block
        .iter()
        .map(|(_, (column, before, _))| column - before + 1)
        .max()
        .expect("non-empty declaration block");
    let can_compress_alignment = block.len() > 1
        && block
            .iter()
            .all(|(_, (column, _, _))| *column >= separator_column);
    for (line_index, (column, before, after)) in block {
        let prefix_end = column - before;
        let target_column = if can_compress_alignment {
            separator_column
        } else {
            prefix_end + 1
        };
        let suffix_start = column + 2 + after;
        let mut line = original[*line_index][..prefix_end].to_vec();
        line.extend(std::iter::repeat_n(b' ', target_column - prefix_end));
        line.extend_from_slice(b":: ");
        line.extend_from_slice(&original[*line_index][suffix_start..]);
        *updated.get_mut(*line_index).expect("block line in range") = line;
    }
}

fn code_context(line: &[u8]) -> &[u8] {
    comment_start(line).map_or(line, |index| &line[..index])
}

fn trimmed(code: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = code.len();
    while start < end && code[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && code[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &code[start..end]
}

fn first_word(code: &[u8]) -> Option<&[u8]> {
    let code = trimmed(code);
    let end = code
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        .unwrap_or(code.len());
    (end > 0).then_some(&code[..end])
}

fn skip_ascii_whitespace(code: &[u8]) -> &[u8] {
    let start = code
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(code.len());
    &code[start..]
}

fn word_is(code: &[u8], word: &[u8]) -> bool {
    first_word(code).is_some_and(|first| first.eq_ignore_ascii_case(word))
}

fn is_contains_statement(code: &[u8]) -> bool {
    word_is(code, b"contains")
}

fn is_program_unit_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    if code.eq_ignore_ascii_case(b"end") {
        return true;
    }
    if !code
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
        || !code.get(3).is_some_and(u8::is_ascii_whitespace)
    {
        return false;
    }
    matches!(
        first_word(&code[4..]),
        Some(word)
            if word.eq_ignore_ascii_case(b"module")
                || word.eq_ignore_ascii_case(b"program")
                || word.eq_ignore_ascii_case(b"function")
                || word.eq_ignore_ascii_case(b"subroutine")
    )
}

fn is_type_definition_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    code.get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
        && code.get(3..).is_some_and(|rest| {
            let rest = skip_ascii_whitespace(rest);
            first_word(rest).is_some_and(|word| word.eq_ignore_ascii_case(b"type"))
        })
}

fn is_type_definition_start(code: &[u8]) -> bool {
    classify(code).kind == StatementKind::Type
}

fn is_interface_start(code: &[u8]) -> bool {
    let code = trimmed(code);
    word_is(code, b"interface")
        || (word_is(code, b"abstract")
            && first_word(&code[first_word(code).unwrap().len()..])
                .is_some_and(|word| word.eq_ignore_ascii_case(b"interface")))
}

fn is_interface_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    if !code
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
    {
        return false;
    }
    let rest = skip_ascii_whitespace(&code[3..]);
    first_word(rest).is_some_and(|word| word.eq_ignore_ascii_case(b"interface"))
}

fn is_module_declaration(code: &[u8]) -> bool {
    let code = trimmed(code);
    if !word_is(code, b"module") {
        return false;
    }
    let first_len = first_word(code).map_or(0, <[u8]>::len);
    let rest = skip_ascii_whitespace(&code[first_len..]);
    let Some(word) = first_word(rest) else {
        return false;
    };
    !word.eq_ignore_ascii_case(b"procedure")
        && !word.eq_ignore_ascii_case(b"subroutine")
        && !word.eq_ignore_ascii_case(b"function")
}

fn scope_header(code: &[u8]) -> bool {
    let code = code_context(code);
    let tokens = scanner::tokens(code);
    tokens.windows(2).any(|pair| {
        (pair[0].text.eq_ignore_ascii_case(b"function")
            || pair[0].text.eq_ignore_ascii_case(b"subroutine")
            || pair[0].text.eq_ignore_ascii_case(b"program"))
            && pair[1]
                .text
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
    })
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
    use super::{
        declaration_separator_alignment, limit_blank_lines, output_whitespace, program_unit_spacing,
    };
    use crate::{config::FormatConfig, transform::document::Document};

    #[test]
    fn trailing_horizontal_whitespace_is_removed_from_every_line() {
        let mut document = Document::from_bytes(b"x = 1   \n\t\n  y = 2\t \n");
        output_whitespace(&mut document, &FormatConfig::default()).unwrap();
        assert_eq!(document.to_bytes(), b"x = 1\n\n  y = 2\n");
    }

    fn apply_all(source: &[u8]) -> Vec<u8> {
        let mut document = Document::from_bytes(source);
        let config = FormatConfig::default();
        declaration_separator_alignment(&mut document, &config).unwrap();
        program_unit_spacing(&mut document, &config).unwrap();
        limit_blank_lines(&mut document, &config).unwrap();
        output_whitespace(&mut document, &config).unwrap();
        document.to_bytes()
    }

    #[test]
    fn declaration_separator_alignment_compresses_blocks_and_is_idempotent() {
        let source = b"module m\nreal :: a\n! between declarations\ninteger, parameter :: b = 1\ncharacter(len=4) :: literal = '::' ! keep\n#define CPP :: body\\\n  continuation :: bytes\nend module m\n";
        let once = apply_all(source);
        assert!(once.windows(2).any(|pair| pair == b"::"));
        assert!(once.windows(4).any(|window| window == b"'::'"));
        assert!(once.windows(9).any(|window| window == b"#define C"));
        assert_eq!(apply_all(&once), once);
    }

    #[test]
    fn program_unit_spacing_handles_contains_types_interfaces_and_is_idempotent() {
        let source = b"module m\ntype :: t\ncontains\nprocedure :: p\nend type t\ncontains\nsubroutine s\nend subroutine s\nend module m\ninterface\nsubroutine x\nend subroutine x\nend interface\n";
        let once = apply_all(source);
        assert_eq!(apply_all(&once), once);
        assert!(once.windows(2).filter(|pair| *pair == b"\n\n").count() >= 2);
    }

    #[test]
    fn blank_runs_are_capped_without_crossing_cpp_continuations() {
        let source = b"#define A \\\n+\n\n\n\nvalue\n\n\n\nnext\n";
        let mut document = Document::from_bytes(source);
        limit_blank_lines(&mut document, &FormatConfig::default()).unwrap();
        assert_eq!(
            document.to_bytes(),
            b"#define A \\\n+\n\n\nvalue\n\n\nnext\n"
        );
        let once = document.to_bytes();
        limit_blank_lines(&mut document, &FormatConfig::default()).unwrap();
        assert_eq!(document.to_bytes(), once);
    }

    #[test]
    fn post_layout_passes_never_lengthen_retained_lines() {
        let source = b"module m\nreal(dl), intent(in) :: x\n! comment\ninteger :: y\ncontains\nsubroutine s\nend subroutine s\nend module m\n";
        let mut document = Document::from_bytes(source);
        let before = document.lines.clone();
        let config = FormatConfig::default();
        declaration_separator_alignment(&mut document, &config).unwrap();
        assert!(before
            .iter()
            .zip(&document.lines)
            .all(|(old, new)| new.len() <= old.len()));

        let before = document.lines.clone();
        program_unit_spacing(&mut document, &config).unwrap();
        assert_retained_lines_do_not_grow(&before, &document.lines);

        let before = document.lines.clone();
        limit_blank_lines(&mut document, &config).unwrap();
        assert_retained_lines_do_not_grow(&before, &document.lines);

        let before = document.lines.clone();
        output_whitespace(&mut document, &config).unwrap();
        assert!(before
            .iter()
            .zip(&document.lines)
            .all(|(old, new)| new.len() <= old.len()));
    }

    fn assert_retained_lines_do_not_grow(before: &[Vec<u8>], after: &[Vec<u8>]) {
        let old = before
            .iter()
            .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()));
        let new = after
            .iter()
            .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()));
        assert!(old.zip(new).all(|(old, new)| new.len() <= old.len()));
    }
}
