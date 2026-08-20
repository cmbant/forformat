//! Post-layout alignment invariants.
//!
//! A pass here may not change a line's width unless `format::full` has already
//! measured that change. Both declaration and trailing-comment alignment only
//! remove authored slack: a block is compressed onto the narrowest column its
//! members already share, never spread out to a wider one. The declaration
//! separator's owed one-space minimum is the only exception.
//!
//! Compression still pads the short members of a block up to that shared
//! column, so the wrap budget bounds it: a member the column would push past
//! the budget is left out of the block rather than padded over it.

use crate::{
    config::FormatConfig,
    error::FormatError,
    source::{
        regions::{comment_start, line_code_spans, line_comment_start},
        syntax::{conditional_compilation_prefix, is_directive_comment},
        LexState,
    },
    transform::{
        document::Document,
        passes::structure::{cpp_line_continues, is_preprocessor_line},
    },
};

/// Step 17: normalize whitespace around declaration `::` and compress blocks
/// onto the narrowest column every member can already reach.
pub fn declaration_separator_alignment(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<bool, FormatError> {
    if !config.align_declarations {
        return Ok(false);
    }
    // The shared column is the one thing here that makes a line *longer*, and a
    // line this pass pushes past the wrap budget is a line the next run wraps —
    // taking that member out of the block, narrowing the column, and leaving the
    // wrapped fragment to be measured against a layout it just invalidated (I1).
    // The budget is therefore part of "the narrowest column every member can
    // reach", exactly as it is for trailing comments.
    let budget = if config.wrap.enabled {
        config.wrap.line_length
    } else {
        usize::MAX
    };
    let widths: Vec<usize> = document.lines.iter().map(Vec::len).collect();
    let mut lines = document.lines.clone();
    loop {
        let cpp_lines = preprocessor_lines(&lines);
        let separators = column_info(&lines, &cpp_lines, declaration_separator_info_in);
        let original = lines.clone();
        let mut updated = original.clone();
        align_blocks(
            &original,
            &cpp_lines,
            &separators,
            &mut updated,
            budget,
            normalize_declaration_block,
        );
        if updated == lines {
            break;
        }
        lines = updated;
    }
    document.set_lines(lines);
    let widths_changed = document
        .lines
        .iter()
        .map(Vec::len)
        .ne(widths.iter().copied());
    Ok(widths_changed)
}

/// Step 17b: compress a run of author-aligned trailing comments onto the
/// narrowest column that preserves the authored alignment and wrap budget.
pub fn trailing_comment_alignment(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    if !config.align_comments {
        return Ok(());
    }
    let budget = if config.wrap.enabled {
        config.wrap.line_length
    } else {
        usize::MAX
    };
    let mut lines = document.lines.clone();
    loop {
        let cpp_lines = preprocessor_lines(&lines);
        let comments = column_info(&lines, &cpp_lines, trailing_comment_info);
        let original = lines.clone();
        let mut updated = original.clone();
        align_comment_runs(&original, &cpp_lines, &comments, &mut updated, budget);
        if updated == lines {
            break;
        }
        lines = updated;
    }
    document.set_lines(lines);
    Ok(())
}

fn trailing_comment_info(line: &[u8], lex: &mut LexState) -> Option<(usize, usize, usize)> {
    let start = line_comment_start(lex, line)?;
    let comment = &line[start..];
    if comment.starts_with(b"!!")
        || is_directive_comment(comment)
        || comment[1..].iter().all(u8::is_ascii_whitespace)
    {
        return None;
    }
    let code_end = line[..start]
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())?
        + 1;
    Some((start, start - code_end, comment.len()))
}

