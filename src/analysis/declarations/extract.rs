use super::{
    facts::{FileFacts, UseAssociation},
    syntax::{
        is_old_style_type_context, is_type_definition, old_style_type_name, select_type_alias,
        type_definition_parent, type_spec_name,
    },
    types::TypeMaps,
};
use crate::{
    analysis::{
        names::{CaseMap, CaseTables},
        scope::{ScopeKind, ScopeTree},
    },
    source::{
        tokens::{tokenize, TokenKind},
        LexState, PhysicalLineKind,
    },
    transform::document::Analysis,
};

/// Extract every declaration fact from one analyzed file.
pub fn extract(analysis: &Analysis, scopes: &ScopeTree) -> FileFacts {
    let mut facts = FileFacts::default();
    scope_names(
        scopes,
        &mut facts.cases,
        &mut facts.file_symbols,
        &mut facts.declared_types,
    );
    for group in &analysis.groups {
        let first = &analysis.buffer.lines[group.lines.start];
        if first.kind == PhysicalLineKind::Preprocessor {
            for line in group.lines.clone() {
                define_name(
                    analysis.buffer.line_bytes(&analysis.buffer.lines[line]),
                    &mut facts,
                );
            }
            continue;
        }
        let owner = scopes.enclosing_type(group.lines.start).map(|scope| {
            scope
                .name
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
        });
        let procedure_scope = scopes
            .ancestors(scopes.index_of_line(group.lines.start))
            .into_iter()
            .find(|scope| is_procedure_scope(scopes.scopes[*scope].kind));
        let procedure = procedure_scope
            .and_then(|scope| scopes.scopes[scope].name.as_deref())
            .map(|name| name.to_ascii_lowercase());
        let file_specification_scope = scopes
            .ancestors(scopes.index_of_line(group.lines.start))
            .into_iter()
            .find(|scope| {
                matches!(
                    scopes.scopes[*scope].kind,
                    ScopeKind::Module | ScopeKind::Program
                )
            })
            .filter(|scope| scopes.scopes[*scope].is_specification(group.lines.start));
        // A program's top-level specification is file-wide like a module's.
        // Only a distinct nested procedure suppresses promotion to file facts.
        let file_scope_declaration = file_specification_scope
            .is_some_and(|scope| procedure_scope.is_none_or(|procedure| procedure == scope));
        let declaring_module = file_specification_scope
            .filter(|scope| scopes.scopes[*scope].kind == ScopeKind::Module)
            .and_then(|scope| scopes.scopes[scope].name.as_deref())
            .map(|name| name.to_ascii_lowercase());

        for statement in &group.statements {
            if let Some((child, parent)) = type_definition_parent(&statement.text) {
                facts.types.insert_parent(child, parent);
            }
            use_statement(
                &statement.text,
                &mut facts.cases.symbols,
                &mut facts.imports,
            );
            auxiliary_declaration(&statement.text, &mut facts.cases.symbols);
            entity_declaration(
                &statement.text,
                owner.as_deref(),
                procedure.as_deref(),
                file_scope_declaration,
                declaring_module.as_deref(),
                &mut facts,
            );
            if let Some(alias) = select_type_alias(&statement.text) {
                if let Some(selector_type) =
                    selector_type(&statement.text, &facts.types, procedure.as_deref())
                {
                    if let Some(procedure) = procedure.as_deref() {
                        facts
                            .types
                            .insert_procedure_local(procedure, &alias, &selector_type);
                    } else {
                        facts.types.insert_local(&alias, &selector_type);
                    }
                }
            }
        }
    }
    facts
}

fn is_procedure_scope(kind: ScopeKind) -> bool {
    matches!(kind, ScopeKind::Program | ScopeKind::Procedure)
}

