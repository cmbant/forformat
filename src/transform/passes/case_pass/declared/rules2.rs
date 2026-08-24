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
