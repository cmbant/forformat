//! Opt-in preparation for rewrapping authored continuations.
//!
//! The full wrapper is intentionally the only implementation that chooses
//! final break points. `--rewrap` merely removes safe authored breaks first:
//! a continued statement is joined into one logical line when the existing
//! wrapper can either keep that joined line within budget or find a safe fresh
//! wrapping. The normal full-mode fixed-point reflow then measures layout and
//! post-layout widths exactly as it already does.

use crate::{
    config::FormatConfig,
    error::FormatError,
    format::{
        planner::{PlanBody, Planner},
        wrapping::{self, ContinuationLayout, Decline},
    },
    source::PhysicalLineKind,
    transform::{document::Document, pipeline::Changed},
};

pub fn prepare(document: &mut Document, config: &FormatConfig) -> Result<Changed, FormatError> {
    let analysis = document.analyze()?;
    let mut planner = Planner::new(config);
    let plans = analysis
        .groups
        .iter()
        .map(|group| planner.plan(&analysis.buffer, group, config))
        .collect::<Vec<_>>();

    let mut lines = Vec::with_capacity(document.lines.len());
    let mut changed = Changed::No;

    for (group, plan) in analysis.groups.iter().zip(&plans) {
        let PlanBody::Code {
            first_indent,
            align,
            replacement: None,
            ..
        } = plan.body
        else {
            copy_group(document, group.lines.clone(), &mut lines);
            continue;
        };

        let safe_shape = group.lines.len() > 1
            && group.statements.len() == 1
            && group.lines.clone().all(|index| {
                analysis.buffer.lines.get(index).is_some_and(|line| {
                    line.kind == PhysicalLineKind::Code
                        && !line.omp
                        && line.comment_span.is_none()
                })
            });
        if !safe_shape {
            copy_group(document, group.lines.clone(), &mut lines);
            continue;
        }

        let body = group.statements[0].text.trim_ascii().to_vec();
        if body.is_empty() || crate::format::emitter::split_label(&body).is_some() {
            copy_group(document, group.lines.clone(), &mut lines);
            continue;
        }

        let continuation = first_indent.saturating_add(if config.indent_continuation {
            config.continuation_indent
        } else {
            0
        });
        let layout = ContinuationLayout {
            first_indent,
            continuation,
        };
        let safe = matches!(
            wrapping::wrap_body_with_alignment(&body, layout, config.wrap.line_length, align),
            Ok(_) | Err(Decline::Fits)
        );
        if !safe {
            copy_group(document, group.lines.clone(), &mut lines);
            continue;
        }

        let first = &document.lines[group.lines.start];
        let indent_end = first
            .iter()
            .position(|byte| !matches!(byte, b' ' | b'\t'))
            .unwrap_or(first.len());
        let mut joined = first[..indent_end].to_vec();
        joined.extend_from_slice(&body);
        lines.push(joined);
        changed = Changed::Structure;
    }

    if changed == Changed::Structure {
        document.set_lines(lines);
    }
    Ok(changed)
}

fn copy_group(
    document: &Document,
    range: std::ops::Range<usize>,
    output: &mut Vec<Vec<u8>>,
) {
    for index in range {
        if let Some(line) = document.lines.get(index) {
            output.push(line.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::prepare;
    use crate::{config::FormatConfig, transform::document::Document};

    #[test]
    fn fitting_authored_continuation_is_joined_for_fresh_layout() {
        let mut document = Document::from_bytes(b"call work(alpha, &\n    beta)\n");
        let config = FormatConfig::default();
        assert_eq!(prepare(&mut document, &config).unwrap(), crate::transform::pipeline::Changed::Structure);
        assert_eq!(document.lines, [b"call work(alpha, beta)".to_vec()]);
    }

    #[test]
    fn comments_keep_authored_continuations() {
        let mut document = Document::from_bytes(b"call work(alpha, & ! note\n    beta)\n");
        let original = document.clone();
        let config = FormatConfig::default();
        assert_eq!(prepare(&mut document, &config).unwrap(), crate::transform::pipeline::Changed::No);
        assert_eq!(document, original);
    }
}
