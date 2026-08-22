//! Steps 1-5: macro casing and the declared-case engine.
//!
//! These run **before** the lexical joins, not after — `format_text` step 5
//! precedes step 6 — because joining tokens changes offsets that case
//! replacement was computed against.

use crate::{
    analysis::{
        names::{resolve, NameSpace},
        project::ResolvedType,
        scoped_declared_names, CaseMap, DeclaredNameIndex, DeclaredSpelling, TypeMaps,
    },
    error::FormatError,
    source::{
        tokens::{tokenize, Token, TokenKind},
        LexState, PhysicalLineKind,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::provenance::{source_spans, spread_replacement},
        pipeline::{Changed, PassContext},
    },
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

#[derive(Debug, Clone, Default)]
struct AssociateFrame {
    names: HashSet<Vec<u8>>,
    types: HashMap<Vec<u8>, Vec<u8>>,
    spellings: HashMap<Vec<u8>, Vec<u8>>,
    resolved_types: HashMap<Vec<u8>, ResolvedType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectAssociationKind {
    Type,
    Rank,
}

#[derive(Debug, Clone)]
enum AssociationScope {
    Associate(AssociateFrame),
    Select {
        kind: SelectAssociationKind,
        alias: Vec<u8>,
        base: AssociateFrame,
        active: AssociateFrame,
    },
}

impl AssociationScope {
    fn frame(&self) -> &AssociateFrame {
        match self {
            Self::Associate(frame) => frame,
            Self::Select { active, .. } => active,
        }
    }
}

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
///
/// `KeepBase` is deliberately the default. Only evidence obtained from the
/// compatibility/project-wide lookups is eligible for scoped re-resolution.
/// That makes a new semantic capability in this pass authoritative by
/// construction instead of requiring a matching syntax guard in `scoped_case`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CaseEvidence {
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

pub(super) type CaseEvidenceMap = HashMap<(usize, usize), CaseEvidence>;

impl AssociateFrame {
    fn extend_visible(&mut self, frame: &Self) {
        for name in &frame.names {
            self.names.insert(name.clone());
            // An untyped inner alias must shadow a typed outer alias with the
            // same name instead of exposing the outer entity by accident.
            self.types.remove(name);
            self.spellings.remove(name);
            self.resolved_types.remove(name);
            if let Some(type_name) = frame.types.get(name) {
                self.types.insert(name.clone(), type_name.clone());
            }
            if let Some(spelling) = frame.spellings.get(name) {
                self.spellings.insert(name.clone(), spelling.clone());
            }
            if let Some(owner) = frame.resolved_types.get(name) {
                self.resolved_types.insert(name.clone(), owner.clone());
            }
        }
    }
}

/// Steps 1-3: apply the spelling of every known macro name.
///
/// Sources of macro names, in collection order:
/// `-D NAME[=VALUE]` from the command line, then every `#define NAME` in the
/// project.  Both are already gathered into `ProjectContext::macros`; what is
/// missing is the replacement itself, in unquoted code only.
///
/// Macro replacement is limited to unquoted Fortran code and preserves directive definitions.
pub fn macros(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
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
        // CPP directive bodies are protected bytes.  In particular, a source
        // `#define` keeps the spelling it had when it was declared; only its
        // uses in Fortran code are canonicalized here.
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

    // The loop above reads one physical line at a time, which is all a macro
    // use needs unless the author broke the name itself across a continuation.
    // `zer&` / `&o` is two names to that loop and one to the statement, so the
    // spelling settled only after step 16 had rejoined the halves — on the run
    // *after* the one that wrapped them (I1).  Re-case those tokens from the
    // assembled statement, where the name is whole.
    changed = changed.or(crossing_macro_names(document, cx));
    Ok(changed)
}

/// Apply macro spellings to the names a continuation splits in two.
///
/// Only tokens that span more than one physical line are considered; every
/// other occurrence was already handled line by line, identically.
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

/// Step 5: `replace_declared_cases`, the whole case-normalization engine.
///
/// The resolution policy is already implemented and tested in
/// [`crate::analysis::names`]; what this pass adds is *finding the occurrences*:
/// every identifier in unquoted code, classified into the right name space —
/// module names in `USE`, type names after `TYPE(`/`CLASS(`, components after
/// `%` resolved through the type maps, type-bound procedure names, and plain
/// symbols everywhere else.
///
/// This is the declared-case pass for the formatter's scoped name tables.
pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    declared_with_names(document, cx, &declared_names)
}

/// [`declared`] against a name index the caller already has.
pub(super) fn declared_with_names(
    document: &mut Document,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
) -> Result<Changed, FormatError> {
    declared_with_names_impl(document, cx, declared_names, None)
}

/// Run the declared-case pass and report the evidence class for every name
/// token in the original analysis stream.
pub(super) fn declared_with_names_and_evidence(
    document: &mut Document,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
) -> Result<(Changed, CaseEvidenceMap), FormatError> {
    let mut evidence = CaseEvidenceMap::default();
    let changed = declared_with_names_impl(document, cx, declared_names, Some(&mut evidence))?;
    Ok((changed, evidence))
}

fn declared_with_names_impl(
    document: &mut Document,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    mut evidence_map: Option<&mut CaseEvidenceMap>,
) -> Result<Changed, FormatError> {
    // An implicit function result is declared in the procedure's local
    // namespace, while calls to that function resolve through the file-wide
    // procedure namespace. Capture that one-entity override once so the
    // header and every other occurrence use the same spelling in this pass.
    let procedure_spellings = implicit_function_spellings(cx.analysis, declared_names);
    let mut association_stack: Vec<AssociationScope> = Vec::new();
    let mut line_edits: Vec<Vec<(Range<usize>, Vec<u8>)>> = vec![Vec::new(); document.lines.len()];
    let record_evidence = evidence_map.is_some();

    // Work on assembled statements, not independently on physical lines. A
    // USE module, a TYPE(...) name, or a component can be on a continuation
    // line; provenance maps the token back to exactly the bytes that need an
    // edit without touching the continuation markers or surrounding text.
    for group in &cx.analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            let first = tokens
                .iter()
                .position(|token| token.kind != TokenKind::Number);
            let mut associate_context = AssociateFrame::default();
            for scope in &association_stack {
                associate_context.extend_visible(scope.frame());
            }
            let opening_scope = association_opening_scope(
                &tokens,
                first,
                group.lines.start,
                active_procedure(cx.scopes, group.lines.start),
                cx,
                &associate_context,
            );
            // Association names participate in ordinary spelling resolution
            // on the opening statement, but their types are not visible in
            // their own selectors. Fortran evaluates every selector in the
            // enclosing scope.
            let mut statement_context = associate_context.clone();
            if let Some(scope) = &opening_scope {
                let selector_only = select_association_spec(&tokens, first)
                    .is_some_and(|spec| !spec.explicit_alias);
                if !selector_only {
                    statement_context
                        .names
                        .extend(scope.frame().names.iter().cloned());
                }
            }
            for (index, token) in tokens.iter().enumerate() {
                if token.kind != TokenKind::Name {
                    continue;
                }
                let spans = source_spans(group, statement, token);
                let Some((line, first_span)) = spans.first() else {
                    continue;
                };
                let line = *line;
                let mut token_evidence = CaseEvidence::KeepBase;
                let replacement = classify_spelling(
                    &tokens,
                    index,
                    line,
                    declared_names,
                    cx,
                    ClassificationContext {
                        associates: Some(&statement_context),
                        procedure_spellings: Some(&procedure_spellings),
                        evidence: record_evidence.then_some(&mut token_evidence),
                    },
                );
                if record_evidence {
                    // Macros are an earlier, higher-priority namespace and are
                    // therefore always base evidence, even when their token
                    // appears in USE/ASSOCIATE syntax.
                    if !cx.project.macros.contains(token.text) {
                        reconcile_occurrence_evidence(
                            &tokens,
                            index,
                            &associate_context,
                            &mut token_evidence,
                        );
                    }
                    if let Some(map) = evidence_map.as_deref_mut() {
                        map.insert((line, first_span.start), token_evidence);
                    }
                }
                let Some(replacement) = replacement else {
                    continue;
                };
                if replacement.as_slice() == token.text {
                    continue;
                }
                let Some(pieces) = spread_replacement(&spans, token, &replacement) else {
                    continue;
                };
                for (line, span, piece) in pieces {
                    let line_start = cx.analysis.buffer.lines[line].span.start as usize;
                    line_edits[line].push((
                        span.start - line_start..span.end - line_start,
                        piece.to_vec(),
                    ));
                }
            }
            if let Some(scope) = opening_scope {
                association_stack.push(scope);
            }
            apply_select_guard(&tokens, group.lines.start, cx, &mut association_stack);
            if first.is_some_and(|index| tokens[index].is_name(b"end"))
                && tokens
                    .get(first.unwrap_or(0) + 1)
                    .is_some_and(|token| token.is_name(b"associate"))
                && matches!(
                    association_stack.last(),
                    Some(AssociationScope::Associate(_))
                )
            {
                association_stack.pop();
            }
            if first.is_some_and(|index| tokens[index].is_name(b"end"))
                && tokens
                    .get(first.unwrap_or(0) + 1)
                    .is_some_and(|token| token.is_name(b"select"))
                && matches!(
                    association_stack.last(),
                    Some(AssociationScope::Select { .. })
                )
            {
                association_stack.pop();
            }
        }
    }

    let mut changed = Changed::No;
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

