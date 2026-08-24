//! Macro-name casing (steps 1-3).

use crate::{
    error::FormatError,
    source::{
        tokens::{tokenize, TokenKind},
        LexState, PhysicalLineKind,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::provenance::{source_spans, spread_replacement},
        pipeline::{Changed, PassContext},
    },
};

/// Apply the spelling of every known macro name.
///
/// Sources of macro names, in collection order, are command-line `-D` names
/// and project `#define` names. Directive bodies and quoted text stay protected.
pub(crate) fn macros(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    if cx.project.macros.is_empty() {
        return Ok(Changed::No);
    }

    let mut state = LexState::default();
    let mut changed = Changed::No;
    for (line_index, line) in document.lines.iter_mut().enumerate() {
        let kind = cx
            .analysis
            .buffer
            .lines
            .get(line_index)
            .map(|physical| physical.kind)
            .unwrap_or(PhysicalLineKind::Code);
        if kind == PhysicalLineKind::Preprocessor {
            state = LexState::default();
            continue;
        }
        let tokens = tokenize(line, &mut state);
        let mut edits = EditBuffer::new(line);
        for token in tokens {
            if token.kind != TokenKind::Name || !cx.project.macros.contains(token.text) {
                continue;
            }
            if let Some(spelling) = cx.project.macros.get(token.text) {
                edits.replace(token.span, spelling);
            }
        }
        let updated = edits.finish();
        if updated != *line {
            *line = updated;
            changed = changed.or(Changed::Text);
        }
    }

    changed = changed.or(crossing_macro_names(document, cx));
    Ok(changed)
}

/// Apply macro spellings to names split across authored continuation lines.
fn crossing_macro_names(document: &mut Document, cx: &PassContext) -> Changed {
    let mut changed = Changed::No;
    for group in &cx.analysis.groups {
        if group.lines.len() < 2 {
            continue;
        }
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            for token in &tokens {
                if token.kind != TokenKind::Name {
                    continue;
                }
                let Some(spelling) = cx.project.macros.get(token.text) else {
                    continue;
                };
                if spelling == token.text {
                    continue;
                }
                let spans = source_spans(group, statement, token);
                if spans.len() < 2 {
                    continue;
                }
                let Some(pieces) = spread_replacement(&spans, token, spelling) else {
                    continue;
                };
                for (line, span, piece) in pieces {
                    let line_start = cx.analysis.buffer.lines[line].span.start as usize;
                    let source = &document.lines[line];
                    let mut buffer = EditBuffer::new(source);
                    buffer.replace(span.start - line_start..span.end - line_start, piece);
                    let updated = buffer.finish();
                    if updated != *source {
                        document.lines[line] = updated;
                        changed = changed.or(Changed::Text);
                    }
                }
            }
        }
    }
    changed
}
