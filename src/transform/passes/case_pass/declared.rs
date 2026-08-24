//! Declared-name spelling engine and occurrence classification.

use super::{
    associations::{
        apply_select_guard, associate_spelling, association_opening_scope,
        is_associate_alias_declaration, is_select_alias_declaration, is_select_type_rank_keyword,
        select_association_spec, AssociateFrame, AssociationScope,
    },
    members::{
        component_owner_names, exact_member_owner, inherited_component_spelling,
        inherited_type_procedure_spelling, member_owner_type,
    },
    syntax::{
        active_procedure, implicit_guard_applies, is_declaration_entity, is_external_reference,
        is_intrinsic_kind_name, is_numeric_literal_kind_name, is_type_spec_name, is_use_intrinsic,
        is_use_module, is_use_only_keyword, is_use_rename_local, is_use_statement, named_end_space,
        preceded_by_percent, scope_header_space, use_module_index,
    },
};
use crate::{
    analysis::{
        names::{resolve, NameSpace},
        project::ResolvedType,
        CaseMap, DeclaredNameIndex, DeclaredSpelling,
    },
    classify::{classify, StatementKind},
    error::FormatError,
    source::{
        tokens::{tokenize, Token, TokenKind},
        LexState,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::provenance::{source_spans, spread_replacement},
        pipeline::{Changed, PassContext},
    },
};
use std::{collections::HashMap, ops::Range};

#[cfg(test)]
use crate::analysis::scoped_declared_names;

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