/// Classify one identifier occurrence and return its canonical spelling.
///
/// When evidence reporting is requested, the slot starts as `KeepBase` and is
/// changed only at the compatibility/project-wide evidence sources below.
/// Semantic early returns therefore stay protected automatically.
fn classify_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
    context: ClassificationContext<'_>,
) -> Option<Vec<u8>> {
    let ClassificationContext {
        associates,
        procedure_spellings,
        mut evidence,
    } = context;
    let token = &tokens[index];

    if is_select_type_rank_keyword(tokens, index) {
        return None;
    }

    if crate::source::syntax::is_end_construct_keyword(tokens, index)
        || (index > 0 && crate::source::syntax::is_end_construct_keyword(tokens, index - 1))
    {
        return None;
    }

    let associate_alias = associates.is_some_and(|context| {
        context
            .names
            .contains(token.text.to_ascii_lowercase().as_slice())
    });

    if preceded_by_percent(tokens, index)
        && matches!(
            token.text.to_ascii_lowercase().as_slice(),
            b"err" | b"index"
        )
        && tokens
            .get(index - 2)
            .is_some_and(|token| token.kind == TokenKind::RParen)
    {
        record_member_evidence(tokens, index, line, cx, associates, &mut evidence);
        return None;
    }
    if cx.project.macros.contains(token.text) {
        return None;
    }

    if is_numeric_literal_kind_name(tokens, index) {
        record_case_evidence(
            &mut evidence,
            CaseEvidence::Symbol {
                allow_external: false,
            },
        );
        return file_symbol_spelling(
            declared_names,
            cx,
            token.text,
            SymbolQuery {
                line,
                associate_alias,
                implicit_guard: ImplicitGuard::Skip,
            },
        );
    }

    if let Some(spelling) =
        procedure_definition_spelling(tokens, index, line, declared_names, procedure_spellings)
    {
        return Some(spelling);
    }
    if is_declaration_entity(tokens, index) {
        return implicit_result_spelling(cx, line, token, procedure_spellings);
    }

    if let Some(space) = named_end_space(tokens, index) {
        return resolver_spelling(cx, space, token.text);
    }

    if let Some(space) = scope_header_space(tokens, index) {
        return resolver_spelling(cx, space, token.text);
    }

    if is_use_module(tokens, index) {
        return resolver_spelling(cx, NameSpace::Module, token.text);
    }

    if is_type_spec_name(tokens, index) {
        record_case_evidence(&mut evidence, CaseEvidence::Type);
        if cx.local.declared_types.contains(token.text)
            || cx.project.declared_types.contains(token.text)
        {
            return resolve(
                &cx.local.declared_types,
                &cx.project.declared_types,
                token.text,
            )
            .map(ToOwned::to_owned);
        }
        return None;
    }

    if is_intrinsic_kind_name(tokens, index) {
        record_case_evidence(
            &mut evidence,
            CaseEvidence::Symbol {
                allow_external: false,
            },
        );
        return file_symbol_spelling(
            declared_names,
            cx,
            token.text,
            SymbolQuery {
                line,
                associate_alias,
                implicit_guard: ImplicitGuard::Skip,
            },
        );
    }

    if preceded_by_percent(tokens, index) {
        record_member_evidence(tokens, index, line, cx, associates, &mut evidence);
        let procedure = active_procedure(cx.scopes, line);
        let owner_type = member_owner_type(
            tokens,
            index,
            procedure,
            cx.local,
            Some(&cx.project.types),
            true,
            associates,
        )?;
        let inherited = inherited_component_spelling(cx, &owner_type, token.text, true);
        if let Some(spelling) = inherited {
            return Some(spelling);
        }
        if let Some(spelling) = inherited_type_procedure_spelling(cx, &owner_type, token.text) {
            return Some(spelling);
        }
        return None;
    }

    if let Some(spelling) = implicit_result_spelling(cx, line, token, procedure_spellings) {
        return Some(spelling);
    }

    match declared_names.governing_local_case(line, token.text) {
        DeclaredSpelling::Spelling(spelling) => return Some(spelling.to_owned()),
        DeclaredSpelling::Ambiguous => return None,
        DeclaredSpelling::Absent => {}
    }
    if let Some(spelling) = procedure_spellings.and_then(|spellings| spellings.get(token.text)) {
        return Some(spelling.to_owned());
    }
    record_case_evidence(
        &mut evidence,
        CaseEvidence::Symbol {
            allow_external: is_external_reference(tokens, index),
        },
    );
    file_symbol_spelling(
        declared_names,
        cx,
        token.text,
        SymbolQuery {
            line,
            associate_alias,
            implicit_guard: if !is_use_statement(tokens) && implicit_guard_applies(tokens, index) {
                ImplicitGuard::Apply
            } else {
                ImplicitGuard::Skip
            },
        },
    )
}

fn record_case_evidence(evidence: &mut Option<&mut CaseEvidence>, value: CaseEvidence) {
    if let Some(slot) = evidence.as_mut() {
        **slot = value;
    }
}

fn record_member_evidence(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    cx: &PassContext,
    associates: Option<&AssociateFrame>,
    evidence: &mut Option<&mut CaseEvidence>,
) {
    let Some(owner) = component_owner_names(tokens, index, true) else {
        return;
    };
    let names = owner.into_iter().map(ToOwned::to_owned).collect::<Vec<_>>();
    let resolved_owner = exact_member_owner(&names, line, cx, associates);
    record_case_evidence(
        evidence,
        CaseEvidence::Member {
            owner: names,
            resolved_owner,
        },
    );
}

fn exact_member_owner(
    names: &[Vec<u8>],
    line: usize,
    cx: &PassContext,
    associates: Option<&AssociateFrame>,
) -> Option<ResolvedType> {
    let root = names.first()?;
    let mut current = associates
        .and_then(|frame| frame.resolved_types.get(root.as_slice()).cloned())
        .or_else(|| cx.project.visible_variable_type(cx.local, line, root))?;
    for link in &names[1..] {
        current = cx
            .project
            .visible_component_type(cx.local, line, &current, link)?;
    }
    Some(current)
}

/// Reclassify occurrence-level namespaces whose provenance depends on the
/// statement's semantic context rather than on the generic symbol lookup.
/// This lives beside the base classifier so `scoped_case` never needs to
/// recognize these statement shapes itself.
fn reconcile_occurrence_evidence(
    tokens: &[Token<'_>],
    index: usize,
    enclosing_associates: &AssociateFrame,
    evidence: &mut CaseEvidence,
) {
    let token = &tokens[index];

    if let Some(module_index) = use_module_index(tokens) {
        if is_use_intrinsic(tokens) || index <= module_index || is_use_only_keyword(tokens, index) {
            *evidence = CaseEvidence::KeepBase;
        } else if is_use_rename_local(tokens, index) {
            *evidence = CaseEvidence::Alias(token.text.to_vec());
        } else {
            *evidence = CaseEvidence::UseRemote {
                module: tokens[module_index].text.to_vec(),
            };
        }
        return;
    }

    if is_associate_alias_declaration(tokens, index) || is_select_alias_declaration(tokens, index) {
        *evidence = CaseEvidence::Alias(token.text.to_vec());
        return;
    }

    if let CaseEvidence::Member {
        owner,
        resolved_owner,
    } = evidence
    {
        if resolved_owner.is_none()
            && owner
                .first()
                .and_then(|root| associate_spelling(enclosing_associates, root))
                .is_some()
        {
            // The base pass can know this member's owner from the selector
            // type inferred for the ASSOCIATE alias. The project-scoped
            // variable resolver cannot reproduce that evidence, so the base
            // answer is authoritative.
            *evidence = CaseEvidence::KeepBase;
            return;
        }
    }

    if !preceded_by_percent(tokens, index) {
        if let Some(spelling) = associate_spelling(enclosing_associates, token.text) {
            *evidence = CaseEvidence::Alias(spelling.to_vec());
        }
    }
}

/// A type-bound binding and the module procedure it names are one entity.
/// Prefer the binding spelling on the procedure definition and its named END;
/// the ordinary symbol table may contain the authored procedure spelling too,
/// but that is not a second declaration of a different entity.
fn procedure_definition_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &crate::analysis::DeclaredNameIndex,
    procedure_spellings: Option<&CaseMap>,
) -> Option<Vec<u8>> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    let procedure_name = if tokens.get(first).is_some_and(|token| token.is_name(b"end")) {
        tokens
            .get(first + 1)
            .filter(|token| token.is_name(b"function") || token.is_name(b"subroutine"))
            .and_then(|_| tokens.get(first + 2))
            .filter(|_| index == first + 2)
    } else {
        tokens[..index]
            .iter()
            .enumerate()
            .rev()
            .find(|(position, token)| {
                *position + 1 == index
                    && token.depth == 0
                    && (token.is_name(b"function") || token.is_name(b"subroutine"))
            })
            .and_then(|_| tokens.get(index))
    }?;
    procedure_spellings
        .and_then(|spellings| spellings.get(procedure_name.text))
        .or_else(|| {
            declared_names
                .local_at(line)
                .and_then(|local| local.get(procedure_name.text))
        })
        .map(ToOwned::to_owned)
}

