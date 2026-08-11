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
        let declined = reflow_with_context(&mut document, project, &local, config)?;
        // Every long line the wrapper refuses is explainable; the corpus check
        // consumes this to separate "unwrappable by design" from a wrapper bug.
        let laid_out = engine::format(&document.to_lf_bytes(), config)?;
        let mut output = Document::from_bytes(&laid_out.bytes);
        output.newline = document.newline;
        output.trailing_newline = document.trailing_newline;
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
                copy_group(document, group, &mut lines);
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
            body = crate::transform::passes::line_rules::respace_joined(
                &body,
                &context,
                &declared_names,
                index,
            );
            body = trim(&body).to_vec();
        }
        if first_indent + body.len() <= config.wrap.line_length {
            copy_group(document, group, &mut lines);
            continue;
        }
        let detached = detach_final_inline_comment(document, group);
        if detached.is_none() {
            copy_group(document, group, &mut lines);
            continue;
        }
        let layout = ContinuationLayout {
            first_indent,
            continuation: first_indent.saturating_add(if config.indent_continuation {
                config.continuation_indent
            } else {
                0
            }),
        };
        match wrapping::wrap_body_with_alignment(&body, layout, config.wrap.line_length, align) {
            Ok(wrapped) => {
                if let Some(comment) = detached.flatten() {
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
    Ok(declined)
}

/// Return the final inline comment, if there is exactly one and it is on the
/// last physical line of the group. `None` means the group is unsafe to detach;
/// `Some(None)` means it has no inline comment.
fn detach_final_inline_comment(
    document: &Document,
    group: &LogicalGroup,
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
    let mut comment = line[..start]
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .copied()
        .collect::<Vec<_>>();
    comment.extend_from_slice(line[start..].trim_ascii_start());
    Some(Some(vec![comment]))
}

fn join_openmp_directive(document: &Document, group: &LogicalGroup) -> Option<Vec<u8>> {
    let mut parts = Vec::new();
    let mut omp_style = false;
    for (position, index) in group.lines.clone().enumerate() {
        let line = &document.lines[index];
        let start = line.iter().position(|byte| !byte.is_ascii_whitespace())?;
        if !line[start..].starts_with(b"!$") {
            return None;
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
    let mut joined = if omp_style {
        b"!$OMP ".to_vec()
    } else {
        b"!$ ".to_vec()
    };
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            joined.push(b' ');
        }
        joined.extend_from_slice(part);
    }
    Some(joined)
}

fn wrap_openmp_directive(line: &[u8], line_length: usize) -> Result<Vec<Vec<u8>>, Decline> {
    let prefix: &[u8] = if line
        .get(2..5)
        .is_some_and(|bytes| bytes.eq_ignore_ascii_case(b"omp"))
    {
        b"!$OMP "
    } else {
        b"!$ "
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
        let mut physical = prefix.to_vec();
        physical.extend_from_slice(trim(&body[..position]));
        physical.extend_from_slice(b" &");
        result.push(physical);
        body = trim(&body[position..]).to_vec();
    }
    let mut last = prefix.to_vec();
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
        analysis::ProjectContext,
        config::{FormatConfig, FormatMode},
        format_source,
    };

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
            "program p\r\n   X = 1\r\nend program p\r\n"
        );
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