fn align_comment_runs(
    lines: &[Vec<u8>],
    cpp_lines: &[bool],
    infos: &[Option<(usize, usize, usize)>],
    updated: &mut [Vec<u8>],
    budget: usize,
) {
    let mut index = 0;
    while index < lines.len() {
        let Some(first) = infos[index] else {
            index += 1;
            continue;
        };
        let mut run = vec![index];
        let mut scan = index + 1;
        while scan < lines.len() {
            match infos[scan] {
                Some(info) if info.0 == first.0 => {
                    run.push(scan);
                    index = scan;
                }
                Some(_) => break,
                None => {
                    let transparent = !cpp_lines[scan]
                        && (lines[scan].trim_ascii().is_empty()
                            || lines[scan].trim_ascii_start().starts_with(b"!")
                            || continues_previous_line(lines, scan));
                    if !transparent {
                        break;
                    }
                }
            }
            scan += 1;
        }
        index += 1;

        let target = run
            .iter()
            .map(|line| infos[*line].expect("run members are carriers"))
            .map(|(column, before, _)| column - before + 1)
            .max()
            .expect("a run always has a first member");
        let fits = run
            .iter()
            .map(|line| infos[*line].expect("run members are carriers"))
            .all(|(_, _, comment)| target + comment <= budget);
        if run.len() < 2 || !fits {
            for line in &run {
                let (column, before, _) = infos[*line].expect("run members are carriers");
                set_comment_column(lines, updated, *line, column, before, column - before + 1);
            }
            continue;
        }
        for line in &run {
            let (column, before, _) = infos[*line].expect("run members are carriers");
            set_comment_column(lines, updated, *line, column, before, target);
        }
    }
}

fn set_comment_column(
    original: &[Vec<u8>],
    updated: &mut [Vec<u8>],
    line_index: usize,
    column: usize,
    before: usize,
    target_column: usize,
) {
    let code_end = column - before;
    let mut line = original[line_index][..code_end].to_vec();
    line.resize(target_column, b' ');
    line.extend_from_slice(&original[line_index][column..]);
    *updated.get_mut(line_index).expect("run line in range") = line;
}

/// The `::` a standalone line offers for alignment, scanned from a clean state.
pub(crate) fn declaration_separator_info(line: &[u8]) -> Option<(usize, usize, usize)> {
    declaration_separator_info_in(line, &mut LexState::default())
}

/// The same, for one line of a group whose lexical state is already known.
fn declaration_separator_info_in(line: &[u8], lex: &mut LexState) -> Option<(usize, usize, usize)> {
    let mut found = None;
    line_code_spans(lex, line, |start, span| {
        if found.is_none() {
            if let Some(at) = span.windows(2).position(|pair| pair == b"::") {
                found = Some(start + at);
            }
        }
    });
    let index = found?;
    let mut before = index;
    while before > 0 && matches!(line[before - 1], b' ' | b'\t') {
        before -= 1;
    }
    let mut after = index + 2;
    while after < line.len() && matches!(line[after], b' ' | b'\t') {
        after += 1;
    }
    if before == 0 || after == line.len() {
        return None;
    }
    Some((index, index - before, after - index - 2))
}

fn preprocessor_lines(lines: &[Vec<u8>]) -> Vec<bool> {
    let mut cpp_lines = Vec::with_capacity(lines.len());
    let mut cpp_continuation = false;
    for line in lines {
        let cpp_line = cpp_continuation || is_preprocessor_line(line);
        cpp_lines.push(cpp_line);
        cpp_continuation = cpp_line && cpp_line_continues(line);
    }
    cpp_lines
}

