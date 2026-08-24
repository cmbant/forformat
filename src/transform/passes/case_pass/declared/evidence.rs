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
