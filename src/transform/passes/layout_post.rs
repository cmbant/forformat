//! Steps 17-20: passes that run **after** the layout engine has placed every
//! line.
//!
//! The contract for this file is short: **a pass here may not change a line's
//! width unless `format::full` has already measured the change.** Wrapping has
//! already happened, so a width the wrapper did not see is a decision it cannot
//! revisit, and the next run would revisit it — which is how I1 breaks.
//!
//! Two passes change width — step 17 (the declaration `::`) and step 17b (the
//! trailing comment).  Both share one rule: **whitespace before the aligned
//! column is only ever removed.** A block may hold a line at a wider column
//! than it would reach alone, but never wider than its author already gave it;
//! a member that cannot reach the shared column splits the block instead of
//! being padded out to it.  The single exception is the space a separator is
//! owed on either side, so `integer::x` grows by up to two columns.
//!
//! Each pass is independently gated (`config.align_declarations`,
//! `config.align_comments`) because they default differently: `::` is a
//! separator with an owed one-space minimum, so aligning it defaults on. A
//! comment's gap has no owed minimum — shrinking one the author maintained by
//! hand is a layout opinion, not a correction — so that default is off.
//!
//! What each pass compresses is an authored alignment column, which on a
//! hand-aligned block can be fifty columns of slack.
//!
//! `format::full` covers both: it runs them over the laid-out document before
//! measuring, and gives the wrapper the separator spelling that will actually
//! be emitted.  A new width-changing pass here has to extend that measurement
//! rather than assume the width is settled.
//!
//! The two differ in what makes a block.  Step 17 asks whether the members
//! *can* share a column, because normalization rewrites the code before a `::`
//! (`TYPE (x)` -> `type(x)`) and their authored columns no longer record what
//! the author did.  Step 17b asks whether they *already* share one, so it
//! never invents a column the author did not maintain — which it can only do
//! because `normalize_comment_spacing` corrects for its own widening and
//! preserves the authored column rather than the authored gap.
//!
//! Step 17b also carries a hazard step 17 does not, and the reason it takes the
//! wrap budget as an input: an over-long line makes the wrapper hoist the
//! trailing comment onto its own line, which removes a member from the run and
//! can *raise* the column for the rest.  Left unchecked that oscillates between
//! runs, so a run whose shared column would put any member over the budget is
//! not aligned at all.

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

