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