/// Measure one column carrier per line, walking the file in order.
///
/// The lexical state is threaded through the walk because a `!` or a `::` on
/// the second physical line of a continued character literal is literal text:
/// padding it to a column would write bytes into the literal. Conditional
/// sentinels are source prefixes, not Fortran body bytes, so they are removed
/// before scanning and their width is added back to any returned column.
fn column_info(
    lines: &[Vec<u8>],
    cpp_lines: &[bool],
    mut info: impl FnMut(&[u8], &mut LexState) -> Option<(usize, usize, usize)>,
) -> Vec<Option<(usize, usize, usize)>> {
    let mut lex = [LexState::default(), LexState::default()];
    lines
        .iter()
        .zip(cpp_lines)
        .map(|(line, cpp)| {
            if *cpp {
                // A directive line is stepped over, not lexed: it can sit
                // between the halves of a continued literal without ending it.
                return None;
            }
            let prefix = conditional_compilation_prefix(line);
            let stream = usize::from(prefix.is_some());
            let body_start = prefix.map_or(0, |prefix| prefix.body_start);
            let body = &line[body_start..];
            // As in SourceBuffer, malformed/inactive code between the halves
            // of a character literal is transparent unless it has the required
            // leading continuation `&`. Preserve the open lexical state rather
            // than exposing protected `!` or `::` bytes on the resumed line.
            if lex[stream].in_literal() && !body.trim_ascii_start().starts_with(b"&") {
                return None;
            }
            info(body, &mut lex[stream])
                .map(|(column, before, after)| (body_start + column, before, after))
        })
        .collect()
}

type BlockMember = (usize, (usize, usize, usize));
type BlockNormalizer = fn(&[Vec<u8>], &mut [Vec<u8>], &[BlockMember], usize);

fn align_blocks(
    original: &[Vec<u8>],
    cpp_lines: &[bool],
    infos: &[Option<(usize, usize, usize)>],
    updated: &mut [Vec<u8>],
    budget: usize,
    normalize: BlockNormalizer,
) {
    let mut index = 0;
    while index < original.len() {
        if infos[index].is_none() {
            index += 1;
            continue;
        }

        let mut block_indices = collect_paragraph(original, cpp_lines, infos, &mut index);
        while let Some(mut resume) = next_carrier(original, cpp_lines, infos, index) {
            let mut merged = block_indices.clone();
            merged.extend(collect_paragraph(original, cpp_lines, infos, &mut resume));
            if !shares_one_column(infos, &merged) {
                break;
            }
            block_indices = merged;
            index = resume;
        }

        let mut block = Vec::new();
        let mut column = 0;
        for line_index in block_indices {
            let info = infos[line_index].expect("block lines are carriers");
            let minimum_column = info.0 - info.1 + 1;
            let proposed_column = column.max(minimum_column);
            if !block.is_empty()
                && (info.0 < proposed_column
                    || block
                        .iter()
                        .any(|(_, (column, _, _))| *column < proposed_column))
            {
                normalize(original, updated, &block, budget);
                block.clear();
                column = 0;
            }
            let proposed_column = column.max(minimum_column);
            block.push((line_index, info));
            column = proposed_column;
        }
        normalize(original, updated, &block, budget);
    }
}

fn normalize_declaration_block(
    original: &[Vec<u8>],
    updated: &mut [Vec<u8>],
    block: &[BlockMember],
    budget: usize,
) {
    if block.is_empty() {
        return;
    }
    let aligned = aligned_members(original, block, budget);
    for (line_index, (column, before, after)) in block {
        let prefix_end = column - before;
        let target_column = match aligned {
            Some((separator_column, ref members)) if members.contains(line_index) => {
                separator_column
            }
            _ => prefix_end + 1,
        };
        let suffix_start = column + 2 + after;
        let mut line = original[*line_index][..prefix_end].to_vec();
        line.extend(std::iter::repeat_n(b' ', target_column - prefix_end));
        line.extend_from_slice(b":: ");
        line.extend_from_slice(&original[*line_index][suffix_start..]);
        *updated.get_mut(*line_index).expect("block line in range") = line;
    }
}

