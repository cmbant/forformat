//! Scope-aware reconciliation for the declared-case pass.
//!
//! `case_pass` remains the spelling engine. Scoped decisions are applied while
//! that pass still owns the token and its base spelling, so the project-aware
//! correction does not need a retained evidence map or a second token walk.

use crate::{
    analysis::{
        is_implicit_letter_name, project::ResolvedType, scoped_declared_names, DeclaredNameIndex,
        DeclaredSpelling,
    },
    error::FormatError,
    source::{tokens::tokenize, LexState},
    transform::{
        document::Document,
        passes::{
            case_pass::{self, CaseEvidence, CaseReconciler, Reconciliation},
            provenance::source_spans,
        },
        pipeline::{Changed, PassContext},
    },
};
use std::ops::Range;

pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let protected = implicit_letter_spellings(cx);
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let mut reconciler = ScopedReconciler {
        cx,
        declared_names: &declared_names,
    };
    let changed = case_pass::declared_with_names_and_reconciler(
        document,
        cx,
        &declared_names,
        &mut reconciler,
    )?;
    restore_implicit_letter_spellings(document, &protected);
    Ok(changed)
}

struct ProtectedSpelling {
    line: usize,
    range: Range<usize>,
    spelling: Vec<u8>,
}

fn implicit_letter_spellings(cx: &PassContext) -> Vec<ProtectedSpelling> {
    let mut protected = Vec::new();
    for group in &cx.analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            for (index, token) in tokens.iter().enumerate() {
                if !is_implicit_letter_name(&tokens, index) {
                    continue;
                }
                let spans = source_spans(group, statement, token);
                let mut taken = 0;
                for (line, span) in spans {
                    let line_start = cx.analysis.buffer.lines[line].span.start as usize;
                    let len = span.len();
                    protected.push(ProtectedSpelling {
                        line,
                        range: span.start - line_start..span.end - line_start,
                        spelling: token.text[taken..taken + len].to_vec(),
                    });
                    taken += len;
                }
            }
        }
    }
    protected
}

fn restore_implicit_letter_spellings(document: &mut Document, protected: &[ProtectedSpelling]) {
    for item in protected {
        let Some(line) = document.lines.get_mut(item.line) else {
            continue;
        };
        if item.range.end <= line.len() && line[item.range.clone()] != item.spelling[..] {
            line.splice(item.range.clone(), item.spelling.iter().copied());
        }
    }
}

struct ScopedReconciler<'a, 'cx> {
    cx: &'a PassContext<'cx>,
    declared_names: &'a DeclaredNameIndex,
}

impl CaseReconciler for ScopedReconciler<'_, '_> {
    const ENABLED: bool = true;

    fn reconcile(&mut self, evidence: &CaseEvidence, name: &[u8], line: usize) -> Reconciliation {
        scoped_spelling(evidence, name, line, self.cx, self.declared_names)
    }
}

fn scoped_spelling(
    evidence: &CaseEvidence,
    name: &[u8],
    line: usize,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
) -> Reconciliation {
    match evidence {
        CaseEvidence::KeepBase => Reconciliation::KeepBase,
        CaseEvidence::Alias(spelling) => Reconciliation::Replace(spelling.clone()),
        CaseEvidence::UseRemote { module } => cx
            .project
            .visible_use_symbol_spelling(module, name)
            .map(Reconciliation::Replace)
            .unwrap_or(Reconciliation::Restore),
        CaseEvidence::Type => cx
            .project
            .visible_type_spelling(cx.local, line, name)
            .map(Reconciliation::Replace)
            .unwrap_or(Reconciliation::Restore),
        CaseEvidence::Member {
            owner,
            resolved_owner,
        } => scoped_member_spelling(owner, resolved_owner.as_ref(), name, line, cx),
        CaseEvidence::Symbol { allow_external } => {
            match declared_names.file_declared_case(line, name) {
                DeclaredSpelling::Spelling(spelling) => {
                    return Reconciliation::Replace(spelling.to_vec());
                }
                DeclaredSpelling::Ambiguous => return Reconciliation::Restore,
                DeclaredSpelling::Absent => {}
            }
            if let Some(spelling) = cx.project.visible_symbol_spelling(cx.local, line, name) {
                return Reconciliation::Replace(spelling);
            }
            if *allow_external {
                if let Some(spelling) = cx.project.external_symbol_spelling(name) {
                    return Reconciliation::Replace(spelling);
                }
            }
            Reconciliation::Restore
        }
    }
}

fn scoped_member_spelling(
    names: &[Vec<u8>],
    resolved_owner: Option<&ResolvedType>,
    name: &[u8],
    line: usize,
    cx: &PassContext,
) -> Reconciliation {
    let owner = if let Some(owner) = resolved_owner {
        owner.clone()
    } else {
        let Some(root) = names.first() else {
            return Reconciliation::KeepBase;
        };
        let Some(current) = cx.project.visible_variable_type(cx.local, line, root) else {
            return Reconciliation::Restore;
        };
        let Some(owner) = resolve_component_owner(current, &names[1..], line, cx) else {
            return Reconciliation::Restore;
        };
        owner
    };
    cx.project
        .visible_member_spelling(cx.local, line, &owner, name)
        .map(Reconciliation::Replace)
        .unwrap_or(Reconciliation::Restore)
}

fn resolve_component_owner(
    mut current: ResolvedType,
    links: &[Vec<u8>],
    line: usize,
    cx: &PassContext,
) -> Option<ResolvedType> {
    for link in links {
        current = cx
            .project
            .visible_component_type(cx.local, line, &current, link)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::declared;
    use crate::{
        analysis::{analyze_file, analyze_project, ScopeTree},
        config::{FormatConfig, FormatMode},
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
    };
    use std::path::Path;

    #[test]
    fn restore_of_split_identifier_emits_no_edit() {
        let declarations =
            b"module unrelated\ntype :: CamelType\nend type CamelType\nend module unrelated\n";
        let target = b"program p\ntype(CAM&\n&ELTYPE) :: value\nend program p\n";
        let project = analyze_project([
            (Path::new("unrelated.f90"), declarations.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ])
        .unwrap();
        let local = analyze_file(target).unwrap();
        let mut document = Document::from_bytes(target);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            ..FormatConfig::default()
        };
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };

        assert!(project.declared_types.contains(b"CAMELTYPE"));
        assert_eq!(project.visible_type_spelling(&local, 1, b"CAMELTYPE"), None);
        assert_eq!(declared(&mut document, &context).unwrap(), Changed::No);
        assert_eq!(document.to_bytes(), target);
    }

    #[test]
    fn implicit_letter_ranges_do_not_follow_declared_symbol_case() {
        let target =
            b"subroutine s\ndimension H(3)\nimplicit real*8 (a-h,o-z)\nx = h\nend subroutine s\n";
        let project = analyze_project([(Path::new("target.f90"), target.as_slice())]).unwrap();
        let local = analyze_file(target).unwrap();
        let mut document = Document::from_bytes(target);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            ..FormatConfig::default()
        };
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };

        declared(&mut document, &context).unwrap();
        let output = document.to_bytes();
        assert!(output
            .windows(b"(a-h,o-z)".len())
            .any(|window| window == b"(a-h,o-z)"));
        assert!(output
            .windows(b"x = H".len())
            .any(|window| window == b"x = H"));
    }
}
