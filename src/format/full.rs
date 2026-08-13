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

type DetachedComment = (usize, usize);
type ReflowResult = (Vec<(usize, Decline)>, Vec<DetachedComment>);

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
        let (declined, detached_comments) =
            reflow_with_context_inner(&mut document, project, &local, config)?;
        // Every long line the wrapper refuses is explainable; the corpus check
        // consumes this to separate "unwrappable by design" from a wrapper bug.
        let laid_out = engine::format(&document.to_lf_bytes(), config)?;
        let mut output = Document::from_bytes(&laid_out.bytes);
        output.newline = document.newline;
        output.trailing_newline = document.trailing_newline;
        restore_detached_comment_indentation(&mut output, &detached_comments);
        restore_overindented_comment_lines(&document, &mut output, config.wrap.line_length);
        pipeline::post_layout(&mut output, config)?;
        return Ok(FormatResult {
            bytes: output.to_bytes(),
            meta: FormatMeta {
                last_indent: laid_out.meta.last_indent,
                last_usable: laid_out.meta.last_usable,
                declines: declined,
            },
        });
    }

    // The layout engine owns every column.  It runs over LF text and its output
    // is re-wrapped into the document's terminator policy at the end.
    let laid_out = engine::format(&document.to_lf_bytes(), config)?;
    let mut output = Document::from_bytes(&laid_out.bytes);
    output.newline = document.newline;
    output.trailing_newline = document.trailing_newline;
    pipeline::post_layout(&mut output, config)?;

    Ok(FormatResult {
        bytes: output.to_bytes(),
        meta: laid_out.meta,
    })
}

/// Step 16: reflow statements that overrun the budget.
///
/// The first-line indent and the continuation column both come from the layout
/// plan, so a user who changes `-k` or turns on `--align-paren` changes where
/// wrapped lines start *and* the width the wrapper had to work with, together.
/// A literal `indent + 4` here — which is what the reference formatter uses —
/// would silently disagree with the engine the moment either option moved.
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
    Ok(reflow_with_context_inner(document, project, local, config)?.0)
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

    let mut lines: Vec<Vec<u8>> = Vec::with_capacity(document.lines.len());
    let mut declined = Vec::new();
    let mut detached_comments = Vec::new();
    for (group, plan) in analysis.groups.iter().zip(&plans) {
        if let Some(directive) = join_openmp_directive(document, group) {
            let long = directive.len() > config.wrap.line_length
                || group
                    .lines
                    .clone()
                    .any(|index| document.lines[index].len() > config.wrap.line_length);
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
        let has_long_physical_line = group
            .lines
            .clone()
            .any(|line| document.lines[line].len() > config.wrap.line_length);
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
        let layout = ContinuationLayout {
            first_indent,
            continuation: first_indent.saturating_add(if config.indent_continuation {
                config.continuation_indent
            } else {
                0
            }),
        };
        // A detached trailing comment belongs to the statement as a whole,
        // not to its last continuation line.  Keeping it at the statement
        // indent also makes a wrapped statement stable on the next pass.
        let comment_indent = layout.first_indent;
        let detached = detach_final_inline_comment(document, group, comment_indent);
        if first_indent + body.len() <= config.wrap.line_length {
            match detached {
                Some(Some(comment)) => {
                    detached_comments.push((lines.len(), comment_indent));
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
        match wrapping::wrap_body_with_alignment(&body, layout, config.wrap.line_length, align) {
            Ok(wrapped) => {
                if let Some(comment) = detached.flatten() {
                    detached_comments.push((lines.len(), comment_indent));
                    lines.extend(comment);
                }
                lines.extend(wrapped)
            }
            Err(Decline::Fits) => lines.push(document.lines[index].clone()),
            Err(reason) => {
                declined.push((index, reason));
                lines.push(document.lines[index].clone());
            }
        }
    }
    document.set_lines(lines);
    Ok((declined, detached_comments))
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

fn restore_detached_comment_indentation(document: &mut Document, comments: &[DetachedComment]) {
    for &(index, indent) in comments {
        let Some(line) = document.lines.get_mut(index) else {
            continue;
        };
        let content = line.trim_ascii_start().to_vec();
        *line = vec![b' '; indent];
        line.extend_from_slice(&content);
    }
}

fn restore_overindented_comment_lines(
    source: &Document,
    output: &mut Document,
    line_length: usize,
) {
    for (source_line, output_line) in source.lines.iter().zip(&mut output.lines) {
        let source_indent = leading_horizontal_width(source_line);
        let source_content = source_line.trim_ascii_start();
        if source_indent == 0
            || source_line.len() <= line_length
            || !source_content.starts_with(b"!")
        {
            continue;
        }
        let output_content = output_line.trim_ascii_start().to_vec();
        if !output_content.starts_with(b"!") {
            continue;
        }
        if source_indent > leading_horizontal_width(output_line) {
            *output_line = vec![b' '; source_indent];
            output_line.extend_from_slice(&output_content);
        }
    }
}

fn leading_horizontal_width(line: &[u8]) -> usize {
    line.iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count()
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