/// The shared column for a block, and which of its members reach it.
///
/// A member is left out when the shared column would push it past the budget:
/// the column is set by the *widest prefix* in the block, so one wide
/// declaration among short ones would otherwise pad every one of them out of
/// the budget.  Exempting the wide member instead — it keeps its own single
/// space, which is the shortest it can be — is what leaves the rest aligned and
/// inside the budget.  A member already too long at its own single space is the
/// wrapper's problem, not the block's, so it never joins and never sets the
/// column.
fn aligned_members(
    original: &[Vec<u8>],
    block: &[BlockMember],
    budget: usize,
) -> Option<(usize, Vec<usize>)> {
    let minimum_column = |(_, (column, before, _)): &BlockMember| column - before + 1;
    // Only the code is measured. A trailing comment is not the block's to
    // shorten — step 17b owns its column, and a comment that overruns is not
    // something a narrower `::` could have saved — so charging the block for
    // one would collapse the alignment of every commented declaration.
    let width_at = |(line_index, (column, _, after)): &BlockMember, target: usize| {
        let line = &original[*line_index];
        let code = code_context(line);
        let body_start = line.len() - code.len();
        let code_end = body_start + code.trim_ascii_end().len();
        target + 3 + code_end.saturating_sub(column + 2 + after)
    };
    let mut members: Vec<&BlockMember> = block
        .iter()
        .filter(|member| width_at(member, minimum_column(member)) <= budget)
        .collect();
    while members.len() > 1 {
        let separator_column = members
            .iter()
            .map(|member| minimum_column(member))
            .max()
            .expect("non-empty member list");
        if members
            .iter()
            .all(|member| width_at(member, separator_column) <= budget)
        {
            // Alignment is only ever preserved or compressed, never introduced:
            // every member has to have been written at or right of the column.
            return members
                .iter()
                .all(|(_, (column, _, _))| *column >= separator_column)
                .then(|| {
                    (
                        separator_column,
                        members.iter().map(|(line, _)| *line).collect(),
                    )
                });
        }
        members.retain(|member| minimum_column(member) < separator_column);
    }
    None
}

fn continues_previous_line(lines: &[Vec<u8>], index: usize) -> bool {
    lines[..index]
        .iter()
        .rev()
        .map(|line| trimmed(code_context(line)))
        .find(|code| !code.is_empty())
        .is_some_and(|code| code.ends_with(b"&"))
}

fn collect_paragraph(
    lines: &[Vec<u8>],
    cpp_lines: &[bool],
    separators: &[Option<(usize, usize, usize)>],
    index: &mut usize,
) -> Vec<usize> {
    let mut paragraph = Vec::new();
    while *index < lines.len() {
        if separators[*index].is_some() {
            paragraph.push(*index);
            *index += 1;
            continue;
        }
        if !cpp_lines[*index] && lines[*index].trim_ascii_start().starts_with(b"!") {
            *index += 1;
            continue;
        }
        if *index > 0 && continues_previous_line(lines, *index) {
            *index += 1;
            continue;
        }
        break;
    }
    paragraph
}

fn next_carrier(
    lines: &[Vec<u8>],
    cpp_lines: &[bool],
    separators: &[Option<(usize, usize, usize)>],
    index: usize,
) -> Option<usize> {
    let stop = (index..lines.len()).find(|candidate| {
        separators[*candidate].is_some()
            || cpp_lines[*candidate]
            || !(lines[*candidate].trim_ascii().is_empty()
                || lines[*candidate].trim_ascii_start().starts_with(b"!"))
    })?;
    separators[stop].is_some().then_some(stop)
}

fn shares_one_column(separators: &[Option<(usize, usize, usize)>], run: &[usize]) -> bool {
    let minimum = run
        .iter()
        .filter_map(|line| separators[*line])
        .map(|(column, before, _)| column - before + 1)
        .max();
    let Some(minimum) = minimum else {
        return true;
    };
    run.iter()
        .filter_map(|line| separators[*line])
        .all(|(column, _, _)| column >= minimum)
}

fn code_context(line: &[u8]) -> &[u8] {
    let body_start = conditional_compilation_prefix(line).map_or(0, |prefix| prefix.body_start);
    let body = &line[body_start..];
    comment_start(body).map_or(body, |index| &body[..index])
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
