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
