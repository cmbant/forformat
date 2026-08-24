#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImplicitGuard {
    Apply,
    Skip,
}

#[derive(Debug, Clone, Copy)]
struct SymbolQuery {
    line: usize,
    associate_alias: bool,
    implicit_guard: ImplicitGuard,
}

#[derive(Default)]
struct ClassificationContext<'a> {
    associates: Option<&'a AssociateFrame>,
    procedure_spellings: Option<&'a CaseMap>,
    evidence: Option<&'a mut CaseEvidence>,
}

/// Why the base declared-case pass made (or declined) a spelling decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CaseEvidence {
    KeepBase,
    Alias(Vec<u8>),
    Symbol {
        allow_external: bool,
    },
    Type,
    UseRemote {
        module: Vec<u8>,
    },
    Member {
        owner: Vec<Vec<u8>>,
        resolved_owner: Option<ResolvedType>,
    },
}

pub(crate) type CaseEvidenceMap = HashMap<(usize, usize), CaseEvidence>;

/// Step 5: apply scoped declared spellings to identifier occurrences.
#[cfg(test)]
pub(crate) fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    declared_with_names(document, cx, &declared_names)
}

#[cfg(test)]
fn declared_with_names(
    document: &mut Document,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
) -> Result<Changed, FormatError> {
    declared_with_names_impl(document, cx, declared_names, None)
}

pub(crate) fn declared_with_names_and_evidence(
    document: &mut Document,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
) -> Result<(Changed, CaseEvidenceMap), FormatError> {
    let mut evidence = CaseEvidenceMap::default();
    let changed = declared_with_names_impl(document, cx, declared_names, Some(&mut evidence))?;
    Ok((changed, evidence))
}