/// Step 17: normalize the whitespace before a declaration's `::` so a block of
/// declarations lines up on the narrowest column the block allows.
///
/// A line is never padded out to a wider neighbour's column — the block splits
/// instead — but a separator with no whitespace at all is given the one space it
/// is owed on each side.  See the module docs for what the wrapper has to know
/// about that.
///
/// Gated by `config.align_declarations` (default on): when off, this is a
/// no-op and the whitespace before `::` is left exactly as authored, since no
/// earlier pass touches it either.
///
/// Returns whether any line's *width* changed, which is what the caller needs
/// in order to know that the layout engine's continuation columns for those
/// statements are now stale.
///
/// Port target: `normalize_declaration_separator_alignment`.
pub fn declaration_separator_alignment(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<bool, FormatError> {
    if !config.align_declarations {
        return Ok(false);
    }
    let widths: Vec<usize> = document.lines.iter().map(Vec::len).collect();
    let mut lines = document.lines.clone();
    loop {
        let cpp_lines = preprocessor_lines(&lines);
        let separators = column_info(&lines, &cpp_lines, declaration_separator_info);
        let original = lines.clone();
        let mut updated = original.clone();
        align_blocks(
            &original,
            &cpp_lines,
            &separators,
            &mut updated,
            normalize_declaration_block,
        );
        if updated == lines {
            break;
        }
        lines = updated;
    }
    document.set_lines(lines);
    // The pass never adds or removes a line, so the widths line up one to one.
    let widths_changed = document
        .lines
        .iter()
        .map(Vec::len)
        .ne(widths.iter().copied());
    Ok(widths_changed)
}

/// Step 17b: compress a block of hand-aligned trailing comments onto the
/// narrowest column the block allows.
///
/// This is the `::` rule over a different column, and it exists for the same
/// reason: a column of trailing comments is a deliberate layout an author
/// maintained by hand, and the per-line pass that normalizes comment spacing
/// cannot see enough to keep it — it looks at one line.  So that pass keeps
/// the authored gap and this one makes the block-wide decision, once the
/// layout engine has settled the code columns the comments hang off.
///
/// The pass only ever *shrinks* a gap, so unlike step 17 it cannot invalidate
/// a wrap decision.  It still has to run before `format::full` measures, or
/// the wrapper would size lines against a gap that is about to disappear.
///
/// A block of one is its own minimum, which is one space — the behaviour
/// before comment gaps were preserved, and still what an isolated trailing
/// comment gets when this pass runs.
///
/// Gated by `config.align_comments` (default **off**): a comment's gap is not
/// a separator with a one-space minimum it is owed, so unlike declarations
/// there is no default this pass falls back to — shrinking a hand-aligned
/// comment column is a layout opinion this formatter only imposes when asked.
/// Off, this is a no-op and every comment keeps the gap
/// `normalize_comment_spacing` preserved.
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

/// The trailing comment on a line that also carries code.
///
/// A full-line comment is not a carrier — it has no code column to hang off.
/// The rest of the exclusions are exactly the ones
/// `normalize_comment_spacing` makes: a directive (`!$omp` and friends) is a
/// statement wearing a comment's syntax, and a `!!` doc comment or a bare `!`
/// is left as authored. Keeping the two sets identical is the point — the
/// pass that preserves a gap is the pass that compresses it, so nothing is
/// left holding a gap no one will ever settle.
///
/// The third field is the comment's own length, which the budget check needs.
fn trailing_comment_info(line: &[u8]) -> Option<(usize, usize, usize)> {
    let start = comment_start(line)?;
    let comment = &line[start..];
    if comment.starts_with(b"!!")
        || crate::transform::passes::line_rules::is_directive_comment(comment)
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

/// Compress each run of trailing comments that the author aligned.
///
/// Unlike the `::` rule this asks for *equality*, not reachability: a run is
/// only compressed when its members already sit on one column, so a column the
/// author maintained is preserved and one that never existed is not invented.
/// Comments the author left at a single space are therefore never padded, and
/// two comments that merely both have slack are not pulled into line with each
/// other. Equality is meaningful here only because
/// `normalize_comment_spacing` corrects for its own widening — without that,
/// normalization alone would break up runs that really were aligned.
///
/// Blank and full-comment lines are transparent; any other line ends the run.
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
                // Only a comment on the same column continues the run.
                Some(info) if info.0 == first.0 => {
                    run.push(scan);
                    index = scan;
                }
                Some(_) => break,
                None => {
                    let transparent = !cpp_lines[scan]
                        && (lines[scan].trim_ascii().is_empty()
                            || lines[scan].trim_ascii_start().starts_with(b"!"));
                    if !transparent {
                        break;
                    }
                }
            }
            scan += 1;
        }
        index += 1;

        // One space after the longest member, unless that would push some
        // member past the wrap budget — an over-long line makes the next run
        // hoist the comment out, which changes the run and so the column, and
        // the two runs never agree.
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
            // Not an alignment this pass can keep, so every member falls back
            // to the single space an unaligned trailing comment gets.
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

pub(crate) fn declaration_separator_info(line: &[u8]) -> Option<(usize, usize, usize)> {
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

/// Which lines are preprocessor directives, continuations included.
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

/// Per-line column info for one alignment, with preprocessor lines excluded.
fn column_info(
    lines: &[Vec<u8>],
    cpp_lines: &[bool],
    info: impl Fn(&[u8]) -> Option<(usize, usize, usize)>,
) -> Vec<Option<(usize, usize, usize)>> {
    lines
        .iter()
        .zip(cpp_lines)
        .map(|(line, cpp)| if *cpp { None } else { info(line) })
        .collect()
}

/// A block member: which line, and its `(column, before, after)` info.
type BlockMember = (usize, (usize, usize, usize));

/// A block's `normalize` callback: original lines, the output to write into,
/// and the block's members.
type BlockNormalizer = fn(&[Vec<u8>], &mut [Vec<u8>], &[BlockMember]);

/// Partition the lines into alignment blocks and hand each to `normalize`.
///
/// Both alignments in this file — the declaration `::` and the trailing
/// comment — are the same problem over a different column, so they share the
/// partition: consecutive carriers, merged across blank lines while the merged
/// set can still share a column, and split as soon as one member would have to
/// be padded out to reach it.
fn align_blocks(
    original: &[Vec<u8>],
    cpp_lines: &[bool],
    infos: &[Option<(usize, usize, usize)>],
    updated: &mut [Vec<u8>],
    normalize: BlockNormalizer,
) {
    let mut index = 0;
    while index < original.len() {
        if infos[index].is_none() {
            index += 1;
            continue;
        }

        let mut block_indices = collect_paragraph(original, cpp_lines, infos, &mut index);
        // A blank line is spacing inside a hand-aligned block, not the end of
        // one: an author who groups declarations into paragraphs still lines
        // the whole set up on one column, and compressing each paragraph to
        // its own minimum would leave the set ragged.  The set has to be able
        // to keep sharing a column, though — merging a paragraph that cannot
        // would let a narrow one drag the column down and split the rest,
        // which is worse than not merging.
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
                normalize(original, updated, &block);
                block.clear();
                column = 0;
            }
            let proposed_column = column.max(minimum_column);
            block.push((line_index, info));
            column = proposed_column;
        }
        normalize(original, updated, &block);
    }
}