fn declared_with_names_impl(
    document: &mut Document,
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    mut evidence_map: Option<&mut CaseEvidenceMap>,
) -> Result<Changed, FormatError> {
    let procedure_spellings = implicit_function_spellings(cx.analysis, declared_names);
    let mut association_stack: Vec<AssociationScope> = Vec::new();
    let mut line_edits: Vec<Vec<(Range<usize>, Vec<u8>)>> = vec![Vec::new(); document.lines.len()];
    let record_evidence = evidence_map.is_some();

    for group in &cx.analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            let statement_kind = classify(&statement.text).kind;
            let first = tokens
                .iter()
                .position(|token| token.kind != TokenKind::Number);
            let mut associate_context = AssociateFrame::default();
            for scope in &association_stack {
                associate_context.extend_visible(scope.frame());
            }
            let opening_scope = matches!(
                statement_kind,
                StatementKind::Associate | StatementKind::Select
            )
            .then(|| {
                association_opening_scope(
                    &tokens,
                    first,
                    group.lines.start,
                    active_procedure(cx.scopes, group.lines.start),
                    cx,
                    &associate_context,
                )
            })
            .flatten();

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
                    if !cx.project.macros.contains(token.text) {
                        reconcile_occurrence_evidence(
                            &tokens,
                            index,
                            &associate_context,
                            &mut token_evidence,
                        );
                    }
                    if !matches!(token_evidence, CaseEvidence::KeepBase) {
                        if let Some(map) = evidence_map.as_deref_mut() {
                            map.insert((line, first_span.start), token_evidence);
                        }
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
            let closes_associate = statement_kind == StatementKind::EndAssociate
                && matches!(
                    association_stack.last(),
                    Some(AssociationScope::Associate(_))
                );
            let closes_select = statement_kind == StatementKind::EndSelect
                && matches!(
                    association_stack.last(),
                    Some(AssociationScope::Select { .. })
                );
            if closes_associate || closes_select {
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

#[derive(Debug)]
enum RuleMatch {
    Miss,
    Decision(Option<Vec<u8>>),
}

/// Classify one identifier occurrence and return its canonical spelling.
///
/// The ordered rules mirror the formatter's namespace precedence. Each helper
/// owns one syntactic namespace, so adding a new shape no longer extends one
/// large mutually-exclusive `if` ladder.
fn classify_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &DeclaredNameIndex,
    cx: &PassContext,
    context: ClassificationContext<'_>,
) -> Option<Vec<u8>> {
    let ClassificationContext {
        associates,
        procedure_spellings,
        mut evidence,
    } = context;
    let token = &tokens[index];
    let associate_alias = associates.is_some_and(|context| {
        context
            .names
            .contains(token.text.to_ascii_lowercase().as_slice())
    });

    if let RuleMatch::Decision(spelling) = protected_spelling(tokens, index, cx) {
        return spelling;
    }
    if let RuleMatch::Decision(spelling) = numeric_kind_spelling(
        tokens,
        index,
        line,
        declared_names,
        cx,
        associate_alias,
        &mut evidence,
    ) {
        return spelling;
    }
    if let RuleMatch::Decision(spelling) =
        declaration_spelling(tokens, index, line, declared_names, cx, procedure_spellings)
    {
        return spelling;
    }
    if let RuleMatch::Decision(spelling) = scoped_name_spelling(tokens, index, cx) {
        return spelling;
    }
    if let RuleMatch::Decision(spelling) = type_name_spelling(tokens, index, cx, &mut evidence) {
        return spelling;
    }
    if let RuleMatch::Decision(spelling) = intrinsic_kind_spelling(
        tokens,
        index,
        line,
        declared_names,
        cx,
        associate_alias,
        &mut evidence,
    ) {
        return spelling;
    }
    if let RuleMatch::Decision(spelling) =
        member_spelling(tokens, index, line, cx, associates, &mut evidence)
    {
        return spelling;
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

fn protected_spelling(tokens: &[Token<'_>], index: usize, cx: &PassContext) -> RuleMatch {
    let token = &tokens[index];
    if is_select_type_rank_keyword(tokens, index)
        || crate::source::syntax::is_end_construct_keyword(tokens, index)
        || (index > 0 && crate::source::syntax::is_end_construct_keyword(tokens, index - 1))
    {
        return RuleMatch::Decision(None);
    }
    if cx.project.macros.contains(token.text) {
        return RuleMatch::Decision(None);
    }
    RuleMatch::Miss
}

fn numeric_kind_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &DeclaredNameIndex,
    cx: &PassContext,
    associate_alias: bool,
    evidence: &mut Option<&mut CaseEvidence>,
) -> RuleMatch {
    if !is_numeric_literal_kind_name(tokens, index) {
        return RuleMatch::Miss;
    }
    record_case_evidence(
        evidence,
        CaseEvidence::Symbol {
            allow_external: false,
        },
    );
    RuleMatch::Decision(file_symbol_spelling(
        declared_names,
        cx,
        tokens[index].text,
        SymbolQuery {
            line,
            associate_alias,
            implicit_guard: ImplicitGuard::Skip,
        },
    ))
}

fn declaration_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &DeclaredNameIndex,
    cx: &PassContext,
    procedure_spellings: Option<&CaseMap>,
) -> RuleMatch {
    if let Some(spelling) =
        procedure_definition_spelling(tokens, index, line, declared_names, procedure_spellings)
    {
        return RuleMatch::Decision(Some(spelling));
    }
    if is_declaration_entity(tokens, index) {
        return RuleMatch::Decision(implicit_result_spelling(
            cx,
            line,
            &tokens[index],
            procedure_spellings,
        ));
    }
    RuleMatch::Miss
}

fn scoped_name_spelling(tokens: &[Token<'_>], index: usize, cx: &PassContext) -> RuleMatch {
    let token = &tokens[index];
    if let Some(space) = named_end_space(tokens, index) {
        return RuleMatch::Decision(resolver_spelling(cx, space, token.text));
    }
    if let Some(space) = scope_header_space(tokens, index) {
        return RuleMatch::Decision(resolver_spelling(cx, space, token.text));
    }
    if is_use_module(tokens, index) {
        return RuleMatch::Decision(resolver_spelling(cx, NameSpace::Module, token.text));
    }
    RuleMatch::Miss
}

fn type_name_spelling(
    tokens: &[Token<'_>],
    index: usize,
    cx: &PassContext,
    evidence: &mut Option<&mut CaseEvidence>,
) -> RuleMatch {
    if !is_type_spec_name(tokens, index) {
        return RuleMatch::Miss;
    }
    let token = &tokens[index];
    record_case_evidence(evidence, CaseEvidence::Type);
    let spelling = if cx.local.declared_types.contains(token.text)
        || cx.project.declared_types.contains(token.text)
    {
        resolve(
            &cx.local.declared_types,
            &cx.project.declared_types,
            token.text,
        )
        .map(ToOwned::to_owned)
    } else {
        None
    };
    RuleMatch::Decision(spelling)
}

fn intrinsic_kind_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &DeclaredNameIndex,
    cx: &PassContext,
    associate_alias: bool,
    evidence: &mut Option<&mut CaseEvidence>,
) -> RuleMatch {
    if !is_intrinsic_kind_name(tokens, index) {
        return RuleMatch::Miss;
    }
    record_case_evidence(
        evidence,
        CaseEvidence::Symbol {
            allow_external: false,
        },
    );
    RuleMatch::Decision(file_symbol_spelling(
        declared_names,
        cx,
        tokens[index].text,
        SymbolQuery {
            line,
            associate_alias,
            implicit_guard: ImplicitGuard::Skip,
        },
    ))
}

fn member_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    cx: &PassContext,
    associates: Option<&AssociateFrame>,
    evidence: &mut Option<&mut CaseEvidence>,
) -> RuleMatch {
    if !preceded_by_percent(tokens, index) {
        return RuleMatch::Miss;
    }
    record_member_evidence(tokens, index, line, cx, associates, evidence);
    let procedure = active_procedure(cx.scopes, line);
    let Some(owner_type) = member_owner_type(
        tokens,
        index,
        procedure,
        cx.local,
        Some(&cx.project.types),
        true,
        associates,
    ) else {
        return RuleMatch::Decision(None);
    };
    let token = &tokens[index];
    if let Some(spelling) = inherited_component_spelling(cx, &owner_type, token.text, true) {
        return RuleMatch::Decision(Some(spelling));
    }
    RuleMatch::Decision(inherited_type_procedure_spelling(
        cx,
        &owner_type,
        token.text,
    ))
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

fn procedure_definition_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &DeclaredNameIndex,
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
    declared_names: &DeclaredNameIndex,
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
    declared_names: &DeclaredNameIndex,
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

fn file_symbol_spelling(
    declared_names: &DeclaredNameIndex,
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
