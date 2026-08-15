//! The full-mode driver.
//!
//! ```text
//! bytes ─► document ─► normalization (steps 1-15)
//!                   ─► wrapping (step 16)
//!                   ─► findent layout engine
//!                   ─► post-layout passes (steps 17-20)
//!                   ─► bytes
//! ```
//!
//! Full-mode style choices are consumed by normalization and post-layout
//! boundaries only.  The indent-only early return remains the compatibility
//! path and never enters these configurable style passes.
//!
//! The order of the last three is the whole design.  Normalization never
//! chooses a column; the layout engine chooses every column; wrapping runs
//! before it and only decides *where text breaks*.  Because the final bytes are
//! literally the output of the indent-only engine over the normalized text,
//! **I2 (`indent_only(full(x)) == full(x)`) holds by construction** — the port
//! plan's hardest invariant is a property of the pipeline shape, not something
//! each new rule has to be careful about.
//!
//! I1 (`full(full(x)) == full(x)`) is not free in the same way: it holds only
//! if every normalization pass is idempotent, which is a per-pass obligation and
//! a per-pass test.

use super::{
    engine,
    planner::{PlanBody, Planner},
    wrapping::{self, ContinuationLayout, Decline},
};
use crate::{
    analysis::{analyze_file, scoped_declared_names, ProjectContext, ScopeTree},
    config::{FormatConfig, FormatMode},
    error::FormatError,
    source::{LogicalGroup, PhysicalLineKind, SourceBuffer},
    transform::{document::Document, pipeline},
    FormatMeta, FormatResult,
};

type ReflowResult = Vec<(usize, Decline)>;

/// Format one buffer with project context.
pub fn format_with_context(
    source: &[u8],
    project: &ProjectContext,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    if config.mode == FormatMode::IndentOnly {
        return engine::format(source, config);
    }

    let mut document = Document::from_bytes(source);
    let local = analyze_file(source)?;
    pipeline::normalize(&mut document, project, &local, config)?;

    if config.mode == FormatMode::NormalizeOnly {
        let bytes = document.to_bytes();
        return Ok(FormatResult {
            bytes,
            meta: FormatMeta::default(),
        });
    }

    if config.wrap.enabled {
        let declined = reflow_with_context_inner(&mut document, project, &local, config)?;
        // Every long line the wrapper refuses is explainable; the diagnostic
        // separates "unwrappable by design" from a wrapper bug.
        let (output, meta) = lay_out(&document, config)?;
        return Ok(FormatResult {
            bytes: output.to_bytes(),
            meta: FormatMeta {
                last_indent: meta.last_indent,
                last_usable: meta.last_usable,
                declines: declined,
            },
        });
    }

    let (output, meta) = lay_out(&document, config)?;
    Ok(FormatResult {
        bytes: output.to_bytes(),
        meta,
    })
}

/// Run the layout engine and the post-layout passes over the normalized text.
///
/// The layout engine owns every column.  It runs over LF text and its output is
/// re-wrapped into the document's terminator policy here.
///
/// Step 17 then runs *after* the engine and can change a line's width, which
/// silently invalidates the continuation columns the engine chose for that
/// statement: under `--align-paren` those columns are anchored on the head
/// line, so compressing a declaration's `::` leaves every continuation of it
/// stranded to the right of the `[` it was lined up with.  The next run reads
/// the compressed head, aligns correctly, and I1 fails.  Laying the text out
/// again — only when step 17 actually moved something, which on a normal file
/// is rare — costs one engine pass and makes the columns agree with the width
/// that is emitted.  The second post-layout pass is a fixed point of the first,
/// so one repeat is enough; the loop bound says so out loud rather than
/// trusting it.
fn lay_out(
    document: &Document,
    config: &FormatConfig,
) -> Result<(Document, FormatMeta), FormatError> {
    let mut source = document.to_lf_bytes();
    let mut rounds = 2;
    loop {
        let laid_out = engine::format(&source, config)?;
        let mut output = Document::from_bytes(&laid_out.bytes);
        output.newline = document.newline;
        output.trailing_newline = document.trailing_newline;
        let widths_changed = pipeline::post_layout(&mut output, config)?;
        rounds -= 1;
        if !widths_changed || rounds == 0 {
            return Ok((output, laid_out.meta));
        }
        source = output.to_lf_bytes();
    }
}

