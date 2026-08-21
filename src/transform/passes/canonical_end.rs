//! Scope-aware END completion for canonicalization-only formatting.
//!
//! Full mode obtains `--refactor-end` replacements from the layout planner.
//! Canonicalization-only deliberately does not run layout, but it still needs
//! the same scope-aware answer. Reuse the planner to decide *what* the END says,
//! then splice only that code spelling back into the authored physical line so
//! indentation, trailing horizontal whitespace, comment spacing, and line
//! endings remain outside this pass's ownership.

use crate::{
    config::FormatConfig,
    error::FormatError,
    format::planner::{PlanBody, Planner},
    transform::{document::Document, pipeline::Changed},
};

pub fn run(document: &mut Document, config: &FormatConfig) -> Result<Changed, FormatError> {
    if !config.refactor_end {
        return Ok(Changed::No);
    }

    let analysis = document.analyze()?;
    let mut planner = Planner::new(config);
    let mut replacements = Vec::new();

    for group in &analysis.groups {
        let plan = planner.plan(&analysis.buffer, group, config);
        let PlanBody::Code {
            replacement: Some(replacement),
            ..
        } = plan.body
        else {
            continue;
        };

        let line_index = group.lines.start;
        let physical = &analysis.buffer.lines[line_index];
        let line_start = physical.span.start as usize;
        let code_start = physical.code_span.start as usize - line_start;
        let code_end = physical.code_span.end as usize - line_start;

        let replacement_end =
            crate::source::buffer::comment_start(&replacement).unwrap_or(replacement.len());
        let replacement_code = replacement[..replacement_end].trim_ascii_end().to_vec();
        replacements.push((line_index, code_start, code_end, replacement_code));
    }

    let mut changed = Changed::No;
    for (line_index, code_start, code_end, replacement) in replacements {
        let line = &document.lines[line_index];
        let mut authored_code_end = code_end;
        while authored_code_end > code_start && matches!(line[authored_code_end - 1], b' ' | b'\t')
        {
            authored_code_end -= 1;
        }

        let mut updated = Vec::with_capacity(
            line.len()
                + replacement
                    .len()
                    .saturating_sub(authored_code_end - code_start),
        );
        updated.extend_from_slice(&line[..code_start]);
        updated.extend_from_slice(&replacement);
        updated.extend_from_slice(&line[authored_code_end..]);
        if updated != *line {
            document.lines[line_index] = updated;
            changed = changed.or(Changed::Text);
        }
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::{config::FormatConfig, transform::document::Document};

    #[test]
    fn completion_keeps_authored_indent_and_comment_gap() {
        let mut document = Document::from_bytes(b"module M\n\tend   ! note\n");
        let config = FormatConfig {
            refactor_end: true,
            ..FormatConfig::default()
        };
        run(&mut document, &config).unwrap();
        assert_eq!(document.to_bytes(), b"module M\n\tend module M   ! note\n");
    }
}