fn implicit_result_spelling(
    cx: &PassContext,
    line: usize,
    token: &Token<'_>,
    procedure_spellings: Option<&CaseMap>,
) -> Option<Vec<u8>> {
    let active = active_procedure(cx.scopes, line)?;
    if !active.eq_ignore_ascii_case(token.text) {
        return None;
    }
    procedure_spellings?.get(token.text).map(ToOwned::to_owned)
}

fn implicit_function_spellings(
    analysis: &crate::transform::document::Analysis,
    declared_names: &crate::analysis::DeclaredNameIndex,
) -> CaseMap {
    let mut spellings = CaseMap::default();
    for group in &analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            let Some(first) = tokens
                .iter()
                .position(|token| token.kind != TokenKind::Number)
            else {
                continue;
            };
            if tokens[first].is_name(b"end") {
                continue;
            }
            let Some(function) = tokens
                .iter()
                .position(|token| token.depth == 0 && token.is_name(b"function"))
            else {
                continue;
            };
            let Some(name) = tokens
                .get(function + 1)
                .filter(|token| token.kind == TokenKind::Name)
            else {
                continue;
            };
            if tokens
                .iter()
                .skip(function + 2)
                .any(|token| token.depth == 0 && token.is_name(b"result"))
            {
                continue;
            }
            if !declared_names.local_contains(group.lines.start, name.text) {
                continue;
            }
            spellings.insert(name.text);
        }
    }
    spellings
}

fn resolver_spelling(cx: &PassContext, space: NameSpace, name: &[u8]) -> Option<Vec<u8>> {
    cx.resolver().spelling(space, name).map(ToOwned::to_owned)
}

pub(crate) fn restore_declined_component_spellings(
    original: &[u8],
    updated: &[u8],
    line: usize,
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
) -> Vec<u8> {
    let original_tokens = tokenize(original, &mut LexState::default());
    let declined: Vec<Option<&[u8]>> = original_tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            if token.kind != TokenKind::Name
                || index == 0
                || original_tokens[index - 1].text != b"%"
            {
                return None;
            }
            let spelling = classify_spelling(
                &original_tokens,
                index,
                line,
                declared_names,
                cx,
                ClassificationContext::default(),
            )
            .is_none()
            .then_some(token.text);
            Some(spelling)
        })
        .collect();
    if declined.iter().all(Option::is_none) {
        return updated.to_vec();
    }

    let updated_tokens = tokenize(updated, &mut LexState::default());
    let updated_components = updated_tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            token.kind == TokenKind::Name && *index > 0 && updated_tokens[*index - 1].text == b"%"
        })
        .count();
    if updated_components != declined.len() {
        debug_assert_eq!(updated_components, declined.len());
        return updated.to_vec();
    }
    let mut component = 0;
    let mut edits = EditBuffer::new(updated);
    for (index, token) in updated_tokens.iter().enumerate() {
        if token.kind != TokenKind::Name || index == 0 || updated_tokens[index - 1].text != b"%" {
            continue;
        }
        if let Some(Some(spelling)) = declined.get(component) {
            edits.replace(token.span.clone(), spelling);
        }
        component += 1;
    }
    edits.finish()
}

fn inherited_component_spelling(
    cx: &PassContext,
    owner: &[u8],
    name: &[u8],
    allow_project: bool,
) -> Option<Vec<u8>> {
    if cx.project.macros.contains(name) {
        return cx.project.macros.get(name).map(ToOwned::to_owned);
    }

    let mut current = owner.to_ascii_lowercase();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current.clone()) {
            return None;
        }
        if cx.local.cases.components.contains(&current, name) {
            return cx
                .local
                .cases
                .components
                .get(&current, name)
                .map(ToOwned::to_owned);
        }
        if allow_project && cx.project.cases.components.contains(&current, name) {
            return cx
                .project
                .cases
                .components
                .get(&current, name)
                .map(ToOwned::to_owned);
        }
        let parent = if cx.local.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.local.types.parent_types.contains_key(&current) {
            cx.local.types.parent_type(&current)
        } else if allow_project && cx.project.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if allow_project && cx.project.types.parent_types.contains_key(&current) {
            cx.project.types.parent_type(&current)
        } else {
            None
        };
        let parent = parent?;
        current = parent.to_vec();
    }
}

fn inherited_type_procedure_spelling(
    cx: &PassContext,
    owner: &[u8],
    name: &[u8],
) -> Option<Vec<u8>> {
    if cx.local.generic_type_procedures.contains(name) {
        return None;
    }
    let resolver = cx.resolver();
    let mut current = owner.to_ascii_lowercase();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current.clone()) {
            return None;
        }
        if let Some(spelling) = resolver.type_procedure_spelling(&current, name) {
            return Some(spelling.to_vec());
        }
        if let Some(spelling) = cx.project.generic_bound_type_procedures.get(&current, name) {
            return Some(spelling.to_vec());
        }
        let parent = if cx.local.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.local.types.parent_types.contains_key(&current) {
            cx.local.types.parent_type(&current)
        } else if cx.project.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.project.types.parent_types.contains_key(&current) {
            cx.project.types.parent_type(&current)
        } else {
            None
        };
        current = parent?.to_vec();
    }
}

fn file_symbol_spelling(
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
    name: &[u8],
    query: SymbolQuery,
) -> Option<Vec<u8>> {
    if cx.local.file_symbols.contains(name) {
        return cx.local.file_symbols.get(name).map(ToOwned::to_owned);
    }
    if !query.associate_alias && declared_names.file_declared_anywhere(name).is_declared() {
        return None;
    }
    if query.implicit_guard == ImplicitGuard::Apply
        && !query.associate_alias
        && declared_names.implicit_allows(query.line, name)
    {
        return None;
    }
    resolve(&cx.local.file_symbols, &cx.project.file_symbols, name).map(ToOwned::to_owned)
}

fn is_use_statement(tokens: &[Token<'_>]) -> bool {
    tokens
        .iter()
        .find(|token| token.kind != TokenKind::Number)
        .is_some_and(|token| token.is_name(b"use"))
}

fn implicit_guard_applies(tokens: &[Token<'_>], index: usize) -> bool {
    if tokens
        .get(index + 1)
        .is_some_and(|token| token.text == b"%")
    {
        return false;
    }
    !index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| token.is_name(b"call"))
}

fn preceded_by_percent(tokens: &[Token<'_>], index: usize) -> bool {
    index > 0 && tokens[index - 1].text == b"%"
}

fn active_procedure(scopes: &crate::analysis::ScopeTree, line: usize) -> Option<&[u8]> {
    scopes
        .ancestors(scopes.index_of_line(line))
        .into_iter()
        .find(|scope| {
            matches!(
                scopes.scopes[*scope].kind,
                crate::analysis::scope::ScopeKind::Program
                    | crate::analysis::scope::ScopeKind::Procedure
            )
        })
        .and_then(|scope| scopes.scopes[scope].name.as_deref())
}

#[derive(Debug)]
struct SelectAssociationSpec<'a> {
    kind: SelectAssociationKind,
    alias_index: usize,
    alias: &'a [u8],
    selector: &'a [Token<'a>],
    explicit_alias: bool,
}

fn select_type_rank_opening(
    tokens: &[Token<'_>],
    first: Option<usize>,
) -> Option<(usize, SelectAssociationKind)> {
    let first = first?;
    let select = if tokens[first].is_name(b"select") {
        first
    } else if tokens[first].kind == TokenKind::Name
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.text == b":")
        && tokens
            .get(first + 2)
            .is_some_and(|token| token.is_name(b"select"))
    {
        first + 2
    } else {
        return None;
    };
    let kind = if tokens
        .get(select + 1)
        .is_some_and(|token| token.is_name(b"type"))
    {
        SelectAssociationKind::Type
    } else if tokens
        .get(select + 1)
        .is_some_and(|token| token.is_name(b"rank"))
    {
        SelectAssociationKind::Rank
    } else {
        return None;
    };
    Some((select, kind))
}

fn select_association_spec<'a>(
    tokens: &'a [Token<'a>],
    first: Option<usize>,
) -> Option<SelectAssociationSpec<'a>> {
    let (select, kind) = select_type_rank_opening(tokens, first)?;
    let open_index = select + 2;
    let open = tokens
        .get(open_index)
        .filter(|token| token.kind == TokenKind::LParen)?;
    let close = tokens
        .iter()
        .enumerate()
        .skip(open_index + 1)
        .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == open.depth)
        .map(|(index, _)| index)?;
    let entry = &tokens[open_index + 1..close];
    if let [alias, arrow, selector @ ..] = entry {
        if alias.kind == TokenKind::Name
            && alias.depth == open.depth + 1
            && arrow.text == b"=>"
            && arrow.depth == alias.depth
            && !selector.is_empty()
        {
            return Some(SelectAssociationSpec {
                kind,
                alias_index: open_index + 1,
                alias: alias.text,
                selector,
                explicit_alias: true,
            });
        }
    }
    let alias = entry
        .first()
        .filter(|token| entry.len() == 1 && token.kind == TokenKind::Name)?;
    Some(SelectAssociationSpec {
        kind,
        alias_index: open_index + 1,
        alias: alias.text,
        selector: entry,
        explicit_alias: false,
    })
}

