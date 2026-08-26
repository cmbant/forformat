//! Scope-aware reconciliation for the declared-case pass.
//!
//! `case_pass` remains the spelling engine. Scoped decisions are applied while
//! that pass still owns the token and its base spelling, so the project-aware
//! correction does not need a retained evidence map or a second token walk.

use crate::{
    analysis::{project::ResolvedType, scoped_declared_names},
    error::FormatError,
    transform::{
        document::Document,
        passes::case_pass::{self, CaseEvidence, CaseReconciler, Reconciliation},
        pipeline::{Changed, PassContext},
    },
};

pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let mut reconciler = ScopedReconciler { cx };
    case_pass::declared_with_names_and_reconciler(document, cx, &declared_names, &mut reconciler)
}

struct ScopedReconciler<'a, 'cx> {
    cx: &'a PassContext<'cx>,
}

impl CaseReconciler for ScopedReconciler<'_, '_> {
    const ENABLED: bool = true;

    fn reconcile(&mut self, evidence: &CaseEvidence, name: &[u8], line: usize) -> Reconciliation {
        scoped_spelling(evidence, name, line, self.cx)
    }
}

fn scoped_spelling(
    evidence: &CaseEvidence,
    name: &[u8],
    line: usize,
    cx: &PassContext,
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

        // The project-wide type table gives the base pass a CamelType spelling,
        // but the type is not visible here, so scoped policy must restore the
        // authored split spelling without ever emitting that base edit.
        assert!(project.declared_types.contains(b"CAMELTYPE"));
        assert_eq!(project.visible_type_spelling(&local, 1, b"CAMELTYPE"), None);
        assert_eq!(declared(&mut document, &context).unwrap(), Changed::No);
        assert_eq!(document.to_bytes(), target);
    }
}
