//! Scope-aware reconciliation for the declared-case pass.
//!
//! `case_pass` remains the spelling engine. While it runs, it records the
//! evidence class behind each identifier decision. This wrapper only revisits
//! evidence that came from compatibility/project-wide tables; semantic answers
//! from the base pass are authoritative by default and therefore cannot be
//! discarded merely because a new token shape was not added here.

use crate::{
    analysis::{project::ResolvedType, scoped_declared_names},
    error::FormatError,
    source::{
        tokens::{tokenize, TokenKind},
        LexState,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::{
            case_pass::{self, CaseEvidence},
            provenance::{source_spans, spread_replacement},
        },
        pipeline::{Changed, PassContext},
    },
};
use std::ops::Range;

pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let (mut changed, evidence) =
        case_pass::declared_with_names_and_evidence(document, cx, &declared_names)?;
    let mut line_edits: Vec<Vec<(Range<usize>, Vec<u8>)>> = vec![Vec::new(); document.lines.len()];

    for group in &cx.analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            for token in &tokens {
                if token.kind != TokenKind::Name {
                    continue;
                }
                let spans = source_spans(group, statement, token);
                let Some((line, first_span)) = spans.first() else {
                    continue;
                };
                let Some(evidence) = evidence.get(&(*line, first_span.start)) else {
                    continue;
                };
                let replacement = match scoped_spelling(evidence, token.text, *line, cx) {
                    Decision::KeepBase => continue,
                    Decision::Replace(spelling) => spelling,
                    Decision::Restore => token.text.to_vec(),
                };
                let Some(pieces) = spread_replacement(&spans, token, &replacement) else {
                    continue;
                };
                for (source_line, span, piece) in pieces {
                    let line_start = cx.analysis.buffer.lines[source_line].span.start as usize;
                    line_edits[source_line].push((
                        span.start - line_start..span.end - line_start,
                        piece.to_vec(),
                    ));
                }
            }
        }
    }

    for (line, edits) in line_edits.into_iter().enumerate() {
        if edits.is_empty() {
            continue;
        }
        let source = &document.lines[line];
        let mut buffer = EditBuffer::new(source);
        for (span, replacement) in edits {
            buffer.replace(span, &replacement);
        }
        let updated = buffer.finish();
        if updated != *source {
            document.lines[line] = updated;
            changed = changed.or(Changed::Text);
        }
    }

    Ok(changed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    KeepBase,
    Replace(Vec<u8>),
    Restore,
}

fn scoped_spelling(
    evidence: &CaseEvidence,
    name: &[u8],
    line: usize,
    cx: &PassContext,
) -> Decision {
    match evidence {
        CaseEvidence::KeepBase => Decision::KeepBase,
        CaseEvidence::Alias(spelling) => Decision::Replace(spelling.clone()),
        CaseEvidence::UseRemote { module } => cx
            .project
            .visible_use_symbol_spelling(module, name)
            .map(Decision::Replace)
            .unwrap_or(Decision::Restore),
        CaseEvidence::Type => cx
            .project
            .visible_type_spelling(cx.local, line, name)
            .map(Decision::Replace)
            .unwrap_or(Decision::Restore),
        CaseEvidence::Member { owner } => scoped_member_spelling(owner, name, line, cx),
        CaseEvidence::Symbol { allow_external } => {
            if let Some(spelling) = cx.project.visible_symbol_spelling(cx.local, line, name) {
                return Decision::Replace(spelling);
            }
            if *allow_external {
                if let Some(spelling) = cx.project.external_symbol_spelling(name) {
                    return Decision::Replace(spelling);
                }
            }
            Decision::Restore
        }
    }
}

fn scoped_member_spelling(
    names: &[Vec<u8>],
    name: &[u8],
    line: usize,
    cx: &PassContext,
) -> Decision {
    let Some(root) = names.first() else {
        return Decision::KeepBase;
    };
    let Some(current) = cx.project.visible_variable_type(cx.local, line, root) else {
        return Decision::Restore;
    };
    let Some(owner) = resolve_component_owner(current, &names[1..], cx) else {
        return Decision::Restore;
    };
    cx.project
        .visible_member_spelling(cx.local, &owner, name)
        .map(Decision::Replace)
        .unwrap_or(Decision::Restore)
}

fn resolve_component_owner(
    mut current: ResolvedType,
    links: &[Vec<u8>],
    cx: &PassContext,
) -> Option<ResolvedType> {
    for link in links {
        current = cx
            .project
            .visible_component_type(cx.local, &current, link)?;
    }
    Some(current)
}