fn normalize_declaration_block(
    original: &[Vec<u8>],
    updated: &mut [Vec<u8>],
    block: &[BlockMember],
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

/// Whether `lines[index]` continues the statement on the line before it.
///
/// Comment and blank lines between the two are transparent, matching the way
/// the block scan already steps over comments.
fn continues_previous_line(lines: &[Vec<u8>], index: usize) -> bool {
    lines[..index]
        .iter()
        .rev()
        .map(|line| trimmed(code_context(line)))
        .find(|code| !code.is_empty())
        .is_some_and(|code| code.ends_with(b"&"))
}

/// Collect one paragraph of alignment carriers, advancing `index` past it.
///
/// Comment lines and continuations are transparent: a continuation belongs to
/// the declaration above it, so letting it end the paragraph would make the
/// partition depend on where the wrapper chose to break, while the wrapper's
/// budget depends on the column this pass picks from the partition — a loop
/// that resolved differently on each run.
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

/// The next carrier of this alignment reachable from `index` across blank and
/// comment lines only.  Anything else — code, a preprocessor line, end of
/// file — means the block really has ended and there is nothing to merge.
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

/// Whether every declaration in `run` can sit on one separator column: the
/// widest one's minimum, which no member may be narrower than.  This is the
/// same no-padding rule the block loop applies incrementally, asked ahead of
/// time about a merge that has not happened yet.
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

/// Step 20: trailing horizontal whitespace and the final newline.
///
/// This matches the formatter's pre-commit-style EOF policy: an
/// empty input stays empty, while every non-empty input ends in exactly one
/// newline.  The newline sequence itself remains the document's dominant one.
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

#[cfg(test)]
mod tests {
    use super::{
        declaration_separator_alignment, limit_blank_lines, output_whitespace,
        program_unit_spacing, trailing_comment_alignment,
    };
    use crate::{config::FormatConfig, transform::document::Document};

    #[test]
    fn trailing_horizontal_whitespace_is_removed_from_every_line() {
        let mut document = Document::from_bytes(b"x = 1   \n\t\n  y = 2\t \n");
        output_whitespace(&mut document, &FormatConfig::default()).unwrap();
        assert_eq!(document.to_bytes(), b"x = 1\n\n  y = 2\n");
    }

    #[test]
    fn final_newlines_match_end_of_file_fixer() {
        for (source, expected) in [
            (b"".as_slice(), b"".as_slice()),
            (b"x = 1".as_slice(), b"x = 1\n".as_slice()),
            (b"x = 1\n\n\n".as_slice(), b"x = 1\n".as_slice()),
            (b"x = 1\r\n\r\n".as_slice(), b"x = 1\r\n".as_slice()),
            (b"\n\n".as_slice(), b"\n".as_slice()),
        ] {
            let mut document = Document::from_bytes(source);
            output_whitespace(&mut document, &FormatConfig::default()).unwrap();
            assert_eq!(document.to_bytes(), expected, "source: {source:?}");
        }
    }

    fn apply_all(source: &[u8]) -> Vec<u8> {
        apply_all_with(source, &FormatConfig::default())
    }

    /// Comment alignment defaults off (`config.align_comments`), so most of
    /// the tests below that exercise it opt in explicitly.
    fn apply_all_with_comment_alignment(source: &[u8]) -> Vec<u8> {
        let config = FormatConfig {
            align_comments: true,
            ..FormatConfig::default()
        };
        apply_all_with(source, &config)
    }

    fn apply_all_with(source: &[u8], config: &FormatConfig) -> Vec<u8> {
        let mut document = Document::from_bytes(source);
        declaration_separator_alignment(&mut document, config).unwrap();
        trailing_comment_alignment(&mut document, config).unwrap();
        program_unit_spacing(&mut document, config).unwrap();
        limit_blank_lines(&mut document, config).unwrap();
        output_whitespace(&mut document, config).unwrap();
        document.to_bytes()
    }

    #[test]
    fn trailing_comments_compress_onto_one_column_across_blank_lines() {
        let source = b"integer(IntKi)  :: i, j, j_ss        ! generic loop counter\ninteger(IntKi)  :: ErrStat           ! Status of error message\n\ninteger(IntKi)  :: n_t_global        ! global-loop time counter\n";
        let output = apply_all_with_comment_alignment(source);
        for expected in [
            b"integer(IntKi) :: i, j, j_ss ! generic loop counter".as_slice(),
            b"integer(IntKi) :: ErrStat    ! Status of error message",
            b"integer(IntKi) :: n_t_global ! global-loop time counter",
        ] {
            assert!(
                output.windows(expected.len()).any(|w| w == expected),
                "missing {}",
                String::from_utf8_lossy(expected)
            );
        }
    }

    #[test]
    fn an_isolated_trailing_comment_keeps_one_space() {
        // A block of one is its own minimum, which is the single space an
        // isolated trailing comment got before gaps were preserved.
        let output =
            apply_all_with_comment_alignment(b"call sub('IF THEN END')      ! a lone comment\n");
        assert_eq!(output, b"call sub('IF THEN END') ! a lone comment\n");
    }

    #[test]
    fn comment_alignment_is_off_by_default_and_leaves_the_authored_gap() {
        // Off by default (config.align_comments): unlike `::`, a comment's
        // gap has no owed minimum, so with nothing enabling this pass, every
        // comment keeps exactly the gap normalize_comment_spacing left it —
        // even a lone comment with plenty of slack.
        let output = apply_all(b"call sub('IF THEN END')      ! a lone comment\n");
        assert_eq!(output, b"call sub('IF THEN END')      ! a lone comment\n");
    }

    #[test]
    fn declaration_alignment_still_shrinks_by_default_when_comment_alignment_does_not() {
        let source = b"integer(IntKi)      :: i        ! keeps its gap\ninteger(IntKi)      :: errstat  ! shorter gap\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"integer(IntKi) :: i".len())
            .any(|w| w == b"integer(IntKi) :: i"));
        assert!(output
            .windows(b"integer(IntKi) :: errstat".len())
            .any(|w| w == b"integer(IntKi) :: errstat"));
        assert!(output
            .windows(b"i        ! keeps its gap".len())
            .any(|w| w == b"i        ! keeps its gap"));
        assert!(output
            .windows(b"errstat  ! shorter gap".len())
            .any(|w| w == b"errstat  ! shorter gap"));
    }

    #[test]
    fn declaration_alignment_can_be_disabled_to_preserve_authored_spacing() {
        let config = FormatConfig {
            align_declarations: false,
            ..FormatConfig::default()
        };
        let source = b"real      :: first\ninteger   :: second\n";
        let output = apply_all_with(source, &config);
        assert_eq!(output, source.to_vec());
    }

    #[test]
    fn a_trailing_comment_is_not_padded_out_to_a_wider_neighbour() {
        // The no-padding rule again: the short line cannot reach the column the
        // long one needs, so the two do not share one.
        let source = b"integer :: a_very_long_variable_name_here ! first\ninteger :: b                             ! second\n";
        let output = apply_all_with_comment_alignment(source);
        assert!(output
            .windows(b"_here ! first".len())
            .any(|w| w == b"_here ! first"));
        assert!(output
            .windows(b":: b ! second".len())
            .any(|w| w == b":: b ! second"));
    }

    #[test]
    fn a_doc_comment_and_a_directive_keep_their_authored_gap() {
        let output =
            apply_all_with_comment_alignment(b"end if  !! trailing doc\ny = 1  !$omp barrier\n");
        assert!(output
            .windows(b"end if  !! trailing doc".len())
            .any(|w| w == b"end if  !! trailing doc"));
        assert!(output
            .windows(b"y = 1  !$omp barrier".len())
            .any(|w| w == b"y = 1  !$omp barrier"));
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
    fn declaration_alignment_preserves_the_minimum_separator() {
        let source = b"real      :: first\ninteger   :: second\n\n! comment\n\nlogical   :: third\n\n\nreal   :: unaligned\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"real    :: first".len())
            .any(|w| w == b"real    :: first"));
        assert!(output
            .windows(b"integer :: second".len())
            .any(|w| w == b"integer :: second"));
        assert!(output
            .windows(b"logical :: third".len())
            .any(|w| w == b"logical :: third"));
        assert!(!output
            .windows(b"real   :: first".len())
            .any(|w| w == b"real   :: first"));
    }

    #[test]
    fn declaration_alignment_compresses_through_blank_lines() {
        // Paragraphs of a hand-aligned block share one column, so compressing
        // each one to its own minimum would leave the block ragged. Blank lines
        // are spacing inside the block, not a boundary.
        let source = b"integer(IntKi)                :: ErrStat\n   \nreal(DbKi)                    :: t_global\n\ntype(MAP_InitOutputType)      :: InitOutData_MAP\n";
        let output = apply_all(source);
        for expected in [
            b"integer(IntKi)           :: ErrStat".as_slice(),
            b"real(DbKi)               :: t_global",
            b"type(MAP_InitOutputType) :: InitOutData_MAP",
        ] {
            assert!(
                output.windows(expected.len()).any(|w| w == expected),
                "missing {}",
                String::from_utf8_lossy(expected)
            );
        }
    }

    #[test]
    fn alignment_only_ever_removes_whitespace() {
        // The rule that keeps this pass safe, over both columns it aligns:
        // merging blocks may hold a line at a wider column than it would reach
        // alone, but never at a wider one than its author already gave it. The
        // sole exception is the space a bare `integer::x` is owed.
        for source in [
            b"integer(IntKi)                :: a  ! one\n\ninteger(IntKi)                :: bb ! two\n\ntype(a_long_type_name_here)   :: c  ! three\n".as_slice(),
            b"real      :: first\ninteger   :: second\n\n! comment\n\nlogical   :: third\n\n\nreal   :: unaligned\n",
            b"integer :: a\n\ntype(a_very_long_derived_type_name) :: b\n",
        ] {
            let output = apply_all_with_comment_alignment(source);
            let gaps = |text: &[u8], marker: &[u8]| -> Vec<usize> {
                text.split(|byte| *byte == b'\n')
                    .filter_map(|line| {
                        let at = line
                            .windows(marker.len())
                            .position(|window| window == marker)?;
                        Some(line[..at].len() - line[..at].trim_ascii_end().len())
                    })
                    .collect()
            };
            for marker in [b"::".as_slice(), b"!"] {
                for (before, after) in gaps(source, marker).iter().zip(gaps(&output, marker)) {
                    assert!(
                        after <= *before || *before == 0,
                        "{marker:?} gap grew from {before} to {after} in {}",
                        String::from_utf8_lossy(source)
                    );
                }
            }
        }
    }

    #[test]
    fn a_blank_line_still_splits_a_block_that_cannot_share_a_column() {
        // Joining paragraphs must never pad a narrower one out to a wider
        // neighbour's column; that is still a split.
        let source = b"integer :: a\n\ntype(a_very_long_derived_type_name) :: b\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"integer :: a".len())
            .any(|w| w == b"integer :: a"));
        assert!(output
            .windows(b"type(a_very_long_derived_type_name) :: b".len())
            .any(|w| w == b"type(a_very_long_derived_type_name) :: b"));
    }

    #[test]
    fn declaration_alignment_reduces_procedure_generic_and_attribute_blocks() {
        let source = b"procedure, private  :: WriteSizedArray1\nprocedure, private  :: WriteSizedArray2\ngeneric  :: LoadTxt => LoadTxt_2D, LoadTxt_1D\ninteger, intent(in)   :: md\nreal(GI), intent(in)    :: xd(nxd)\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"procedure, private :: WriteSizedArray1".len())
            .any(|w| w == b"procedure, private :: WriteSizedArray1"));
        assert!(output
            .windows(b"generic :: LoadTxt =>".len())
            .any(|w| w == b"generic :: LoadTxt =>"));
        assert!(output
            .windows(b"integer, intent(in)  :: md".len())
            .any(|w| w == b"integer, intent(in)  :: md"));
        assert!(output
            .windows(b"real(GI), intent(in) :: xd(nxd)".len())
            .any(|w| w == b"real(GI), intent(in) :: xd(nxd)"));
    }

    #[test]
    fn declaration_alignment_compresses_through_comment_lines() {
        let source = b"real(dl), intent(in)              :: ax\nreal(dl), intent(in)              :: bx\n!! of the final result\nreal(dl), intent(out)             :: xzero\ninteger, intent(out)              :: iflag\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"real(dl), intent(in)  :: ax".len())
            .any(|w| w == b"real(dl), intent(in)  :: ax"));
        assert!(output
            .windows(b"real(dl), intent(out) :: xzero".len())
            .any(|w| w == b"real(dl), intent(out) :: xzero"));
        assert!(output
            .windows(b"integer, intent(out)  :: iflag".len())
            .any(|w| w == b"integer, intent(out)  :: iflag"));
    }

    #[test]
    fn declaration_alignment_keeps_a_compressible_subblock_before_an_unaligned_line() {
        let source = b"real(dl), intent(in)              :: ax\nreal(dl), intent(in)              :: bx\nreal(dl), intent(in), optional     :: fax\nreal(dl), parameter :: one = 1._dl\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"real(dl), intent(in)           :: ax".len())
            .any(|w| w == b"real(dl), intent(in)           :: ax"));
        assert!(output
            .windows(b"real(dl), intent(in), optional :: fax".len())
            .any(|w| w == b"real(dl), intent(in), optional :: fax"));
        assert!(output
            .windows(b"real(dl), parameter :: one".len())
            .any(|w| w == b"real(dl), parameter :: one"));
    }

    #[test]
    fn declaration_alignment_never_adds_padding_to_short_lines() {
        let source = b"type(c_ptr) :: cptr\ntype(ModelParams), pointer :: PType\nclass(InterfaceClass), pointer :: P\n\nclass(ModelParams), target :: this\ntype(ModelParams), pointer :: p\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"type(c_ptr) :: cptr".len())
            .any(|w| w == b"type(c_ptr) :: cptr"));
        assert!(output
            .windows(b"type(ModelParams), pointer :: PType".len())
            .any(|w| w == b"type(ModelParams), pointer :: PType"));
        assert!(!output
            .windows(b"type(c_ptr)     :: cptr".len())
            .any(|w| w == b"type(c_ptr)     :: cptr"));
    }

    #[test]
    fn program_unit_spacing_handles_contains_types_interfaces_and_is_idempotent() {
        let source = b"module m\ntype :: t\ncontains\nprocedure :: p\nend type t\ncontains\nsubroutine s\nend subroutine s\nend module m\ninterface\nsubroutine x\nend subroutine x\nend interface\n";
        let once = apply_all(source);
        assert_eq!(apply_all(&once), once);
        assert!(once.windows(2).filter(|pair| *pair == b"\n\n").count() >= 2);
    }

    #[test]
    fn module_interfaces_are_limited_to_one_blank_line() {
        let source = b"module demo\n\n\ninterface\n\n\nend interface\n\n\ncontains\nsubroutine work\nend subroutine work\nend module demo\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"module demo\n\ninterface\n\nend interface\n\ncontains".len())
            .any(|w| w == b"module demo\n\ninterface\n\nend interface\n\ncontains"));
    }

    #[test]
    fn contains_boundaries_keep_exactly_one_blank_line() {
        let source = b"module demo\ninteger :: value\n\n\n\ncontains\n\n\n\nsubroutine work\nend subroutine work\nend module demo\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"integer :: value\n\ncontains\n\nsubroutine work".len())
            .any(|w| w == b"integer :: value\n\ncontains\n\nsubroutine work"));
    }

    #[test]
    fn contains_after_select_type_keeps_the_following_blank_line() {
        let source = b"function format_value(value) result(text)\nclass(*) :: value\nselect type (value)\ntype is (integer)\ntext = 'integer'\nend select\ncontains\nsubroutine error\nend subroutine error\nend function format_value\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"end select\n\ncontains\n\nsubroutine error".len())
            .any(|w| w == b"end select\n\ncontains\n\nsubroutine error"));
    }

    #[test]
    fn bare_program_unit_ends_have_the_same_separator_as_named_ends() {
        let source = b"subroutine first\ninteger :: value\nvalue = 1\nend\nsubroutine second\ninteger :: value\nvalue = 2\nend\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"value = 1\n\nend\nsubroutine second".len())
            .any(|w| w == b"value = 1\n\nend\nsubroutine second"));
    }

    #[test]
    fn named_program_unit_end_reduces_the_following_blank_run() {
        let source = b"subroutine a\nend subroutine a\n\n\n\nx=1\n";
        let output = apply_all(source);
        assert!(output
            .windows(b"end subroutine a\n\n\nx=1".len())
            .any(|w| w == b"end subroutine a\n\n\nx=1"));
        assert!(!output
            .windows(b"end subroutine a\n\n\n\n".len())
            .any(|w| w == b"end subroutine a\n\n\n\n"));
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