/// Ordinary variable names declared by one statement. Derived-type
/// declarations and procedure bindings live in other namespaces.
fn entity_declaration(
    text: &[u8],
    owner: Option<&[u8]>,
    procedure: Option<&[u8]>,
    file_scope_declaration: bool,
    declaring_module: Option<&[u8]>,
    facts: &mut FileFacts,
) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    let first = &tokens[first_index];
    if first.kind != TokenKind::Name || first.is(b"use") {
        return;
    }
    if is_type_definition(&tokens, first_index) {
        return;
    }
    let Some(separator) = tokens
        .iter()
        .position(|t| t.depth == 0 && t.kind == TokenKind::Operator && t.text == b"::")
    else {
        old_style_declaration(
            &tokens,
            first_index,
            owner,
            procedure,
            file_scope_declaration,
            declaring_module,
            facts,
        );
        return;
    };

    let declared_type = (first.is(b"type") || first.is(b"class"))
        .then(|| type_spec_name(&tokens, first_index, separator))
        .flatten()
        .map(|name| {
            facts.cases.types.insert(name);
            name.to_ascii_lowercase()
        });

    let bound_procedure =
        owner.is_some() && (first.is(b"procedure") || first.is(b"generic") || first.is(b"final"));

    let mut expect_name = true;
    for token in &tokens[separator + 1..] {
        if token.depth > 0 {
            continue;
        }
        match token.kind {
            TokenKind::Comma => expect_name = true,
            TokenKind::Name if expect_name => {
                expect_name = false;
                if bound_procedure {
                    facts.cases.type_procedures.insert(token.text);
                    if !first.is(b"generic") {
                        if let Some(owner) = owner {
                            facts.cases.bound_type_procedures.insert(owner, token.text);
                        }
                    }
                    if first.is(b"generic") {
                        facts.generic_type_procedures.insert(token.text);
                        if let Some(owner) = owner {
                            facts
                                .generic_bound_type_procedures
                                .insert(owner, token.text);
                        }
                    }
                    continue;
                }
                match (owner, &declared_type) {
                    (Some(owner), declared) => {
                        facts.cases.components.insert(owner, token.text);
                        facts.cases.symbols.insert(token.text);
                        if file_scope_declaration {
                            facts.file_symbols.insert(token.text);
                        }
                        if let Some(declared) = declared {
                            facts.types.insert_component(owner, token.text, declared);
                        }
                    }
                    (None, declared) => {
                        facts.cases.symbols.insert(token.text);
                        if file_scope_declaration {
                            facts.file_symbols.insert(token.text);
                        }
                        if let Some(declared) = declared {
                            if let Some(procedure) = procedure {
                                facts
                                    .types
                                    .insert_procedure_local(procedure, token.text, declared);
                            } else {
                                facts.types.insert_variable(token.text, declared);
                            }
                            if let Some(module) = declaring_module {
                                facts
                                    .types
                                    .insert_module_variable(module, token.text, declared);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn old_style_declaration(
    tokens: &[crate::source::Token<'_>],
    first_index: usize,
    owner: Option<&[u8]>,
    procedure: Option<&[u8]>,
    file_scope_declaration: bool,
    declaring_module: Option<&[u8]>,
    facts: &mut FileFacts,
) {
    let first = &tokens[first_index];
    let declaration = matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer" | b"real" | b"complex" | b"logical" | b"character" | b"type" | b"class"
    ) || first.is(b"double")
        && tokens
            .get(first_index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    if !declaration || is_old_style_type_context(tokens, first_index) {
        return;
    }
    let mut expect_name = true;
    let mut initializer = false;
    let entity_start = first_index
        + 1
        + usize::from(
            first.is(b"double")
                && tokens
                    .get(first_index + 1)
                    .is_some_and(|token| token.is_name(b"precision")),
        );
    if tokens
        .iter()
        .skip(entity_start)
        .find(|token| token.kind == TokenKind::Name && token.depth == 0)
        .is_some_and(|token| token.is_name(b"function"))
    {
        return;
    }
    let declared_type = if first.is(b"type") || first.is(b"class") {
        old_style_type_name(tokens, first_index).map(|token| {
            facts.cases.types.insert(token);
            token.to_ascii_lowercase()
        })
    } else {
        None
    };
    for token in tokens.iter().skip(entity_start) {
        if token.depth > 0 {
            continue;
        }
        if token.text == b"=" || token.text == b"=>" {
            initializer = true;
            continue;
        }
        if token.kind == TokenKind::Comma {
            initializer = false;
            expect_name = true;
            continue;
        }
        if !initializer && expect_name && token.kind == TokenKind::Name {
            expect_name = false;
            if let Some(owner) = owner {
                facts.cases.components.insert(owner, token.text);
                facts.cases.symbols.insert(token.text);
                if file_scope_declaration {
                    facts.file_symbols.insert(token.text);
                }
                if let Some(declared_type) = &declared_type {
                    facts
                        .types
                        .insert_component(owner, token.text, declared_type);
                }
            } else {
                facts.cases.symbols.insert(token.text);
                if file_scope_declaration {
                    facts.file_symbols.insert(token.text);
                }
                if let Some(declared_type) = &declared_type {
                    if let Some(procedure) = procedure {
                        facts
                            .types
                            .insert_procedure_local(procedure, token.text, declared_type);
                    } else {
                        facts.types.insert_variable(token.text, declared_type);
                    }
                    if let Some(module) = declaring_module {
                        facts
                            .types
                            .insert_module_variable(module, token.text, declared_type);
                    }
                }
            }
        }
    }
}

fn scope_names(
    scopes: &ScopeTree,
    cases: &mut CaseTables,
    file_symbols: &mut CaseMap,
    declared_types: &mut CaseMap,
) {
    for scope in &scopes.scopes {
        let Some(name) = scope.name.as_deref() else {
            continue;
        };
        match scope.kind {
            ScopeKind::Module | ScopeKind::Submodule => cases.modules.insert(name),
            ScopeKind::Program | ScopeKind::Procedure => {
                cases.symbols.insert(name);
                file_symbols.insert(name);
            }
            ScopeKind::DerivedType => {
                cases.types.insert(name);
                cases.symbols.insert(name);
                file_symbols.insert(name);
                declared_types.insert(name);
            }
            ScopeKind::File | ScopeKind::Interface | ScopeKind::Construct => {}
        }
    }
}

/// Record authored USE names and the association needed for imported type lookup.
fn use_statement(text: &[u8], symbols: &mut CaseMap, imports: &mut Vec<UseAssociation>) {
    let tokens = tokenize(text, &mut LexState::default());
    let first = usize::from(tokens.first().is_some_and(|t| t.kind == TokenKind::Number));
    if !tokens.get(first).is_some_and(|t| t.is_name(b"use")) {
        return;
    }
    let separator = tokens
        .iter()
        .enumerate()
        .skip(first + 1)
        .find(|(_, token)| token.depth == 0 && token.text == b"::")
        .map(|(index, _)| index);
    let module_start = separator.map_or(first + 1, |index| index + 1);
    let Some((module_index, module)) = tokens
        .iter()
        .enumerate()
        .skip(module_start)
        .find(|(_, token)| token.depth == 0 && token.kind == TokenKind::Name)
    else {
        return;
    };
    let only = tokens
        .iter()
        .enumerate()
        .skip(module_index + 1)
        .find(|(_, token)| token.depth == 0 && token.is_name(b"only"))
        .and_then(|(index, _)| {
            tokens
                .get(index + 1)
                .is_some_and(|token| token.text == b":")
                .then_some(index)
        });
    let list_start = only.map_or(module_index + 1, |index| index + 2);
    let mut association = UseAssociation {
        module: module.text.to_ascii_lowercase(),
        only: only.is_some(),
        names: Vec::new(),
    };

    let mut item_start = list_start;
    for item_end in (list_start..=tokens.len()).filter(|index| {
        *index == tokens.len()
            || tokens[*index].depth == 0 && tokens[*index].kind == TokenKind::Comma
    }) {
        let item = &tokens[item_start..item_end];
        let arrow = item.iter().position(|token| token.text == b"=>");
        let local = item
            .iter()
            .take(arrow.unwrap_or(item.len()))
            .find(|token| token.depth == 0 && token.kind == TokenKind::Name);
        let remote = arrow.and_then(|arrow| {
            item.iter()
                .skip(arrow + 1)
                .find(|token| token.depth == 0 && token.kind == TokenKind::Name)
        });
        if let Some(local) = local {
            symbols.insert(local.text);
            let remote = remote.unwrap_or(local);
            symbols.insert(remote.text);
            association.names.push((
                local.text.to_ascii_lowercase(),
                remote.text.to_ascii_lowercase(),
            ));
        }
        item_start = item_end.saturating_add(1);
    }
    imports.push(association);
}

/// Statement forms whose entities are names but do not have a type-spec `::`.
fn auxiliary_declaration(text: &[u8], symbols: &mut CaseMap) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    let keyword = &tokens[first];
    if keyword.is_name(b"entry") {
        if let Some(name) = tokens
            .get(first + 1)
            .filter(|token| token.kind == TokenKind::Name)
        {
            symbols.insert(name.text);
        }
        return;
    }
    if !(keyword.is_name(b"external") || keyword.is_name(b"intrinsic")) {
        if !(keyword.is_name(b"common") || keyword.is_name(b"namelist")) {
            return;
        }
        let mut slash_count = 0;
        let mut in_names = false;
        for token in tokens.iter().skip(first + 1) {
            if token.depth != 0 {
                continue;
            }
            if token.text == b"/" {
                slash_count += 1;
                in_names = slash_count % 2 == 0;
                continue;
            }
            if token.kind == TokenKind::Name && (in_names || slash_count == 0 || slash_count == 1) {
                symbols.insert(token.text);
            }
        }
        return;
    }
    let start = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")
        .map_or(first + 1, |separator| separator + 1);
    let mut expect_name = true;
    for token in tokens.iter().skip(start) {
        if token.depth != 0 {
            continue;
        }
        if token.kind == TokenKind::Comma {
            expect_name = true;
        } else if token.kind == TokenKind::Name && expect_name {
            symbols.insert(token.text);
            expect_name = false;
        }
    }
}

/// Resolve the selector of `SELECT TYPE (alias => selector)` using type facts
/// accumulated earlier in the file.
fn selector_type(text: &[u8], types: &TypeMaps, procedure: Option<&[u8]>) -> Option<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let select = tokens.iter().position(|token| token.is_name(b"select"))?;
    if !tokens
        .get(select + 1)
        .is_some_and(|token| token.is_name(b"type"))
        || !tokens
            .get(select + 2)
            .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        return None;
    }
    let arrow = tokens
        .iter()
        .enumerate()
        .skip(select + 3)
        .find(|(_, token)| token.depth == 1 && token.text == b"=>")
        .map(|(index, _)| index)?;
    let root = tokens
        .get(arrow + 1)
        .filter(|token| token.kind == TokenKind::Name)?;
    let mut links = Vec::new();
    let mut index = arrow + 2;
    while let Some(percent) = tokens.get(index) {
        if percent.text != b"%" {
            break;
        }
        let link = tokens
            .get(index + 1)
            .filter(|token| token.kind == TokenKind::Name)?;
        links.push(link.text);
        index += 2;
    }
    // Indexed components are deliberately unresolved; exact owner type is not
    // known well enough to authorize a case rewrite after the alias.
    if tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        return None;
    }
    types.resolve_chain_with_locals(procedure, root.text, &links)
}

/// `#define NAME` and `#define NAME(args)`.
fn define_name(line: &[u8], facts: &mut FileFacts) {
    let mut rest = line;
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    if !rest.starts_with(b"#") {
        return;
    }
    rest = &rest[1..];
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    if rest.len() < 6 || !rest[..6].eq_ignore_ascii_case(b"define") {
        return;
    }
    rest = &rest[6..];
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    let end = rest
        .iter()
        .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'))
        .unwrap_or(rest.len());
    if end > 0 && rest[0].is_ascii_alphabetic() {
        facts.macros.insert(&rest[..end]);
    }
}