/// Step 16: reflow statements that overrun the budget.
///
/// The first-line indent and the continuation column both come from the layout
/// plan, so a user who changes `-k` or turns on `--align-paren` changes where
/// wrapped lines start *and* the width the wrapper had to work with, together.
/// A literal `indent + 4` here would silently disagree with the engine the
/// moment either option moved.
pub fn reflow(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<Vec<(usize, Decline)>, FormatError> {
    let local = analyze_file(&document.to_lf_bytes())?;
    reflow_with_context(document, &ProjectContext::empty(), &local, config)
}

pub fn reflow_with_context(
    document: &mut Document,
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<Vec<(usize, Decline)>, FormatError> {
    reflow_with_context_inner(document, project, local, config)
}

fn reflow_with_context_inner(
    document: &mut Document,
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<ReflowResult, FormatError> {
    let analysis = document.analyze()?;
    let scopes = ScopeTree::build(&analysis);
    let declared_names = scoped_declared_names(&analysis, &scopes);
    let context = crate::transform::pipeline::PassContext {
        config,
        project,
        local,
        analysis: &analysis,
        scopes: &scopes,
    };
    let mut planner = Planner::new(config);
    let mut plans = Vec::with_capacity(analysis.groups.len());
    for group in &analysis.groups {
        plans.push(planner.plan(&analysis.buffer, group, config));
    }
    // The budget applies to the bytes this run is about to emit, not to the
    // ones it read.  A statement whose authored lines all fit can still
    // overrun once the layout engine has moved it to its final column — and
    // then the *next* run would see the overrun and rewrap, which is exactly
    // how the fixed point (I1) breaks.  Asking the engine for those columns
    // rather than re-deriving them keeps labels, OpenMP sentinels and
    // `--align-paren` in agreement with the pass that will actually place the
    // text; the engine emits one line per physical line.
    let mut laid_out =
        Document::from_bytes(&engine::format(&document.to_lf_bytes(), config)?.bytes);
    // Step 17 is the one post-layout pass that can make a line *longer*, by
    // giving a declaration's `::` the space it is entitled to.  It rewrites
    // lines in place, so measuring after it keeps the index correspondence and
    // makes the width exact rather than nearly right.
    crate::transform::passes::layout_post::declaration_separator_alignment(&mut laid_out, config)?;
    // Comment alignment only shrinks a gap, so it cannot invalidate a wrap
    // decision — but it must still be measured, or the wrapper sizes lines
    // against an authored gap that is about to be compressed away.
    crate::transform::passes::layout_post::trailing_comment_alignment(&mut laid_out, config)?;
    let laid_out_width = |line: usize| laid_out.lines.get(line).map_or(0, |text| text.len());

    let mut lines: Vec<Vec<u8>> = Vec::with_capacity(document.lines.len());
    let mut declined = Vec::new();
    for (group, plan) in analysis.groups.iter().zip(&plans) {
        if let Some(directive) = join_openmp_directive(document, group) {
            // A directive is measured and wrapped at the column the layout
            // engine is about to move it to.  Measuring the authored indent
            // instead leaves an over-long directive for the next run to find,
            // which breaks the fixed point exactly as it does for statements.
            let directive = reindent(
                &directive,
                match plan.body {
                    PlanBody::Uniform { indent } => indent,
                    PlanBody::Code { first_indent, .. } => first_indent,
                },
            );
            let long = directive.len() > config.wrap.line_length
                || group.lines.clone().any(|index| {
                    document.lines[index].len() > config.wrap.line_length
                        || laid_out_width(index) > config.wrap.line_length
                });
            if long {
                match wrap_openmp_directive(&directive, config.wrap.line_length) {
                    Ok(wrapped) => lines.extend(wrapped),
                    Err(reason) => {
                        declined.push((group.lines.start, reason));
                        copy_group(document, group, &mut lines);
                    }
                }
            } else {
                lines.push(document.lines[group.lines.start].clone());
            }
            if group.lines.len() > 1 {
                for index in group.lines.clone().skip(1) {
                    lines.push(document.lines[index].clone());
                }
            }
            continue;
        }
        let PlanBody::Code {
            first_indent,
            align,
            ..
        } = plan.body
        else {
            copy_group(document, group, &mut lines);
            continue;
        };
        if !eligible(&analysis.buffer, group) {
            copy_group(document, group, &mut lines);
            continue;
        }
        if group.statements.len() != 1 {
            copy_group(document, group, &mut lines);
            continue;
        }
        let index = group.lines.start;
        let layout = ContinuationLayout {
            first_indent,
            continuation: first_indent.saturating_add(if config.indent_continuation {
                config.continuation_indent
            } else {
                0
            }),
        };
        // Only the laid-out width decides.  The normalized line still carries
        // the authored indent and the authored `::` run, and both are about to
        // change: a declaration whose author lined its `::` up in a wide block
        // reads as 120 columns here and is emitted at 79.  Wrapping it anyway
        // broke the fixed point in the worst way, because the wrap changes
        // which lines step 17 groups together and therefore the width the
        // *next* run measures.  Measuring only what is emitted closes that
        // loop: leaving the statement alone leaves the block — and so the
        // measurement — exactly as it was.
        let has_long_physical_line = group
            .lines
            .clone()
            .any(|line| laid_out_width(line) > config.wrap.line_length);
        if !has_long_physical_line {
            copy_group(document, group, &mut lines);
            continue;
        }
        let mut body = trim(&group.statements[0].text).to_vec();
        if group.lines.len() > 1 {
            let original_body = body.clone();
            body = crate::transform::passes::line_rules::respace_joined(
                &body,
                &context,
                &declared_names,
                index,
            );
            body = crate::transform::passes::case_pass::restore_declined_component_spellings(
                &original_body,
                &body,
                index,
                &declared_names,
                &context,
            );
            body = trim(&body).to_vec();
        }
        // Step 17 does not only pad: on a declaration whose author lined the
        // `::` up in a much wider block, it *compresses*, and the statement the
        // wrapper is about to break is then far narrower than the one it read.
        // Measuring the authored run made an over-long declaration unwrappable
        // (`NoSafeBreak`: no break left the head inside the budget) while the
        // emitted, compressed line was both over-long and perfectly breakable —
        // so the next run wrapped it and I1 failed. `laid_out` already has step
        // 17 applied, so its run is the one that will be emitted.
        body = with_laid_out_separator(body, laid_out.lines.get(index));
        // A detached trailing comment belongs to the statement as a whole, not
        // to its last continuation line, so it is written above the statement
        // at the statement's own indent.  The layout engine then places it like
        // any other comment line — which is what makes it stable: forcing it
        // back to the statement indent afterwards disagreed with the engine
        // above a dedented `else if`, and the next run moved it.
        let comment_indent = layout.first_indent;
        let detached = detach_final_inline_comment(document, group, comment_indent);
        // Whatever step 17 is going to add around `::` has to be paid for
        // here, from the same budget: a break chosen against the unpadded text
        // lands one column over once step 17 runs, and the run after that
        // would rewrap it.
        let budget = config
            .wrap
            .line_length
            .saturating_sub(declaration_separator_growth(&body));
        if first_indent + body.len() <= budget {
            match detached {
                Some(Some(comment)) => {
                    lines.extend(comment);
                    if group.lines.len() > 1 {
                        emit_joined_body(&mut lines, &body, first_indent);
                    } else {
                        copy_group_without_final_comment(document, group, &mut lines);
                    }
                }
                Some(None) if group.lines.len() > 1 => {
                    emit_joined_body(&mut lines, &body, first_indent);
                }
                _ => copy_group(document, group, &mut lines),
            }
            continue;
        }
        if detached.is_none() {
            copy_group(document, group, &mut lines);
            continue;
        }
        match wrapping::wrap_body_with_alignment(&body, layout, budget, align) {
            Ok(wrapped) => {
                if let Some(comment) = detached.flatten() {
                    lines.extend(comment);
                }
                lines.extend(wrapped)
            }
            // A decline means the statement stays exactly as authored.  It has
            // to be copied whole: pushing only the first physical line silently
            // deleted the continuations of a multi-line group, which turns an
            // unwrappable statement into a syntax error.
            Err(Decline::Fits) => copy_group(document, group, &mut lines),
            Err(reason) => {
                declined.push((index, reason));
                copy_group(document, group, &mut lines);
            }
        }
    }
    document.set_lines(lines);
    Ok(declined)
}

/// Return the final inline comment, if there is exactly one and it is on the
/// last physical line of the group. `None` means the group is unsafe to detach;
/// `Some(None)` means it has no inline comment.
fn detach_final_inline_comment(
    document: &Document,
    group: &LogicalGroup,
    comment_indent: usize,
) -> Option<Option<Vec<Vec<u8>>>> {
    let mut comments = Vec::new();
    for index in group.lines.clone() {
        if let Some(start) = crate::source::regions::comment_start(&document.lines[index]) {
            comments.push((index, start));
        }
    }
    if comments.is_empty() {
        return Some(None);
    }
    if comments.len() != 1 || comments[0].0 + 1 != group.lines.end {
        return None;
    }
    let (index, start) = comments[0];
    let line = &document.lines[index];
    let mut comment = vec![b' '; comment_indent];
    comment.extend_from_slice(line[start..].trim_ascii_start());
    Some(Some(vec![comment]))
}

fn emit_joined_body(lines: &mut Vec<Vec<u8>>, body: &[u8], first_indent: usize) {
    let mut line = vec![b' '; first_indent];
    line.extend_from_slice(body);
    lines.push(line);
}

fn join_openmp_directive(document: &Document, group: &LogicalGroup) -> Option<Vec<u8>> {
    // A continued OpenMP directive is already a sequence of physical
    // directives.  Joining it here would erase the repeated sentinel and one
    // physical line when the wrapper decides the joined text fits.  Wrapping
    // remains available for a single overlong directive, even when the
    // classifier grouped the following statement with the directive comment.
    let mut indices: Vec<usize> = group.lines.clone().collect();
    if indices.len() > 1 {
        if !is_openmp_line(&document.lines[indices[0]])
            || is_openmp_line(&document.lines[indices[1]])
        {
            return None;
        }
        indices.truncate(1);
    }
    let mut parts = Vec::new();
    let mut omp_style = false;
    let mut indent = Vec::new();
    for (position, index) in indices.into_iter().enumerate() {
        let line = &document.lines[index];
        let start = line.iter().position(|byte| !byte.is_ascii_whitespace())?;
        if !line[start..].starts_with(b"!$") {
            return None;
        }
        if position == 0 {
            indent.extend_from_slice(&line[..start]);
        }
        let mut body = line[start + 2..].trim_ascii_start();
        if body.len() >= 3 && body[..3].eq_ignore_ascii_case(b"omp") {
            omp_style = true;
            body = body[3..].trim_ascii_start();
        }
        if position > 0 && body.first() == Some(&b'&') {
            body = body[1..].trim_ascii_start();
        }
        if crate::source::regions::comment_start(body).is_some() {
            return None;
        }
        if position + 1 < group.lines.len() {
            let mut end = body.len();
            while end > 0 && body[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            if body.get(end - 1) != Some(&b'&') {
                return None;
            }
            body = body[..end - 1].trim_ascii_end();
        }
        parts.push(body);
    }
    let mut joined = indent;
    joined.extend_from_slice(if omp_style { b"!$OMP " } else { b"!$ " });
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            joined.push(b' ');
        }
        joined.extend_from_slice(part);
    }
    Some(joined)
}

fn is_openmp_line(line: &[u8]) -> bool {
    let start = line.iter().position(|byte| !byte.is_ascii_whitespace());
    start.is_some_and(|start| {
        line[start..]
            .get(..5)
            .is_some_and(|prefix| prefix[..2] == *b"!$" && prefix[2..].eq_ignore_ascii_case(b"omp"))
    })
}

fn wrap_openmp_directive(line: &[u8], line_length: usize) -> Result<Vec<Vec<u8>>, Decline> {
    let indent_end = line
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(0);
    let indent = &line[..indent_end];
    let prefix: Vec<u8> = if line
        .get(indent_end + 2..indent_end + 5)
        .is_some_and(|bytes| bytes.eq_ignore_ascii_case(b"omp"))
    {
        [indent, b"!$OMP "].concat()
    } else {
        [indent, b"!$ "].concat()
    };
    if line.len() <= line_length {
        return Ok(vec![line.to_vec()]);
    }
    let mut body = line
        .get(prefix.len()..)
        .ok_or(Decline::NoSafeBreak)?
        .to_vec();
    let mut result = Vec::new();
    while prefix.len() + body.len() > line_length {
        let limit = line_length.saturating_sub(prefix.len() + 2);
        let position = wrapping::wrap_position(&body, limit).ok_or(Decline::NoSafeBreak)?;
        let mut physical = prefix.clone();
        physical.extend_from_slice(trim(&body[..position]));
        physical.extend_from_slice(b" &");
        result.push(physical);
        body = trim(&body[position..]).to_vec();
    }
    let mut last = prefix;
    last.extend_from_slice(&body);
    result.push(last);
    Ok(result)
}

/// Rewrite the whitespace run in front of `body`'s `::` to the run step 17 has
/// already chosen for the same line, so the wrapper measures and breaks the
/// text that will be emitted rather than the text that was authored.
///
/// Only the run is copied, never the column: `body` is the statement without
/// its indent, and for a continued declaration it is the joined form, but in
/// both cases the bytes in front of the `::` are the same ones the laid-out
/// line carries after its indent.
fn with_laid_out_separator(body: Vec<u8>, laid_out: Option<&Vec<u8>>) -> Vec<u8> {
    let Some(laid_out) = laid_out else {
        return body;
    };
    let (Some((at, run, _)), Some((_, laid_out_run, _))) = (
        crate::transform::passes::layout_post::declaration_separator_info(&body),
        crate::transform::passes::layout_post::declaration_separator_info(laid_out),
    ) else {
        return body;
    };
    // A missing space on either side is `declaration_separator_growth`'s
    // business, not this function's.
    if run == laid_out_run || run == 0 || laid_out_run == 0 {
        return body;
    }
    let mut result = Vec::with_capacity(body.len() + laid_out_run);
    result.extend_from_slice(&body[..at - run]);
    result.resize(result.len() + laid_out_run, b' ');
    result.extend_from_slice(&body[at..]);
    result
}

/// How many bytes step 17 will insert around this statement's `::` — and,
/// when the body carries a second one (a `[type ::` array-constructor
/// spelling in the same declaration), around that one too.  Once wrapping
/// puts the constructor's `::` on its own physical line,
/// `declaration_separator_alignment` treats it exactly like a declaration
/// separator and pads it independently — so a budget that only paid for the
/// first `::` left the second one unaccounted for, and a break chosen
/// against the unpadded text landed over budget once step 17 ran.
///
/// `declaration_separator_alignment` never pads one declaration out to a wider
/// neighbour's column — except when there is no whitespace at all, where it
/// writes the one space the separator is owed on each side.  Compression is not
/// this function's business: [`with_laid_out_separator`] has already given
/// `body` the run that will be emitted.
fn declaration_separator_growth(body: &[u8]) -> usize {
    let mut quote = 0u8;
    let mut index = 0;
    let mut growth = 0;
    while index < body.len() {
        let byte = body[index];
        if quote != 0 {
            if byte == quote {
                if body.get(index + 1) == Some(&quote) {
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
            break;
        } else if body.get(index..index + 2) == Some(b"::") {
            let before = usize::from(index == 0 || !matches!(body[index - 1], b' ' | b'\t'));
            let after = usize::from(!matches!(body.get(index + 2), Some(b' ' | b'\t')));
            growth += before + after;
            index += 2;
        } else {
            index += 1;
        }
    }
    growth
}

fn reindent(line: &[u8], indent: usize) -> Vec<u8> {
    let mut result = vec![b' '; indent];
    result.extend_from_slice(line.trim_ascii_start());
    result
}

fn copy_group(document: &Document, group: &LogicalGroup, lines: &mut Vec<Vec<u8>>) {
    for index in group.lines.clone() {
        if let Some(line) = document.lines.get(index) {
            lines.push(line.clone());
        }
    }
}

fn copy_group_without_final_comment(
    document: &Document,
    group: &LogicalGroup,
    lines: &mut Vec<Vec<u8>>,
) {
    let final_line = group.lines.end.saturating_sub(1);
    for index in group.lines.clone() {
        let Some(line) = document.lines.get(index) else {
            continue;
        };
        if index == final_line {
            if let Some(comment) = crate::source::regions::comment_start(line) {
                lines.push(line[..comment].trim_ascii_end().to_vec());
                continue;
            }
        }
        {
            lines.push(line.clone());
        }
    }
}

/// Reflow is declined when the group interleaves anything that cannot sit
/// between a continuation marker and the text it continues (I5).
fn eligible(buffer: &SourceBuffer, group: &LogicalGroup) -> bool {
    group.lines.clone().all(|index| {
        buffer.lines.get(index).is_some_and(|line| {
            matches!(
                line.kind,
                PhysicalLineKind::Code | PhysicalLineKind::FindentFix
            )
        })
    })
}

fn trim(line: &[u8]) -> &[u8] {
    let mut s = line;
    while s.first().is_some_and(u8::is_ascii_whitespace) {
        s = &s[1..];
    }
    while s.last().is_some_and(u8::is_ascii_whitespace) {
        s = &s[..s.len() - 1];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::format_with_context;
    use crate::{
        analysis::{analyze_project, ProjectContext},
        config::{FormatConfig, FormatMode},
        format_source,
        source::LogicalGroup,
        transform::document::Document,
    };
    use std::path::Path;

    fn full(config_setup: impl FnOnce(&mut FormatConfig), source: &[u8]) -> Vec<u8> {
        let mut config = FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        };
        config_setup(&mut config);
        format_with_context(source, &ProjectContext::empty(), &config)
            .unwrap()
            .bytes
    }

    fn profile_full(source: &[u8]) -> Vec<u8> {
        full(
            |config| {
                config.indent = 4;
                config.start_indent = 4;
                config.contains_indent = 0;
                config.openmp = false;
                config.contains_restart = true;
                config.indent_continuation = true;
                config.continuation_indent = 4;
                config.indent_ampersand = true;
                config.construct_indents.set_all(4);
                config.construct_indents.module = 0;
                config.construct_indents.procedure = 0;
                config.construct_indents.interface = 0;
            },
            source,
        )
    }

    #[test]
    fn conditional_sentinel_body_follows_declared_case_with_or_without_project_tables() {
        let source = b"module t\ninteger :: MyVar\ncontains\nsubroutine s()\n!$ myvar = 1\nmyvar = 2\nend subroutine s\nend module t\n";
        let config = FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        };
        let expected = b"module t\n   integer :: MyVar\n\ncontains\n\n   subroutine s\n!$    MyVar = 1\n      MyVar = 2\n\n   end subroutine s\n\nend module t\n";
        let empty = format_with_context(source, &ProjectContext::empty(), &config)
            .unwrap()
            .bytes;
        let project = analyze_project([(Path::new("sentinel.f90"), source.as_slice())]).unwrap();
        let single_file = format_with_context(source, &project, &config)
            .unwrap()
            .bytes;
        assert_eq!(empty, expected);
        assert_eq!(single_file, expected);
    }

    #[test]
    fn reflow_reuses_component_case_from_the_unjoined_statement() {
        let source = b"module m\ntype :: T\ninteger :: FIRST\ninteger :: SECOND\nend type T\ncontains\nsubroutine s(this)\ntype(T) :: this\nthis%first = this%second + 12345678901234567890 + 12345678901234567890 + 12345678901234567890\nend subroutine s\nend module m\n";
        let one_line = full(|config| config.wrap.line_length = 120, source);
        let continued = full(|config| config.wrap.line_length = 70, source);
        let one_line_text = String::from_utf8(one_line).unwrap();
        let continued_text = String::from_utf8(continued).unwrap();

        assert!(one_line_text.contains("this%FIRST = this%SECOND"));
        assert!(continued_text.contains("this%FIRST = this%SECOND"));
        assert!(continued_text.contains("&\n"));
    }

    #[test]
    fn detached_comment_uses_the_single_line_layout_indent() {
        let source = br#"module m
implicit none
contains
subroutine s(a)
real :: a
call some_procedure_with_a_long_name(argument_number_1, argument_number_2, argument_number_3, argument_number_4, argument_number_5, argument_number_6, argument_number_7, argument_number_8, argument_number_9, argument_number_10, argument_number_11) ! short note
end subroutine s
end module m
"#;
        let once = profile_full(source);
        let twice = profile_full(&once);
        assert_eq!(once, twice);
        assert!(String::from_utf8_lossy(&once).contains("    ! short note\n"));
    }

    #[test]
    fn a_fitting_joined_group_is_emitted_as_one_statement() {
        let source = br#"module m
implicit none
contains
subroutine s(a)
real :: a
a = 1 ! this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long
call f(a, &
a) ! this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long
b = 2 ! short
end subroutine s
end module m
"#;
        let once = profile_full(source);
        let twice = profile_full(&once);
        assert_eq!(once, twice);
        let output = String::from_utf8_lossy(&once);
        assert!(output.contains("    ! this trailing comment"));
        assert!(output.contains("    call f(a, a)\n"));
        assert!(!output.contains("call f(a, &\n"));
    }

    #[test]
    fn only_the_final_line_comment_is_stripped() {
        let document = Document::from_bytes(b"  code ! keep\n  code ! strip\n");
        let group = LogicalGroup {
            lines: 0..2,
            statements: Vec::new(),
            pieces: Vec::new(),
        };
        let mut once = Vec::new();
        super::copy_group_without_final_comment(&document, &group, &mut once);
        let transformed = Document::from_bytes(b"  code ! keep\n  code\n");
        let mut twice = Vec::new();
        super::copy_group_without_final_comment(&transformed, &group, &mut twice);
        assert_eq!(once, [b"  code ! keep".to_vec(), b"  code".to_vec()]);
        assert_eq!(twice, once);
    }

    #[test]
    fn full_output_is_a_findent_fixed_point() {
        // I2: running indent-only over full output must change nothing.
        let source =
            b"PROGRAM Main\nIF (X > 1) THEN\nCALL DoThing(Value)\nEND IF\nEND PROGRAM Main\n";
        let once = full(|_| {}, source);
        let indent_only = format_source(&once, &FormatConfig::default())
            .unwrap()
            .bytes;
        assert_eq!(
            String::from_utf8_lossy(&indent_only),
            String::from_utf8_lossy(&once)
        );
    }

    #[test]
    fn empty_openmp_sentinel_is_trimmed_and_remains_an_indent_fixed_point() {
        let once = full(|_| {}, b"\n!$ \n");
        assert_eq!(once, b"\n!$\n");
        let indent_only = format_source(&once, &FormatConfig::default())
            .unwrap()
            .bytes;
        assert_eq!(indent_only, once);
    }

    #[test]
    fn full_formatting_reaches_its_fixed_point_in_one_pass() {
        // I1.
        for source in [
            b"PROGRAM p\nX = 1\nEND PROGRAM p\n".as_slice(),
            b"module m\ncontains\nSUBROUTINE s()\nEND SUBROUTINE s\nend module m\n".as_slice(),
            b"".as_slice(),
            b"! just a comment\n".as_slice(),
        ] {
            let once = full(|_| {}, source);
            let twice = full(|_| {}, &once);
            assert_eq!(
                String::from_utf8_lossy(&twice),
                String::from_utf8_lossy(&once),
                "not idempotent for {source:?}"
            );
        }
    }

    #[test]
    fn the_dominant_line_ending_is_restored() {
        let crlf = full(|_| {}, b"PROGRAM p\r\nX = 1\r\nEND PROGRAM p\r\n");
        assert!(crlf.windows(2).any(|pair| pair == b"\r\n"));
        assert_eq!(
            String::from_utf8_lossy(&crlf),
            "program p\r\n   X = 1\r\n\r\nend program p\r\n"
        );
    }

    #[test]
    fn full_mode_normalizes_the_final_newline() {
        assert_eq!(full(|_| {}, b""), b"");
        assert_eq!(full(|_| {}, b"X = 1"), b"X = 1\n");
        assert_eq!(full(|_| {}, b"X = 1\n\n\n"), b"X = 1\n");
        assert_eq!(full(|_| {}, b"X = 1\r\n\r\n"), b"X = 1\r\n");
    }

    #[test]
    fn a_long_statement_is_wrapped_within_its_budget() {
        let source =
            b"program p\ntotal = alpha + beta + gamma + delta + epsilon + zeta + eta + theta\nend program p\n";
        let wrapped = full(|config| config.wrap.line_length = 40, source);
        let text = String::from_utf8_lossy(&wrapped).into_owned();
        for line in text.lines() {
            assert!(line.len() <= 40, "overlong line {line:?} in\n{text}");
        }
        assert!(text.contains(" &\n"), "no continuation produced:\n{text}");
        // The wrapped result is still a findent fixed point.
        let again = format_source(&wrapped, &FormatConfig::default())
            .unwrap()
            .bytes;
        assert_eq!(String::from_utf8_lossy(&again), text);
    }

    /// The four ways step 16 used to need a second run to settle. They share
    /// one shape: something the
    /// pipeline does *after* the wrapper measured the text — normalization
    /// widening it, the layout engine moving it, step 17 padding a `::` —
    /// pushed a line past the budget that the next run then rewrapped.
    #[test]
    fn statements_settle_on_the_first_run_when_later_passes_widen_them() {
        let cases: [&[u8]; 4] = [
            // Normalization adds the spaces around `//` that tip the joined
            // statement over the budget (`source/EstCovmat.f90`).
            b"module m\ncontains\nsubroutine s\n    if (Feedback >1 ) write(*,*) &\n     ' Parameter '//trim(BaseParams%UsedParamNameOrNumber(i))//' is weakly constrained, neglect correlations'\nend subroutine s\nend module m\n",
            // The layout engine moves the directive right, and the sentinel has
            // to be repeated on the wrapped line.
            b"module m\ncontains\nsubroutine s\ndo i = 1, n\ndo j = 1, n\n!$OMP PARALLEL DO DEFAULT(SHARED), SCHEDULE(STATIC), PRIVATE(zpeak, sigma_z, zpeakstart, zpeakend, nu_i, Win)\ndo k = 1, n\nx = 1\nend do\nend do\nend do\nend subroutine s\nend module m\n",
            // Step 17 gives `::` the space the wrapper had not paid for
            // because step 17 gives `::` its owed space.
            b"module m\ncontains\nsubroutine s\nreal (dl):: dif_old,dif,max,min,dlm,binz,m_min,m_max,mp,yp,zp,thp,xk1,xk2,xk3,yk1,yk2,yk3,fact,qmin,qmax,dlogy\nend subroutine s\nend module m\n",
            // A detached trailing comment above a dedented `else if`.
            b"module m\ncontains\nsubroutine s\nif (fb == zero) then\nxzero = b\nelseif (fa*(fb/abs(fb))<zero) then  ! check that f(ax) and f(bx) have different signs\nc = a\nend if\nend subroutine s\nend module m\n",
        ];
        for source in cases {
            for length in [80usize, 100, 120] {
                let once = full(|config| config.wrap.line_length = length, source);
                let twice = full(|config| config.wrap.line_length = length, &once);
                assert_eq!(
                    String::from_utf8_lossy(&once),
                    String::from_utf8_lossy(&twice),
                    "not a fixed point at {length} columns"
                );
            }
        }
    }

    #[test]
    fn project_case_does_not_make_wrapped_intrinsics_non_idempotent() {
        // The unrelated project declaration supplies project-wide `Size` evidence. The
        // target declaration uses the intrinsic twice in a dimension bound;
        // at a narrow budget the second occurrence becomes a continuation
        // fragment, which must retain the intrinsic's canonical lowercase
        // spelling across both runs.
        let target = b"module target\n\
implicit none\n\
contains\n\
subroutine s(x, i, j)\n\
real :: x\n\
real(ReallyLongKindName) :: LongJMat(size(x%element(i, j)%x), size(x%element(i, j)%x))\n\
end subroutine s\n\
end module target\n";
        let project_source = b"module project_names\n\
real :: Size\n\
end module project_names\n";
        let project = analyze_project([
            (Path::new("target.f90"), target.as_slice()),
            (Path::new("project_names.f90"), project_source.as_slice()),
        ])
        .unwrap();
        let config = FormatConfig {
            mode: FormatMode::Full,
            wrap: crate::config::WrapConfig {
                enabled: true,
                line_length: 80,
            },
            ..FormatConfig::default()
        };
        let once = format_with_context(target, &project, &config)
            .unwrap()
            .bytes;
        let twice = format_with_context(&once, &project, &config).unwrap().bytes;
        assert_eq!(twice, once);
        let output = String::from_utf8(once).unwrap();
        assert!(
            output.contains("LongJMat(size(x%element(i, j)%x), &\n"),
            "{output}"
        );
        assert!(output.contains("size(x%element(i, j)%x))"), "{output}");
    }

    /// An unwrappable statement keeps every physical line it came with.  The
    /// decline path used to emit the first line alone, which silently deleted
    /// the rest of the statement.
    #[test]
    fn a_declined_wrap_keeps_the_whole_statement() {
        let mut source = b"module m\ncontains\nsubroutine s\ncall f(a, '".to_vec();
        source.extend(std::iter::repeat_n(b'x', 150));
        source.extend_from_slice(b"', &\n    b)\nend subroutine s\nend module m\n");
        let once = full(|_| {}, &source);
        let text = String::from_utf8_lossy(&once).into_owned();
        assert!(text.contains("b)\n"), "continuation line dropped:\n{text}");
        let twice = full(|_| {}, &once);
        assert_eq!(text, String::from_utf8_lossy(&twice));
    }

    /// `/)` closes a FORMAT statement's edit-descriptor list; only an array
    /// constructor's `/)` becomes `]`.  On a continuation line there is no
    /// `format` keyword to see, so the statement-level fact has to be carried.
    #[test]
    fn a_continued_format_statement_keeps_its_slash_before_the_paren() {
        let source = b"module m\ncontains\nsubroutine s\n9060 format ('    NXD =', i5, ',  NYD =', i5, ',  NXI =', i5, &\n    ',  NYI =', i5 /)\nend subroutine s\nend module m\n";
        let once = full(|_| {}, source);
        let text = String::from_utf8_lossy(&once).into_owned();
        assert!(
            text.contains("i5 /)"),
            "format descriptor rewritten:\n{text}"
        );
        assert!(
            !text.contains("i5]"),
            "format descriptor rewritten:\n{text}"
        );
    }

    #[test]
    fn normalize_only_mode_leaves_every_column_untouched() {
        let source = b"program p\n        X = 1\nend program p\n";
        let normalized = full(|config| config.mode = FormatMode::NormalizeOnly, source);
        assert_eq!(
            String::from_utf8_lossy(&normalized),
            String::from_utf8_lossy(source)
        );
    }

    #[test]
    fn generated_wrapping_stress_cases_are_fixed_points_and_fit_safe_breaks() {
        let sources = [
            br#"program p
             real :: values(1), weights(2), alpha, beta, gamma, delta
             call compute(alpha, beta, gamma, delta, nested(first_value, second_value, third_value), named=value)
             result_value = alpha + beta + gamma + delta + epsilon + zeta + eta + theta + iota + kappa
             end program p
             "# as &[u8],
            br#"program p
             real :: values(1), weights(2), alpha, beta, &
             & gamma, delta
             call compute(alpha, beta, gamma, delta, nested(first_value, second_value, &
             & third_value), named=value)
             result_value = alpha + beta + gamma + delta + epsilon + zeta + eta + theta + iota + kappa
             end program p
             "# as &[u8],
        ];
        for source in sources {
            for line_length in [60, 80, 100, 120] {
                for align in [false, true] {
                    for continuation in [0, 3, 9] {
                        let config = FormatConfig {
                            mode: FormatMode::Full,
                            wrap: crate::config::WrapConfig {
                                enabled: true,
                                line_length,
                            },
                            align_paren: align,
                            align_paren_value: usize::from(align),
                            continuation_indent: continuation,
                            ..FormatConfig::default()
                        };
                        let once = format_with_context(source, &ProjectContext::empty(), &config)
                            .unwrap()
                            .bytes;
                        let twice = format_with_context(&once, &ProjectContext::empty(), &config)
                            .unwrap()
                            .bytes;
                        assert_eq!(
                            once, twice,
                            "not idempotent at {line_length}/{align}/{continuation}"
                        );
                        let mut indent_only = config.clone();
                        indent_only.mode = FormatMode::IndentOnly;
                        let indented = crate::format_source(&once, &indent_only).unwrap().bytes;
                        assert_eq!(
                            once, indented,
                            "I2 failed at {line_length}/{align}/{continuation}"
                        );
                        for line in once.split(|byte| *byte == b'\n') {
                            if line.len() <= line_length || line.iter().all(u8::is_ascii_whitespace)
                            {
                                continue;
                            }
                            let text = line.trim_ascii_start();
                            assert!(
                                text.starts_with(b"!") || text.starts_with(b"#"),
                                "generated code line exceeded {line_length}: {:?}",
                                String::from_utf8_lossy(line)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn openmp_wrapping_repeats_the_sentinel_and_keeps_macro_case() {
        let source = b"!$OMP PARALLEL DO DEFAULT(SHARED), private(worker), SCHEDULE(STATIC), REDUCTION(+:total)\n";
        let mut project = ProjectContext::empty();
        project.define(&[crate::config::MacroDefine {
            name: "private".into(),
            value: None,
        }]);
        let config = FormatConfig {
            mode: FormatMode::Full,
            wrap: crate::config::WrapConfig {
                enabled: true,
                line_length: 42,
            },
            ..FormatConfig::default()
        };
        let output = format_with_context(source, &project, &config)
            .unwrap()
            .bytes;
        for line in output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            assert!(line.starts_with(b"!$OMP"), "invalid sentinel: {line:?}");
            assert!(line.len() <= 42, "overlong OpenMP line: {line:?}");
        }
        assert!(output
            .windows(b"PRIVATE".len())
            .all(|window| window != b"PRIVATE"));
        assert!(output
            .windows(b"private".len())
            .any(|window| window == b"private"));
    }
}
