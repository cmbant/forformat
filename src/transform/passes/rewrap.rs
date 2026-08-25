//! Opt-in preparation for rewrapping authored continuations.
//!
//! The full wrapper is intentionally the only implementation that chooses
//! final break points. `--rewrap` merely removes safe authored breaks first:
//! a continued statement is joined into one logical line when the existing
//! wrapper can either keep that joined line within budget or find a safe fresh
//! wrapping. The normal full-mode fixed-point reflow then measures layout and
//! post-layout widths exactly as it already does.

use crate::{
    analysis::scoped_declared_names,
    error::FormatError,
    format::{
        planner::{PlanBody, Planner},
        wrapping::{self, ContinuationLayout, Decline},
    },
    source::PhysicalLineKind,
    transform::{
        document::Document,
        passes::line_rules,
        pipeline::{Changed, PassContext},
    },
};

pub fn prepare(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    prepare_inner(document, cx, false)
}

/// Rejoin wrapper-generated continuations for an internal fixed-point round.
///
/// The ordinary pipeline follows [`prepare`] with the physical line-rule pass.
/// An internal round must not rerun that pass over every unrelated line while
/// holding the previous project snapshot, because project casing is a
/// whole-checkout decision. The joined statement has already been normalized
/// by `respace_joined` for the safety probe, so this variant emits exactly that
/// spelling and leaves unrelated lines byte-identical.
pub(crate) fn prepare_settlement(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    prepare_inner(document, cx, true)
}

fn prepare_inner(
    document: &mut Document,
    cx: &PassContext,
    emit_measured: bool,
) -> Result<Changed, FormatError> {
    let config = cx.config;
    let analysis = cx.analysis;
    let declared_names = scoped_declared_names(analysis, cx.scopes);
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
                    line.kind == PhysicalLineKind::Code && !line.omp && line.comment_span.is_none()
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

        // The logical statement assembler preserves whitespace from both sides
        // of an authored continuation seam. The full wrapper does not measure
        // those raw bytes: it first applies the joined-statement rule subset.
        // Probe the same spelling here so `--rewrap` cannot reject a join that
        // the wrapper would accept (or accept one the wrapper would reject)
        // merely because the removed seam temporarily contains extra spaces.
        let measured_body =
            line_rules::respace_joined(&body, cx, &declared_names, group.lines.start);
        let measured_body = measured_body.trim_ascii();

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
            wrapping::wrap_body_with_alignment(
                measured_body,
                layout,
                config.wrap.line_length,
                align,
            ),
            Ok(_) | Err(Decline::Fits)
        );
        if !safe {
            copy_group(document, group.lines.clone(), &mut lines);
            continue;
        }

        // The ordinary pass stays structural: the physical line-rule pass that
        // follows owns user-visible seam normalization. An internal settlement
        // round cannot safely rerun that whole-file pass against a project
        // snapshot from before the first round, so it emits the same measured
        // joined spelling directly and touches no unrelated line.
        let first = &document.lines[group.lines.start];
        let indent_end = first
            .iter()
            .position(|byte| !matches!(byte, b' ' | b'\t'))
            .unwrap_or(first.len());
        let mut joined = first[..indent_end].to_vec();
        joined.extend_from_slice(if emit_measured { measured_body } else { &body });
        lines.push(joined);
        changed = Changed::Structure;
    }

    if changed == Changed::Structure {
        document.set_lines(lines);
    }
    Ok(changed)
}

fn copy_group(document: &Document, range: std::ops::Range<usize>, output: &mut Vec<Vec<u8>>) {
    for index in range {
        if let Some(line) = document.lines.get(index) {
            output.push(line.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::prepare;
    use crate::{
        analysis::{analyze_file, ProjectContext, ScopeTree},
        config::FormatConfig,
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
    };

    fn run_prepare(document: &mut Document, config: &FormatConfig) -> Changed {
        let project = ProjectContext::empty();
        let source = document.to_lf_bytes();
        let local = analyze_file(&source).unwrap();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        prepare(document, &context).unwrap()
    }

    #[test]
    fn fitting_authored_continuation_is_joined_for_fresh_layout() {
        let mut document = Document::from_bytes(b"call work(alpha, &\n    beta)\n");
        let config = FormatConfig::default();
        assert_eq!(run_prepare(&mut document, &config), Changed::Structure);
        // Preparation removes the authored break but does not take ownership
        // of the whitespace at the seam; the following physical line-rule pass
        // normalizes that user-visible spelling.
        assert_eq!(document.lines, [b"call work(alpha,  beta)".to_vec()]);
    }

    #[test]
    fn comments_keep_authored_continuations() {
        let mut document = Document::from_bytes(b"call work(alpha, & ! note\n    beta)\n");
        let original = document.clone();
        let config = FormatConfig::default();
        assert_eq!(run_prepare(&mut document, &config), Changed::No);
        assert_eq!(document, original);
    }
}