fn association_opening_scope(
    tokens: &[Token<'_>],
    first: Option<usize>,
    line: usize,
    procedure: Option<&[u8]>,
    cx: &PassContext,
    outer: &AssociateFrame,
) -> Option<AssociationScope> {
    if associate_opening(tokens, first).is_some() {
        return Some(AssociationScope::Associate(associate_frame(
            tokens, line, procedure, cx, outer,
        )));
    }
    let spec = select_association_spec(tokens, first)?;
    let alias = spec.alias.to_ascii_lowercase();
    let mut frame = AssociateFrame::default();
    insert_association(
        &mut frame,
        spec.alias,
        spec.selector,
        spec.explicit_alias,
        line,
        procedure,
        cx,
        outer,
    );
    Some(AssociationScope::Select {
        kind: spec.kind,
        alias,
        base: frame.clone(),
        active: frame,
    })
}

fn apply_select_guard(
    tokens: &[Token<'_>],
    line: usize,
    cx: &PassContext,
    stack: &mut [AssociationScope],
) {
    let Some(AssociationScope::Select {
        kind,
        alias,
        base,
        active,
    }) = stack.last_mut()
    else {
        return;
    };
    match kind {
        SelectAssociationKind::Type => {
            let Some(guard) = select_type_guard_name(tokens) else {
                return;
            };
            *active = base.clone();
            let Some(type_name) = guard else {
                return;
            };
            active.types.remove(alias.as_slice());
            active.resolved_types.remove(alias.as_slice());
            if let Some(owner) = cx.project.visible_type(cx.local, line, type_name) {
                active.types.insert(alias.clone(), owner.name.clone());
                active.resolved_types.insert(alias.clone(), owner);
            }
        }
        SelectAssociationKind::Rank => {
            if is_select_rank_guard(tokens) {
                *active = base.clone();
            }
        }
    }
}

fn select_type_guard_name<'a>(tokens: &'a [Token<'a>]) -> Option<Option<&'a [u8]>> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !(tokens[first].is_name(b"type") || tokens[first].is_name(b"class")) {
        return None;
    }
    if tokens
        .get(first + 1)
        .is_some_and(|token| token.is_name(b"default"))
    {
        return Some(None);
    }
    if !tokens
        .get(first + 1)
        .is_some_and(|token| token.is_name(b"is"))
    {
        return None;
    }
    let open = tokens
        .get(first + 2)
        .filter(|token| token.kind == TokenKind::LParen)?;
    let name = tokens
        .get(first + 3)
        .filter(|token| token.kind == TokenKind::Name && token.depth == open.depth + 1)?;
    Some(Some(name.text))
}

fn is_select_rank_guard(tokens: &[Token<'_>]) -> bool {
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    tokens[first].is_name(b"rank")
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen || token.is_name(b"default"))
}

fn associate_opening(tokens: &[Token<'_>], first: Option<usize>) -> Option<usize> {
    let first = first?;
    if tokens[first].is_name(b"associate") {
        return Some(first);
    }
    (tokens[first].kind == TokenKind::Name
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.text == b":")
        && tokens
            .get(first + 2)
            .is_some_and(|token| token.is_name(b"associate")))
    .then_some(first + 2)
}

fn associate_frame(
    tokens: &[Token<'_>],
    line: usize,
    procedure: Option<&[u8]>,
    cx: &PassContext,
    outer: &AssociateFrame,
) -> AssociateFrame {
    let mut frame = AssociateFrame::default();
    for (alias, selector) in associate_specs(tokens) {
        insert_association(
            &mut frame, alias, selector, true, line, procedure, cx, outer,
        );
    }
    frame
}

fn insert_association(
    frame: &mut AssociateFrame,
    alias: &[u8],
    selector: &[Token<'_>],
    preserve_spelling: bool,
    line: usize,
    procedure: Option<&[u8]>,
    cx: &PassContext,
    outer: &AssociateFrame,
) {
    let name = alias.to_ascii_lowercase();
    frame.names.insert(name.clone());
    if preserve_spelling {
        frame.spellings.insert(name.clone(), alias.to_vec());
    }
    if let Some(type_name) = designator_type(
        selector,
        procedure,
        cx.local,
        Some(&cx.project.types),
        outer,
    ) {
        frame.types.insert(name.clone(), type_name);
    }
    if let Some(names) = designator_names(selector) {
        let root = names[0].to_ascii_lowercase();
        let mut resolved = outer
            .resolved_types
            .get(root.as_slice())
            .cloned()
            .or_else(|| cx.project.visible_variable_type(cx.local, line, &root));
        for link in &names[1..] {
            resolved = resolved.and_then(|owner| {
                cx.project
                    .visible_component_type(cx.local, line, &owner, link)
            });
        }
        if let Some(resolved) = resolved {
            frame.resolved_types.insert(name, resolved);
        }
    }
}

fn associate_specs<'a>(tokens: &'a [Token<'a>]) -> Vec<(&'a [u8], &'a [Token<'a>])> {
    let Some(associate) = tokens.iter().position(|token| token.is_name(b"associate")) else {
        return Vec::new();
    };
    let Some(open) = tokens
        .get(associate + 1)
        .filter(|token| token.kind == TokenKind::LParen)
    else {
        return Vec::new();
    };
    let entry_depth = open.depth + 1;
    let close = tokens
        .iter()
        .enumerate()
        .skip(associate + 2)
        .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == open.depth)
        .map(|(index, _)| index)
        .unwrap_or(tokens.len());

    let mut specs = Vec::new();
    let mut start = associate + 2;
    for end in (start..close)
        .filter(|index| {
            tokens[*index].kind == TokenKind::Comma && tokens[*index].depth == entry_depth
        })
        .chain(std::iter::once(close))
    {
        let entry = &tokens[start..end];
        if let [alias, arrow, selector @ ..] = entry {
            if alias.kind == TokenKind::Name
                && alias.depth == entry_depth
                && arrow.text == b"=>"
                && arrow.depth == entry_depth
                && !selector.is_empty()
            {
                specs.push((alias.text, selector));
            }
        }
        start = end.saturating_add(1);
    }
    specs
}

fn designator_type(
    tokens: &[Token<'_>],
    procedure: Option<&[u8]>,
    local: &crate::analysis::FileFacts,
    project: Option<&TypeMaps>,
    associates: &AssociateFrame,
) -> Option<Vec<u8>> {
    let names = designator_names(tokens)?;
    let root = names.first()?;
    if associates
        .names
        .contains(root.to_ascii_lowercase().as_slice())
    {
        let current = associates
            .types
            .get(root.to_ascii_lowercase().as_slice())?
            .clone();
        return resolve_component_owner(current, &names[1..], &local.types, project);
    }
    if local
        .types
        .resolve_chain_with_locals(procedure, root, &[])
        .is_some()
    {
        let current = local
            .types
            .resolve_chain_with_locals(procedure, root, &[])?;
        return resolve_component_owner(current, &names[1..], &local.types, project);
    }
    if procedure.is_none() && local.types.has_procedure_local_root(root) {
        return None;
    }
    if let (Some(project), Some(imported)) = (
        project,
        project.and_then(|types| local.imported_variable_type(types, root)),
    ) {
        return resolve_component_owner(imported, &names[1..], &local.types, Some(project));
    }
    project.and_then(|types| types.resolve_chain(root, &names[1..]))
}

fn designator_names<'a>(tokens: &'a [Token<'a>]) -> Option<Vec<&'a [u8]>> {
    let root = tokens.first()?.kind == TokenKind::Name;
    if !root {
        return None;
    }
    let base_depth = tokens[0].depth;
    let mut names = vec![tokens[0].text];
    let mut index = 1;
    while index < tokens.len() {
        if tokens[index].kind.opens_bracket() && tokens[index].depth == base_depth {
            let open_kind = tokens[index].kind;
            let close_kind = match open_kind {
                TokenKind::LParen => TokenKind::RParen,
                TokenKind::LBracket => TokenKind::RBracket,
                _ => return None,
            };
            index += 1;
            while index < tokens.len()
                && !(tokens[index].kind == close_kind && tokens[index].depth == base_depth)
            {
                index += 1;
            }
            if index == tokens.len() {
                return None;
            }
            index += 1;
            continue;
        }
        if tokens[index].text != b"%" || tokens[index].depth != base_depth {
            return None;
        }
        let member = tokens.get(index + 1)?;
        if member.kind != TokenKind::Name || member.depth != base_depth {
            return None;
        }
        names.push(member.text);
        index += 2;
    }
    Some(names)
}

fn associate_spelling<'a>(frame: &'a AssociateFrame, name: &[u8]) -> Option<&'a [u8]> {
    frame
        .spellings
        .get(name.to_ascii_lowercase().as_slice())
        .map(Vec::as_slice)
}

fn is_associate_alias_declaration(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(associate) = tokens.iter().position(|token| token.is_name(b"associate")) else {
        return false;
    };
    let Some(open) = tokens
        .get(associate + 1)
        .filter(|token| token.kind == TokenKind::LParen)
    else {
        return false;
    };
    tokens[index].depth == open.depth + 1
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.text == b"=>" && token.depth == tokens[index].depth)
}

fn is_select_alias_declaration(tokens: &[Token<'_>], index: usize) -> bool {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number);
    select_association_spec(tokens, first)
        .is_some_and(|spec| spec.explicit_alias && spec.alias_index == index)
}

fn is_select_type_rank_keyword(tokens: &[Token<'_>], index: usize) -> bool {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number);
    if let Some((select, _)) = select_type_rank_opening(tokens, first) {
        return index == select || index == select + 1;
    }
    let Some(first) = first else {
        return false;
    };
    if tokens[first].is_name(b"type") || tokens[first].is_name(b"class") {
        return index == first
            || (index == first + 1
                && tokens[index].kind == TokenKind::Name
                && (tokens[index].is_name(b"is") || tokens[index].is_name(b"default")));
    }
    if tokens[first].is_name(b"rank") {
        return index == first || (index == first + 1 && tokens[index].is_name(b"default"));
    }
    false
}

fn use_module_index(tokens: &[Token<'_>]) -> Option<usize> {
    let use_index = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens[use_index].is_name(b"use") {
        return None;
    }
    let mut cursor = use_index + 1;
    if tokens
        .get(cursor)
        .is_some_and(|token| token.kind == TokenKind::Comma)
    {
        while cursor < tokens.len() && tokens[cursor].text != b"::" {
            cursor += 1;
        }
        if cursor == tokens.len() {
            return None;
        }
        cursor += 1;
    } else if tokens.get(cursor).is_some_and(|token| token.text == b"::") {
        cursor += 1;
    }
    tokens
        .get(cursor)
        .filter(|token| token.kind == TokenKind::Name)
        .map(|_| cursor)
}

fn is_use_intrinsic(tokens: &[Token<'_>]) -> bool {
    let Some(use_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    let Some(separator) = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")
    else {
        return false;
    };
    tokens[use_index + 1..separator]
        .iter()
        .any(|token| token.depth == 0 && token.is_name(b"intrinsic"))
}

fn is_use_module(tokens: &[Token<'_>], index: usize) -> bool {
    use_module_index(tokens) == Some(index)
}

fn is_use_only_keyword(tokens: &[Token<'_>], index: usize) -> bool {
    tokens[index].is_name(b"only")
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.text == b":" && token.depth == tokens[index].depth)
}

fn is_use_rename_local(tokens: &[Token<'_>], index: usize) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.text == b"=>" && token.depth == tokens[index].depth)
}

fn is_external_reference(tokens: &[Token<'_>], index: usize) -> bool {
    index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| token.is_name(b"call"))
        || tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen)
}

fn is_type_spec_name(tokens: &[Token<'_>], index: usize) -> bool {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number);
    if select_association_spec(tokens, first).is_some_and(|spec| spec.alias_index == index) {
        return false;
    }
    if index < 2 || tokens[index - 1].kind != TokenKind::LParen {
        return false;
    }
    if tokens[index - 2].kind == TokenKind::Name
        && (tokens[index - 2].is_name(b"type") || tokens[index - 2].is_name(b"class"))
    {
        return true;
    }
    index >= 3
        && tokens[index - 2].is_name(b"is")
        && (tokens[index - 3].is_name(b"type") || tokens[index - 3].is_name(b"class"))
}

fn is_intrinsic_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
    if index < 2 || tokens[index - 1].kind != TokenKind::LParen {
        return false;
    }
    let Some(type_name) = tokens.get(index - 2) else {
        return false;
    };
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    type_name.kind == TokenKind::Name
        && index - 2 == first
        && [b"integer".as_slice(), b"real", b"complex", b"logical"]
            .iter()
            .any(|candidate| type_name.is_name(candidate))
}

fn named_end_space(tokens: &[Token<'_>], index: usize) -> Option<NameSpace> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens.get(first).is_some_and(|token| token.is_name(b"end")) {
        return None;
    }
    if index != first + 2 {
        return None;
    }
    match tokens.get(first + 1).map(|token| token.text) {
        Some(kind)
            if kind.eq_ignore_ascii_case(b"module") || kind.eq_ignore_ascii_case(b"submodule") =>
        {
            Some(NameSpace::Module)
        }
        Some(kind)
            if [
                b"function".as_slice(),
                b"subroutine",
                b"program",
                b"procedure",
                b"blockdata",
            ]
            .iter()
            .any(|candidate| kind.eq_ignore_ascii_case(candidate)) =>
        {
            Some(NameSpace::Symbol)
        }
        Some(kind) if kind.eq_ignore_ascii_case(b"type") => Some(NameSpace::Type),
        _ => None,
    }
}

fn scope_header_space(tokens: &[Token<'_>], index: usize) -> Option<NameSpace> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if first + 1 != index {
        return None;
    }
    match tokens[first].text.to_ascii_lowercase().as_slice() {
        b"module" | b"submodule" => Some(NameSpace::Module),
        b"program" | b"function" | b"subroutine" | b"procedure" | b"blockdata" => {
            Some(NameSpace::Symbol)
        }
        _ => None,
    }
}

fn is_declaration_entity(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(separator) = tokens[..index]
        .iter()
        .enumerate()
        .filter(|(_, token)| token.depth == 0 && token.text == b"::")
        .map(|(position, _)| position)
        .next_back()
    else {
        return old_style_declaration_entity(tokens, index);
    };
    let mut initializer = false;
    let mut array_depth = 0usize;
    for token in &tokens[separator + 1..index] {
        if token.kind == TokenKind::LBracket {
            array_depth += 1;
            continue;
        }
        if token.kind == TokenKind::RBracket {
            array_depth = array_depth.saturating_sub(1);
            continue;
        }
        if token.depth != 0 && array_depth == 0 {
            continue;
        }
        if token.text == b"=" || token.text == b"=>" {
            initializer = true;
        } else if token.kind == TokenKind::Comma {
            initializer = false;
        }
    }
    !initializer && array_depth == 0 && tokens[index].depth == 0
}

fn old_style_declaration_entity(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(first_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    let first = &tokens[first_index];
    let is_type = matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer" | b"real" | b"complex" | b"logical" | b"character" | b"type" | b"class"
    ) || first.is_name(b"double")
        && tokens
            .get(first_index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    if !is_type || index <= first_index {
        return false;
    }
    if tokens
        .iter()
        .skip(first_index + 1)
        .take(index.saturating_sub(first_index + 1))
        .any(|token| token.kind == TokenKind::Name && token.is_name(b"function"))
    {
        return false;
    }
    if tokens[..index]
        .iter()
        .any(|token| token.depth == 0 && (token.text == b"=" || token.text == b"=>"))
    {
        return false;
    }
    let mut start = first_index + 1;
    if first.is_name(b"double") {
        start += 1;
    }
    if tokens
        .get(start)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        let depth = tokens[start].depth;
        let Some(close) = tokens
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == depth)
            .map(|(position, _)| position)
        else {
            return false;
        };
        start = close + 1;
    }
    if index < start || tokens[index].kind != TokenKind::Name || tokens[index].depth != 0 {
        return false;
    }

    let entity_start = tokens[start..index]
        .iter()
        .enumerate()
        .filter(|(_, token)| token.depth == 0 && token.kind == TokenKind::Comma)
        .map(|(offset, _)| start + offset + 1)
        .next_back()
        .unwrap_or(start);
    let before = &tokens[entity_start..index];
    !before
        .iter()
        .any(|token| token.depth == 0 && (token.text == b"=" || token.text == b"=>"))
        && !before.iter().any(|token| {
            token.kind == TokenKind::Name && token.depth == 0 && token.text != b"intent"
        })
}

fn is_numeric_literal_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2 && tokens[index - 1].text == b"_" && tokens[index - 2].kind == TokenKind::Number
}

fn member_owner_type(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &crate::analysis::FileFacts,
    project: Option<&TypeMaps>,
    indexed_chain: bool,
    associates: Option<&AssociateFrame>,
) -> Option<Vec<u8>> {
    let names = component_owner_names(tokens, index, indexed_chain)?;
    let root = names.first()?;
    if let Some(associates) =
        associates.filter(|context| context.names.contains(root.to_ascii_lowercase().as_slice()))
    {
        let current = associates
            .types
            .get(root.to_ascii_lowercase().as_slice())?
            .clone();
        return resolve_component_owner(current, &names[1..], &local.types, project);
    }
    if local
        .types
        .resolve_chain_with_locals(procedure, root, &[])
        .is_some()
    {
        member_owner_type_with_project_components(tokens, index, procedure, &local.types, project)
    } else if procedure.is_none() && local.types.has_procedure_local_root(root) {
        None
    } else if let (Some(project), Some(imported)) = (
        project,
        project.and_then(|types| local.imported_variable_type(types, root)),
    ) {
        resolve_component_owner(imported, &names[1..], &local.types, Some(project))
    } else {
        project.and_then(|types| types.resolve_chain(root, &names[1..]))
    }
}

fn member_owner_type_with_project_components(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &TypeMaps,
    project: Option<&TypeMaps>,
) -> Option<Vec<u8>> {
    let names = component_owner_names(tokens, index, true)?;
    let root = names.first()?;
    let current = local.resolve_chain_with_locals(procedure, root, &[])?;
    resolve_component_owner(current, &names[1..], local, project)
}

fn resolve_component_owner(
    mut current: Vec<u8>,
    links: &[&[u8]],
    local: &TypeMaps,
    project: Option<&TypeMaps>,
) -> Option<Vec<u8>> {
    for link in links {
        current = local
            .component_type(&current, link)
            .or_else(|| project.and_then(|types| types.component_type(&current, link)))?;
    }
    Some(current)
}

fn component_owner_names<'a>(
    tokens: &'a [Token<'a>],
    index: usize,
    indexed_chain: bool,
) -> Option<Vec<&'a [u8]>> {
    if index < 2 || !preceded_by_percent(tokens, index) {
        return None;
    }
    let mut names = Vec::new();
    let mut cursor = index - 2;
    loop {
        if indexed_chain && tokens.get(cursor)?.kind == TokenKind::RParen {
            let mut depth = 1;
            while cursor > 0 {
                cursor -= 1;
                match tokens[cursor].kind {
                    TokenKind::RParen => depth += 1,
                    TokenKind::LParen => {
                        depth -= 1;
                        if depth == 0 {
                            cursor = cursor.checked_sub(1)?;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let token = tokens.get(cursor)?;
        if token.kind != TokenKind::Name {
            return None;
        }
        names.push(token.text);
        if cursor < 2 || tokens[cursor - 1].text != b"%" {
            break;
        }
        cursor -= 2;
    }
    names.reverse();
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::{declared, macros};
    use crate::{
        analysis::{analyze_file, analyze_project, ScopeTree},
        config::{FormatConfig, FormatMode},
        format_source_with_context,
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
    };
    use std::path::Path;

    fn run_pass(
        source: &[u8],
        project: &crate::analysis::ProjectContext,
        pass: impl FnOnce(&mut Document, &PassContext<'_>) -> Changed,
    ) -> Vec<u8> {
        let mut document = Document::from_bytes(source);
        let local = analyze_file(source).unwrap();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            ..FormatConfig::default()
        };
        let context = PassContext {
            config: &config,
            project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        let _ = pass(&mut document, &context);
        document.to_bytes()
    }

    #[test]
    fn macro_uses_are_replaced_but_cpp_strings_and_comments_are_protected() {
        let source = b"#define My_Macro 1\nprogram p\nx = MY_MACRO\ns = 'MY_MACRO' ! MY_MACRO\n#if MY_MACRO\nend program p\n";
        let project = analyze_project([(Path::new("macros.f90"), source.as_slice())]).unwrap();
        let output = run_pass(source, &project, |document, context| {
            macros(document, context).unwrap()
        });
        assert_eq!(
            output,
            b"#define My_Macro 1\nprogram p\nx = My_Macro\ns = 'MY_MACRO' ! MY_MACRO\n#if MY_MACRO\nend program p\n"
        );
    }

    #[test]
    fn declared_occurrences_use_their_name_spaces_and_are_idempotent() {
        let source = b"module MiXeD\ntype :: MyType\ninteger :: Source\ncontains\nprocedure :: BuildValue\nend type MyType\ninteger :: Global\ncontains\nsubroutine Work(Local)\ntype(MyType) :: obj\nlocal = GLOBAL\nobj%source = 1\ncall obj%buildvalue()\nend subroutine work\nend module mixed\n";
        let project = analyze_project([(Path::new("names.f90"), source.as_slice())]).unwrap();
        let once = run_pass(source, &project, |document, context| {
            macros(document, context).unwrap();
            declared(document, context).unwrap()
        });
        assert_eq!(
            once,
            b"module MiXeD\ntype :: MyType\ninteger :: Source\ncontains\nprocedure :: BuildValue\nend type MyType\ninteger :: Global\ncontains\nsubroutine Work(Local)\ntype(MyType) :: obj\nLocal = Global\nobj%Source = 1\ncall obj%BuildValue()\nend subroutine Work\nend module MiXeD\n"
        );
        let twice = run_pass(&once, &project, |document, context| {
            macros(document, context).unwrap();
            declared(document, context).unwrap()
        });
        assert_eq!(twice, once);
    }

    #[test]
    fn implicit_function_result_spelling_is_shared_with_calls() {
        let source = b"module m\n\
contains\n\
function BETA3(x)\n\
implicit none\n\
real :: x\n\
real :: BeTa3\n\
BeTa3 = x\n\
end function beta3\n\
subroutine s(x, num)\n\
real :: x, num\n\
num = bEtA3(x)\n\
end subroutine s\n\
end module m\n";
        let project =
            analyze_project([(Path::new("implicit-result.f90"), source.as_slice())]).unwrap();
        let config = FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        };
        let once = format_source_with_context(source, &project, &config)
            .unwrap()
            .bytes;
        let twice = format_source_with_context(&once, &project, &config)
            .unwrap()
            .bytes;
        assert_eq!(twice, once);
        let output = String::from_utf8(once).unwrap();
        assert!(output.contains("function BETA3(x)"));
        assert!(output.contains("real :: BETA3"));
        assert!(output.contains("BETA3 = x"));
        assert!(output.contains("end function BETA3"));
        assert!(output.contains("num = BETA3(x)"));
    }

    #[test]
    fn explicit_function_result_does_not_use_result_spelling_for_calls() {
        let source = b"module m\n\
contains\n\
function BETA3(x) result(ResultValue)\n\
implicit none\n\
real :: x\n\
real :: resultvalue\n\
resultvalue = x\n\
end function beta3\n\
subroutine s(x, num)\n\
real :: x, num\n\
num = bEtA3(x)\n\
end subroutine s\n\
end module m\n";
        let project =
            analyze_project([(Path::new("explicit-result.f90"), source.as_slice())]).unwrap();
        let config = FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        };
        let once = format_source_with_context(source, &project, &config)
            .unwrap()
            .bytes;
        let twice = format_source_with_context(&once, &project, &config)
            .unwrap()
            .bytes;
        assert_eq!(twice, once);
        let output = String::from_utf8(once).unwrap();
        assert!(output.contains("function BETA3(x) result(resultvalue)"));
        assert!(output.contains("resultvalue = x"));
        assert!(output.contains("num = BETA3(x)"));
        assert!(!output.contains("num = ResultValue(x)"));
    }

    #[test]
    fn a_block_declaration_does_not_recase_uses_after_its_end() {
        let source = b"module m\n\
integer :: ModuleVar\n\
contains\n\
subroutine s()\n\
block\n\
integer :: MYVAR\n\
myvar = 1\n\
end block\n\
myvar = 2\n\
modulevar = 3\n\
end\n\
end module m\n";
        let project = analyze_project([(Path::new("block.f90"), source.as_slice())]).unwrap();
        let config = FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        };
        let once = format_source_with_context(source, &project, &config)
            .unwrap()
            .bytes;
        let twice = format_source_with_context(&once, &project, &config)
            .unwrap()
            .bytes;
        assert_eq!(twice, once);
        let output = String::from_utf8(once).unwrap();
        assert!(output.contains("MYVAR = 1"));
        assert!(output.contains("myvar = 2"));
        assert!(output.contains("ModuleVar = 3"));
    }

    #[test]
    fn program_locals_resolve_type_bound_procedure_owners() {
        let declarations = b"module settings\n\
type :: TSettingIni\n\
contains\n\
procedure :: ReadFilename\n\
end type TSettingIni\n\
end module settings\n";
        let source = b"program ExampleApp\n\
use settings\n\
type(TSettingIni) :: Ini\n\
x = Ini%ReadFileName('file_root')\n\
end program ExampleApp\n";
        let project = analyze_project([
            (Path::new("settings.f90"), declarations.as_slice()),
            (Path::new("driver.f90"), source.as_slice()),
        ])
        .unwrap();
        let output = run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        });
        assert!(output
            .windows(b"Ini%ReadFilename".len())
            .any(|window| window == b"Ini%ReadFilename"));
        assert!(!output
            .windows(b"Ini%ReadFileName".len())
            .any(|window| window == b"Ini%ReadFileName"));
    }

    #[test]
    fn use_associated_module_variables_resolve_component_owners() {
        let results = b"module results\n\
type :: ModelData\n\
integer :: MODEL_PK\n\
end type ModelData\n\
end module results\n";
        let gauge = b"module GaugeInterface\n\
use results\n\
class(ModelData), pointer :: State\n\
end module GaugeInterface\n";
        let unrelated = b"module unrelated\n\
type :: Other\n\
integer :: MODEL_Pk\n\
end type Other\n\
type(Other) :: State\n\
end module unrelated\n";
        let source = b"module ExampleMain\n\
use GaugeInterface\n\
use GaugeInterface, only: Active => State\n\
contains\n\
subroutine OtherWork(State)\n\
type(Other) :: State\n\
end subroutine OtherWork\n\
subroutine MakeNonlinearSources\n\
x = State%MODEL_Pk\n\
x = Active%MODEL_Pk\n\
end subroutine MakeNonlinearSources\n\
end module ExampleMain\n";
        let project = analyze_project([
            (Path::new("results.f90"), results.as_slice()),
            (Path::new("equations.f90"), gauge.as_slice()),
            (Path::new("unrelated.f90"), unrelated.as_slice()),
            (Path::new("examplemain.f90"), source.as_slice()),
        ])
        .unwrap();
        assert_eq!(project.types.resolve_chain(b"State", &[]), None);
        let output = run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        });
        assert!(output
            .windows(b"State%MODEL_PK".len())
            .any(|window| window == b"State%MODEL_PK"));
        assert!(!output
            .windows(b"State%MODEL_Pk".len())
            .any(|window| window == b"State%MODEL_Pk"));
        assert!(output
            .windows(b"Active%MODEL_PK".len())
            .any(|window| window == b"Active%MODEL_PK"));
    }

    #[test]
    fn declared_names_do_not_leak_from_type_components() {
        let source = b"module C\ntype Foo\ninteger :: SIZE\nend type Foo\ncontains\nsubroutine report(x)\nreal, intent(in) :: x(:)\nprint *, SIZE(x)\nend subroutine report\nend module C\n";
        let output = crate::format_source(
            source,
            &FormatConfig {
                mode: FormatMode::Full,
                ..FormatConfig::default()
            },
        )
        .unwrap()
        .bytes;
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("print *, size(x)"));
        assert!(!output.contains("print *, SIZE(x)"));
    }

    #[test]
    fn interface_dummies_are_not_module_variables() {
        let source = b"module M\ninterface\nsubroutine ext(ArgCase)\ninteger :: ArgCase\nend subroutine ext\nend interface\ncontains\nsubroutine s\nprint *, argcase\nend subroutine s\nend module M\n";
        let project = analyze_project([(Path::new("interface.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn implicit_identifiers_do_not_borrow_unrelated_project_case() {
        let declarations = b"module globals\ninteger :: i\nend module globals\n";
        let cases = [
            (
                b"subroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            ),
            (
                b"subroutine s(A)\nimplicit none\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\nimplicit none\ndo i = 1, 3\nA(i) = i\nend subroutine s\n".as_slice(),
            ),
            (
                b"subroutine s(A)\nimplicit none(type)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\nimplicit none(type)\ndo i = 1, 3\nA(i) = i\nend subroutine s\n".as_slice(),
            ),
            (
                b"subroutine s(A)\nimplicit none(external)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\nimplicit none(external)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            ),
            (
                b"subroutine host\nimplicit none\ncontains\nsubroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\nend subroutine host\n".as_slice(),
                b"subroutine host\nimplicit none\ncontains\nsubroutine s(A)\ndo i = 1, 3\nA(i) = i\nend subroutine s\nend subroutine host\n".as_slice(),
            ),
            (
                b"module target\nimplicit none\ninterface\nsubroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\nend interface\nend module target\n".as_slice(),
                b"module target\nimplicit none\ninterface\nsubroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\nend interface\nend module target\n".as_slice(),
            ),
            (
                b"subroutine s(A)\nimplicit none(type)\nimplicit integer(i-n)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\nimplicit none(type)\nimplicit integer(i-n)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            ),
            (
                b"subroutine s(A)\nimplicit none(type)\nimplicit real(a-)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\nimplicit none(type)\nimplicit real(A-)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            ),
            (
                b"subroutine s(A)\nimplicit real(a-)\nimplicit none\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
                b"subroutine s(A)\nimplicit real(A-)\nimplicit none\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            ),
        ];

        for (index, (source, expected)) in cases.into_iter().enumerate() {
            let name = format!("case-{index}.f90");
            let project = analyze_project([
                (Path::new("globals.f90"), declarations.as_slice()),
                (Path::new(&name), source),
            ])
            .unwrap();
            assert_eq!(
                run_pass(source, &project, |document, context| {
                    declared(document, context).unwrap()
                }),
                expected,
                "implicit policy case {index}"
            );
        }
    }

    #[test]
    fn implicit_function_syntax_is_guarded_but_call_syntax_is_not() {
        let declarations = b"module globals\ninteger :: xfun\ncontains\nsubroutine xproc(n)\ninteger :: n\nend subroutine xproc\nend module globals\n";
        let source = b"subroutine s(out)\nout = XFUN(3)\ncall XPROC(3)\nend subroutine s\n";
        let project = analyze_project([
            (Path::new("globals.f90"), declarations.as_slice()),
            (Path::new("target.f90"), source.as_slice()),
        ])
        .unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"subroutine s(out)\nout = XFUN(3)\ncall xproc(3)\nend subroutine s\n"
        );
    }

    #[test]
    fn explicit_host_locals_and_use_names_still_canonicalize() {
        let declarations = b"module globals\ninteger :: ProjectName\nend module globals\n";
        let source = b"subroutine host\ninteger :: HostName\ncontains\nsubroutine child\nhostname = 1\nend subroutine child\nend subroutine host\nsubroutine imports\nuse globals, only: projectname\nend subroutine imports\n";
        let project = analyze_project([
            (Path::new("globals.f90"), declarations.as_slice()),
            (Path::new("target.f90"), source.as_slice()),
        ])
        .unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"subroutine host\ninteger :: HostName\ncontains\nsubroutine child\nHostName = 1\nend subroutine child\nend subroutine host\nsubroutine imports\nuse globals, only: ProjectName\nend subroutine imports\n"
        );
    }

    #[test]
    fn unresolved_members_do_not_borrow_other_name_spaces() {
        let sources = [
            (
                Path::new("global.f90"),
                b"type :: ComponentCase\nend type ComponentCase\n".as_slice(),
            ),
            (
                Path::new("components.f90"),
                b"subroutine Work\nreal :: WINDOW\nWINDOW = RedWin%componentcase%Window_f_a(a, winamp)\nend subroutine work\n".as_slice(),
            ),
        ];
        let project = analyze_project(sources).unwrap();
        let source = b"subroutine Work\nreal :: WINDOW\nWINDOW = RedWin%componentcase%Window_f_a(a, winamp)\nend subroutine work\n";
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"subroutine Work\nreal :: WINDOW\nWINDOW = RedWin%componentcase%Window_f_a(a, winamp)\nend subroutine Work\n"
        );
    }

    #[test]
    fn missing_owner_members_do_not_borrow_unrelated_bindings_or_symbols() {
        let declarations = b"module declarations\n\
type :: Other\n\
contains\n\
procedure :: RunCase\n\
end type Other\n\
integer :: ValueCase\n\
end module declarations\n";
        let source = b"program p\n\
type(Unknown) :: item\n\
call item%runcase()\n\
item%valuecase = 1\n\
end program p\n";
        let project = analyze_project([
            (Path::new("declarations.f90"), declarations.as_slice()),
            (Path::new("use.f90"), source.as_slice()),
        ])
        .unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn associate_aliases_propagate_indexed_selector_types_with_lexical_shadowing() {
        let source = b"module SourceWindows\n\
type :: TSourceWindow\n\
contains\n\
procedure :: Window_f_a\n\
end type TSourceWindow\n\
type :: TRedWin\n\
class(TSourceWindow), pointer :: Window\n\
end type TRedWin\n\
type :: ModelData\n\
type(TRedWin), allocatable :: Redshift_W(:)\n\
end type ModelData\n\
type :: Other\n\
integer :: WrongCase\n\
end type Other\n\
contains\n\
subroutine Work(State, OtherState)\n\
class(ModelData) :: State\n\
type(Other) :: OtherState\n\
AssocBlock: associate(UnTyped => UnknownCall(1, 2), RedWin => State%Redshift_W(1))\n\
call RedWin%window%window_F_A()\n\
associate(RedWin => OtherState)\n\
RedWin%wrongcase = 1\n\
end associate\n\
call RedWin%WINDOW%WINDOW_F_A()\n\
end associate AssocBlock\n\
call RedWin%WINDOW%WINDOW_F_A()\n\
end subroutine Work\n\
end module SourceWindows\n";
        let project = analyze_project([(Path::new("associate.f90"), source.as_slice())]).unwrap();
        let output = run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        });
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.matches("RedWin%Window%Window_f_a()").count(), 2);
        assert!(output.contains("RedWin%WrongCase = 1"));
        assert!(output.contains("call RedWin%WINDOW%WINDOW_F_A()\nend subroutine Work"));
    }

    #[test]
    fn module_variables_are_case_matched_without_leaking_local_shadowing() {
        let config = b"module config\ninteger :: FeedbackLevel\ntype :: State\nreal :: transfer_times\nreal :: H0\nend type State\nend module config\n";
        let source = b"module Uses\nuse config\ncontains\nsubroutine Work(Feedbacklevel, H0)\ninteger :: Feedbacklevel\nreal :: H0\ntype(State) :: obj\nprint *, feedbacklevel\nprint *, obj%transfer_times\nend subroutine work\nend module Uses\n";
        let project = analyze_project([
            (Path::new("config.f90"), config.as_slice()),
            (Path::new("uses.f90"), source.as_slice()),
        ])
        .unwrap();
        let output = format_source_with_context(
            source,
            &project,
            &FormatConfig {
                mode: FormatMode::Full,
                ..FormatConfig::default()
            },
        )
        .unwrap()
        .bytes;
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("print *, Feedbacklevel"));
        assert!(!output.contains("print *, FeedbackLevel"));
        assert!(output.contains("obj%transfer_times"));
    }

    #[test]
    fn declaration_entities_are_not_replaced_by_global_symbol_case() {
        let source = b"module M\ninteger :: ERROR\ntype :: T\ncontains\nprocedure :: Error\nend type T\nend module M\n";
        let project = analyze_project([(Path::new("declaration.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn local_type_components_after_module_contains_do_not_leak() {
        let source = b"module m\ncontains\nsubroutine s\ntype :: Local\ninteger :: WeirdCase\nend type Local\nend subroutine s\nend module m\nprogram p\nx = weirdcase\nend program p\n";
        let project = analyze_project([(Path::new("local_type.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn ambiguous_local_and_project_cases_are_silent() {
        let local_source = b"module Foo\nmodule fOO\nuse foo\n";
        let local_project =
            analyze_project([(Path::new("local.f90"), local_source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(local_source, &local_project, |document, context| {
                declared(document, context).unwrap()
            }),
            local_source
        );

        let project = analyze_project([
            (Path::new("a.f90"), b"module Foo\n".as_slice()),
            (Path::new("b.f90"), b"module FOO\n".as_slice()),
        ])
        .unwrap();
        let use_source = b"program p\nuse foo\nend program p\n";
        assert_eq!(
            run_pass(use_source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            use_source
        );
    }

    #[test]
    fn old_style_procedure_headers_supply_local_case_spellings() {
        let source = b"module m\ncontains\nfunction f(this, maxfun)\nclass(*) this\ninteger maxfun\nthis = maxfun\nend function f\nend module m\n";
        let analysis = Document::from_bytes(source).analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        assert_eq!(
            names.local_at(5).and_then(|map| map.get(b"this")),
            Some(b"this".as_slice())
        );
        assert_eq!(
            names.local_at(5).and_then(|map| map.get(b"maxfun")),
            Some(b"maxfun".as_slice())
        );
    }

    #[test]
    fn old_style_declaration_protects_each_comma_separated_entity() {
        let source = b"program p\nreal(dl) kh, Mixed\nend program p\n";
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn explicit_local_declarations_override_continued_header_spelling() {
        let source = b"module m\ncontains\nfunction f(this, maxfun)\nclass(*) THIS\ninteger MAXFUN\nthis = maxfun\nend function f\nend module m\n";
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ncontains\nfunction f(THIS, MAXFUN)\nclass(*) THIS\ninteger MAXFUN\nTHIS = MAXFUN\nend function f\nend module m\n"
        );
    }

    #[test]
    fn associate_aliases_use_an_agreed_project_symbol_case() {
        let source = b"module m\ninteger :: W\ncontains\nsubroutine p\nassociate(w => W)\nx = w\nend associate\nend subroutine p\nend module m\n";
        let project = analyze_project([(Path::new("names.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ninteger :: W\ncontains\nsubroutine p\nassociate(W => W)\nx = W\nend associate\nend subroutine p\nend module m\n"
        );
    }

    #[test]
    fn numeric_kind_suffixes_follow_declared_case_including_exponents() {
        let source = b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_dl\nreal(DL), parameter :: Y = 1.0e8_dl\nend module Constants\n";
        let project = analyze_project([(Path::new("constants.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_DL\nreal(DL), parameter :: Y = 1.0e8_DL\nend module Constants\n"
        );
    }

    #[test]
    fn kind_suffixes_use_project_declarations_and_ignore_digit_kinds() {
        let source = b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\nmodule Values\nuse Kinds\nreal(MyReal) :: x\nx = 1.0_myreal + 2.0_8\nend module Values\n";
        let project = analyze_project([(Path::new("kinds.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\nmodule Values\nuse Kinds\nreal(MyReal) :: x\nx = 1.0_MyReal + 2.0_8\nend module Values\n"
        );
    }

    #[test]
    fn undeclared_kind_suffixes_are_inert() {
        let source = b"program p\nx = 1.0_unknown + 2.0_8\nend program p\n";
        let project = analyze_project([(Path::new("unknown.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn unresolved_same_named_components_are_silent() {
        let source = b"module m\ntype :: first\ninteger :: Source\nend type first\ntype :: second\ninteger :: source\nend type second\nunknown_first%source = 1\nunknown_second%Source = 2\nend module m\n";
        let project = analyze_project([(Path::new("component.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn inherited_components_shadow_and_preserve_nearest_ambiguity() {
        let source = b"module m\n\
type :: Parent\n\
real :: INTEGRATE_TOL\n\
procedure :: ParentRun\n\
real :: Value\n\
end type Parent\n\
type, extends(Parent) :: Child\n\
real :: VALUE\n\
real :: Ambig\n\
real :: AMBIG\n\
end type Child\n\
contains\n\
subroutine work(this)\n\
class(Child) :: this\n\
type(Unknown) :: unknown\n\
this%integrate_tol = 1\n\
this%value = 2\n\
this%ambig = 3\n\
call this%parentrun()\n\
unknown%INTEGRATE_TOL = 4\n\
end subroutine work\n\
end module m\n";
        let project = analyze_project([(Path::new("inheritance.f90"), source.as_slice())]).unwrap();
        assert!(!project
            .cases
            .components
            .contains(b"child", b"integrate_tol"));
        assert!(project.cases.components.contains(b"child", b"value"));
        assert!(project.cases.components.contains(b"child", b"ambig"));
        assert!(project.cases.components.get(b"child", b"ambig").is_none());
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\n\
type :: Parent\n\
real :: INTEGRATE_TOL\n\
procedure :: ParentRun\n\
real :: Value\n\
end type Parent\n\
type, extends(Parent) :: Child\n\
real :: VALUE\n\
real :: Ambig\n\
real :: AMBIG\n\
end type Child\n\
contains\n\
subroutine work(this)\n\
class(Child) :: this\n\
type(Unknown) :: unknown\n\
this%INTEGRATE_TOL = 1\n\
this%VALUE = 2\n\
this%ambig = 3\n\
call this%ParentRun()\n\
unknown%INTEGRATE_TOL = 4\n\
end subroutine work\n\
end module m\n"
        );
    }

    #[test]
    fn powell_bobyqb_has_a_local_case_map() {
        let source = b"module m\ninteger :: MAXFUN\ncontains\nfunction f(THIS, &\n maxfun)\nclass(*) :: this\ninteger :: Maxfun\nthis = maxfun\nend function f\nend module m\n";
        let analysis = Document::from_bytes(source).analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        let line = source
            .split(|byte| *byte == b'\n')
            .position(|line| line.starts_with(b"function f"))
            .unwrap();
        assert_eq!(
            names.local_at(line).and_then(|map| map.get(b"this")),
            Some(b"this".as_slice())
        );
        assert_eq!(
            names.local_at(line).and_then(|map| map.get(b"maxfun")),
            Some(b"Maxfun".as_slice())
        );
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ninteger :: MAXFUN\ncontains\nfunction f(this, &\n Maxfun)\nclass(*) :: this\ninteger :: Maxfun\nthis = Maxfun\nend function f\nend module m\n"
        );
    }

    #[test]
    fn nested_declaration_bounds_use_the_active_procedure_local_case() {
        let source = b"module m\ncontains\nsubroutine first(EV)\ntype(EvolutionVars) EV, EVout\nreal(dl), intent(out) :: yout(EVOut%nvar)\nend subroutine first\nsubroutine second(EV)\ntype(EvolutionVars) EV, EVOut\nreal(dl), intent(out) :: yout(EVout%nvar)\nend subroutine second\nend module m\n";
        let analysis = Document::from_bytes(source).analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        assert_eq!(
            names.local_at(4).and_then(|map| map.get(b"evout")),
            Some(b"EVout".as_slice())
        );
        assert_eq!(
            names.local_at(8).and_then(|map| map.get(b"evout")),
            Some(b"EVOut".as_slice())
        );
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ncontains\nsubroutine first(EV)\ntype(EvolutionVars) EV, EVout\nreal(dl), intent(out) :: yout(EVout%nvar)\nend subroutine first\nsubroutine second(EV)\ntype(EvolutionVars) EV, EVOut\nreal(dl), intent(out) :: yout(EVOut%nvar)\nend subroutine second\nend module m\n"
        );
    }
}
