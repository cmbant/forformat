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
    analysis::{analyze_file, ProjectContext},
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
        let declined = reflow(&mut document, config)?;
        // Every long line the wrapper refuses is explainable; the corpus check
        // consumes this to separate "unwrappable by design" from a wrapper bug.
        let _ = declined;
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
    let analysis = document.analyze()?;
    let mut planner = Planner::new(config);
    let mut plans = Vec::with_capacity(analysis.groups.len());
    for group in &analysis.groups {
        plans.push(planner.plan(&analysis.buffer, group, config));
    }

    let mut lines: Vec<Vec<u8>> = Vec::with_capacity(document.lines.len());
    let mut declined = Vec::new();
    for (group, plan) in analysis.groups.iter().zip(&plans) {
        let PlanBody::Code {
            first_indent,
            group_first_cont,
            ..
        } = plan.body
        else {
            copy_group(document, group, &mut lines);
            continue;
        };
        if !eligible(&analysis.buffer, group) || group_first_cont && group.lines.len() > 1 {
            // A statement that is already continued is joined and re-wrapped by
            // a later task; until then it is passed through untouched.
            copy_group(document, group, &mut lines);
            continue;
        }
        let index = group.lines.start;
        let body = trim(&document.lines[index]);
        let layout = ContinuationLayout {
            first_indent,
            continuation: first_indent.saturating_add(if config.indent_continuation {
                config.continuation_indent
            } else {
                0
            }),
        };
        match wrapping::wrap_body(body, layout, config.wrap.line_length) {
            Ok(wrapped) => lines.extend(wrapped),
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
}
