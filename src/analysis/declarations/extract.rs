use super::{
    facts::{FileFacts, IncludeDirective},
    syntax::{
        is_old_style_type_context, is_type_definition, old_style_type_name, select_type_alias,
        type_definition_parent, type_spec_name,
    },
    types::TypeMaps,
    Accessibility, HostAccess, HostUnit, UnitFacts, UseAssociation, UseName,
};
use crate::{
    analysis::{
        names::CaseMap,
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
    initialize_units(scopes, &mut facts);
    scope_names(scopes, &mut facts);
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
        let ancestors = scopes.ancestors(scopes.index_of_line(group.lines.start));
        let unit_scope = ancestors
            .iter()
            .copied()
            .find(|scope| owns_declarations(scopes.scopes[*scope].kind))
            .unwrap_or(0);
        let owner = scopes.enclosing_type(group.lines.start).map(|scope| {
            scope
                .name
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
        });
        let procedure_scope = ancestors
            .iter()
            .copied()
            .find(|scope| is_procedure_scope(scopes.scopes[*scope].kind));
        let procedure = procedure_scope
            .and_then(|scope| scopes.scopes[scope].name.as_deref())
            .map(|name| name.to_ascii_lowercase());
        let file_specification_scope = ancestors
            .iter()
            .copied()
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
            scope_header_semantics(&statement.text, unit_scope, scopes, &mut facts);
            import_statement(&statement.text, unit_scope, &mut facts);
            include_statement(&statement.text, group.lines.start, &mut facts.includes);
            // A bare PRIVATE inside a derived type sets component accessibility
            // and must not touch the module's own default, so the access
            // statement stays gated on being outside a type body. The
            // accessibility attribute of `TYPE, PUBLIC :: T` does not: that
            // statement opens the type it names, so it is always seen with the
            // type already entered.
            if owner.is_none() {
                access_statement(&statement.text, unit_scope, &mut facts);
            }
            type_definition_access(&statement.text, unit_scope, &mut facts);
            if let Some((child, parent)) = type_definition_parent(&statement.text) {
                facts.types.insert_parent(child, parent);
                if let Some(unit) = facts.units.get_mut(&unit_scope) {
                    unit.type_graph.insert_parent(child, parent);
                }
            }
            if let Some(association) = use_statement(&statement.text, &mut facts.cases.symbols) {
                if let Some(unit) = facts.units.get_mut(&unit_scope) {
                    unit.imports.push(association);
                }
            }
            if let Some(unit) = facts.units.get_mut(&unit_scope) {
                auxiliary_declaration(&statement.text, &mut facts.cases.symbols, &mut unit.symbols);
            }
            entity_declaration(
                &statement.text,
                owner.as_deref(),
                procedure.as_deref(),
                unit_scope,
                file_scope_declaration,
                declaring_module.as_deref(),
                &mut facts,
            );
            if let Some(alias) = select_type_alias(&statement.text) {
                if let Some(selector_type) =
                    selector_type(&statement.text, &facts.types, procedure.as_deref())
                {
                    if let Some(unit) = facts.units.get_mut(&unit_scope) {
                        unit.symbols.insert(&alias);
                        unit.insert_variable_type(&alias, &selector_type);
                    }
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

fn owns_declarations(kind: ScopeKind) -> bool {
    kind == ScopeKind::File || kind.is_program_unit() || kind == ScopeKind::Construct
}

fn initialize_units(scopes: &ScopeTree, facts: &mut FileFacts) {
    for (scope_index, scope) in scopes.scopes.iter().enumerate() {
        if !owns_declarations(scope.kind) {
            continue;
        }
        let mut parent = scope.parent;
        while parent.is_some_and(|candidate| !owns_declarations(scopes.scopes[candidate].kind)) {
            parent = parent.and_then(|candidate| scopes.scopes[candidate].parent);
        }
        let mut unit = UnitFacts::new(
            scope_index,
            scope.kind,
            scope.name.as_deref().map(|name| name.to_ascii_lowercase()),
            parent,
            scope.lines.clone(),
        );
        if scope.kind == ScopeKind::Procedure
            && scope
                .parent
                .is_some_and(|parent| scopes.scopes[parent].kind == ScopeKind::Interface)
        {
            unit.host_access = HostAccess::none_by_default();
        }
        facts.units.insert(scope_index, unit);
    }
}

fn scope_header_semantics(
    text: &[u8],
    unit_scope: usize,
    scopes: &ScopeTree,
    facts: &mut FileFacts,
) {
    let Some(scope) = scopes.scopes.get(unit_scope) else {
        return;
    };
    if scope.kind == ScopeKind::Submodule {
        if let Some((identity, host)) = submodule_header(text) {
            if let Some(unit) = facts.units.get_mut(&unit_scope) {
                unit.project_host = Some(identity);
                unit.semantic_host = Some(host);
            }
        }
    }
    if scope.kind == ScopeKind::Procedure
        && scope
            .parent
            .is_some_and(|parent| scopes.scopes[parent].kind == ScopeKind::Interface)
        && module_procedure_interface_header(text)
    {
        if let Some(unit) = facts.units.get_mut(&unit_scope) {
            unit.host_access.set_default_all();
        }
    }
}

fn submodule_header(text: &[u8]) -> Option<(HostUnit, HostUnit)> {
    let tokens = tokenize(text, &mut LexState::default());
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens[first].is_name(b"submodule") {
        return None;
    }
    let open = tokens[first + 1..]
        .iter()
        .position(|token| token.text == b"(")?
        + first
        + 1;
    let close = tokens[open + 1..]
        .iter()
        .position(|token| token.text == b")")?
        + open
        + 1;
    let ancestor = tokens[open + 1..close]
        .iter()
        .find(|token| token.kind == TokenKind::Name)?
        .text
        .to_ascii_lowercase();
    let parent = tokens[open + 1..close]
        .iter()
        .position(|token| token.text == b":")
        .and_then(|colon| {
            tokens[open + 1 + colon + 1..close]
                .iter()
                .find(|token| token.kind == TokenKind::Name)
        })
        .map(|token| token.text.to_ascii_lowercase());
    let child = tokens[close + 1..]
        .iter()
        .find(|token| token.kind == TokenKind::Name)?
        .text
        .to_ascii_lowercase();
    let identity = HostUnit::Submodule {
        ancestor: ancestor.clone(),
        name: child,
    };
    let host = parent.map_or_else(
        || HostUnit::Module(ancestor.clone()),
        |name| HostUnit::Submodule {
            ancestor: ancestor.clone(),
            name,
        },
    );
    Some((identity, host))
}

fn module_procedure_interface_header(text: &[u8]) -> bool {
    let tokens = tokenize(text, &mut LexState::default());
    let mut saw_module = false;
    for token in tokens.iter().filter(|token| token.depth == 0) {
        if token.kind != TokenKind::Name {
            continue;
        }
        if token.is_name(b"module") {
            saw_module = true;
        }
        if token.is_name(b"subroutine") || token.is_name(b"function") {
            return saw_module;
        }
    }
    false
}

fn import_statement(text: &[u8], unit_scope: usize, facts: &mut FileFacts) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    if !tokens[first].is_name(b"import") {
        return;
    }
    let Some(unit) = facts.units.get_mut(&unit_scope) else {
        return;
    };
    // `IMPORT, ALL` and `IMPORT :: All` are different productions, not one
    // production with a special name in it: the host-access forms are
    // `IMPORT, NONE`, `IMPORT, ALL` and `IMPORT, ONLY : list`, while
    // `IMPORT [[ :: ] name-list]` names entities. Only the comma tells them
    // apart, so a scope that imports an entity spelled `all` must not be read
    // as importing everything.
    let comma_qualified = tokens[first + 1..]
        .iter()
        .find(|token| {
            token.depth == 0 && !matches!(token.kind, TokenKind::Ampersand | TokenKind::Comment)
        })
        .is_some_and(|token| token.kind == TokenKind::Comma);
    let qualifier = comma_qualified
        .then(|| {
            tokens[first + 1..]
                .iter()
                .find(|token| token.depth == 0 && token.kind == TokenKind::Name)
        })
        .flatten();
    if qualifier.is_some_and(|token| token.is_name(b"none")) {
        unit.host_access.import_none();
        return;
    }
    if qualifier.is_some_and(|token| token.is_name(b"all")) {
        unit.host_access.import_all();
        return;
    }
    let only = qualifier.is_some_and(|token| token.is_name(b"only"));
    let start = if only {
        tokens
            .iter()
            .position(|token| token.depth == 0 && token.text == b":")
            .map_or(tokens.len(), |colon| colon + 1)
    } else {
        tokens
            .iter()
            .position(|token| token.depth == 0 && token.text == b"::")
            .map_or(first + 1, |separator| separator + 1)
    };
    let names = tokens[start..]
        .iter()
        .filter(|token| token.depth == 0 && token.kind == TokenKind::Name)
        .map(|token| token.text.to_vec())
        .collect::<Vec<_>>();
    if names.is_empty() && !only {
        unit.host_access.import_all();
    } else {
        unit.host_access.import_only(names);
    }
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
    unit_scope: usize,
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
    let Some(separator) = tokens.iter().position(|token| {
        token.depth == 0 && token.kind == TokenKind::Operator && token.text == b"::"
    }) else {
        old_style_declaration(
            &tokens,
            first_index,
            owner,
            procedure,
            unit_scope,
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
    let access = declaration_access(&tokens, first_index, separator);

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
                            if let Some(unit) = facts.units.get_mut(&unit_scope) {
                                unit.bound_type_procedures.insert(owner, token.text);
                            }
                        }
                    }
                    if first.is(b"generic") {
                        facts.generic_type_procedures.insert(token.text);
                        if let Some(owner) = owner {
                            facts
                                .generic_bound_type_procedures
                                .insert(owner, token.text);
                            if let Some(unit) = facts.units.get_mut(&unit_scope) {
                                unit.generic_bound_type_procedures.insert(owner, token.text);
                            }
                        }
                    }
                    continue;
                }
                match (owner, &declared_type) {
                    (Some(owner), declared) => {
                        facts.cases.components.insert(owner, token.text);
                        facts.cases.symbols.insert(token.text);
                        if let Some(unit) = facts.units.get_mut(&unit_scope) {
                            unit.components.insert(owner, token.text);
                            if let Some(declared) = declared {
                                unit.type_graph
                                    .insert_component(owner, token.text, declared);
                            }
                        }
                        if file_scope_declaration {
                            facts.file_symbols.insert(token.text);
                        }
                        if let Some(declared) = declared {
                            facts.types.insert_component(owner, token.text, declared);
                        }
                    }
                    (None, declared) => {
                        facts.cases.symbols.insert(token.text);
                        if let Some(unit) = facts.units.get_mut(&unit_scope) {
                            unit.symbols.insert(token.text);
                            if let Some(declared) = declared {
                                unit.insert_variable_type(token.text, declared);
                            }
                            if let Some(access) = access {
                                if matches!(unit.kind, ScopeKind::Module | ScopeKind::File) {
                                    unit.access.mark(token.text, access);
                                }
                            }
                        }
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

/// The accessibility attribute carried in a declaration's attribute list, if
/// it has one. Absent means the enclosing scope's default governs.
///
/// Only top-level tokens count. An attribute is a member of the comma-separated
/// list itself, never something nested inside one of its parentheses, so a type
/// or kind selector that happens to name an entity `public` or `private` —
/// `type(Public), private :: Secret` — must not be read as the attribute and
/// invert the declaration's accessibility.
fn declaration_access(
    tokens: &[crate::source::Token<'_>],
    start: usize,
    separator: usize,
) -> Option<Accessibility> {
    tokens[start..separator]
        .iter()
        .filter(|token| token.depth == 0)
        .find_map(|token| {
            token
                .is_name(b"private")
                .then_some(Accessibility::Private)
                .or_else(|| token.is_name(b"public").then_some(Accessibility::Public))
        })
}

#[allow(clippy::too_many_arguments)]
fn old_style_declaration(
    tokens: &[crate::source::Token<'_>],
    first_index: usize,
    owner: Option<&[u8]>,
    procedure: Option<&[u8]>,
    unit_scope: usize,
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
                if let Some(unit) = facts.units.get_mut(&unit_scope) {
                    unit.components.insert(owner, token.text);
                    if let Some(declared_type) = &declared_type {
                        unit.type_graph
                            .insert_component(owner, token.text, declared_type);
                    }
                }
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
                if let Some(unit) = facts.units.get_mut(&unit_scope) {
                    unit.symbols.insert(token.text);
                    if let Some(declared_type) = &declared_type {
                        unit.insert_variable_type(token.text, declared_type);
                    }
                }
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

fn scope_names(scopes: &ScopeTree, facts: &mut FileFacts) {
    for (scope_index, scope) in scopes.scopes.iter().enumerate() {
        let Some(name) = scope.name.as_deref() else {
            continue;
        };
        match scope.kind {
            ScopeKind::Module | ScopeKind::Submodule => facts.cases.modules.insert(name),
            ScopeKind::Program => {
                facts.cases.symbols.insert(name);
                facts.file_symbols.insert(name);
            }
            ScopeKind::Procedure => {
                facts.cases.symbols.insert(name);
                facts.file_symbols.insert(name);
                let parent_scope = facts.units.get(&scope_index).and_then(|unit| unit.parent);
                let host_kind = parent_scope
                    .and_then(|parent_scope| facts.units.get_mut(&parent_scope))
                    .map(|parent| {
                        parent.symbols.insert(name);
                        parent.kind
                    });
                if host_kind == Some(ScopeKind::File) {
                    facts.external_symbols.insert(name);
                }
            }
            ScopeKind::DerivedType => {
                facts.cases.types.insert(name);
                facts.cases.symbols.insert(name);
                facts.file_symbols.insert(name);
                facts.declared_types.insert(name);
                let mut parent = scope.parent;
                while let Some(candidate) = parent {
                    if let Some(unit) = facts.units.get_mut(&candidate) {
                        unit.types.insert(name);
                        unit.symbols.insert(name);
                        break;
                    }
                    parent = scopes.scopes[candidate].parent;
                }
            }
            ScopeKind::File | ScopeKind::Interface | ScopeKind::Construct => {}
        }
    }
}

fn access_statement(text: &[u8], unit_scope: usize, facts: &mut FileFacts) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    let access = if tokens[first].is_name(b"private") {
        Accessibility::Private
    } else if tokens[first].is_name(b"public") {
        Accessibility::Public
    } else {
        return;
    };
    let Some(unit) = facts.units.get_mut(&unit_scope) else {
        return;
    };
    if !matches!(unit.kind, ScopeKind::Module | ScopeKind::File) {
        return;
    }
    let start = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")
        .map_or(first + 1, |separator| separator + 1);
    let names = tokens[start..]
        .iter()
        .filter(|token| token.depth == 0 && token.kind == TokenKind::Name)
        .map(|token| token.text)
        .collect::<Vec<_>>();
    if names.is_empty() {
        unit.access.set_default(access);
    } else {
        for name in names {
            unit.access.mark(name, access);
        }
    }
}

fn type_definition_access(text: &[u8], unit_scope: usize, facts: &mut FileFacts) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    if !is_type_definition(&tokens, first) {
        return;
    }
    let Some(separator) = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")
    else {
        return;
    };
    let Some(access) = declaration_access(&tokens, first, separator) else {
        return;
    };
    let Some(name) = tokens[separator + 1..]
        .iter()
        .find(|token| token.depth == 0 && token.kind == TokenKind::Name)
    else {
        return;
    };
    if let Some(unit) = facts.units.get_mut(&unit_scope) {
        if matches!(unit.kind, ScopeKind::Module | ScopeKind::File) {
            unit.access.mark(name.text, access);
        }
    }
}

/// Record authored USE names and retain the association for scope-aware lookup.
fn use_statement(text: &[u8], symbols: &mut CaseMap) -> Option<UseAssociation> {
    let tokens = tokenize(text, &mut LexState::default());
    let first = usize::from(
        tokens
            .first()
            .is_some_and(|token| token.kind == TokenKind::Number),
    );
    if !tokens.get(first).is_some_and(|token| token.is_name(b"use")) {
        return None;
    }
    let separator = tokens
        .iter()
        .enumerate()
        .skip(first + 1)
        .find(|(_, token)| token.depth == 0 && token.text == b"::")
        .map(|(index, _)| index);
    let module_start = separator.map_or(first + 1, |index| index + 1);
    let (module_index, module) = tokens
        .iter()
        .enumerate()
        .skip(module_start)
        .find(|(_, token)| token.depth == 0 && token.kind == TokenKind::Name)?;
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
            association.names.push(UseName {
                local: local.text.to_ascii_lowercase(),
                remote: remote.text.to_ascii_lowercase(),
                local_spelling: local.text.to_vec(),
            });
        }
        item_start = item_end.saturating_add(1);
    }
    Some(association)
}

fn include_statement(text: &[u8], line: usize, includes: &mut Vec<IncludeDirective>) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    if !tokens[first].is_name(b"include") {
        return;
    }
    let Some(string) = tokens
        .get(first + 1)
        .filter(|token| token.kind == TokenKind::String)
    else {
        return;
    };
    let Some(path) = include_path(string.text) else {
        return;
    };
    includes.push(IncludeDirective { line, path });
}

fn include_path(literal: &[u8]) -> Option<Vec<u8>> {
    let quote = *literal.first()?;
    if literal.len() < 2 || !matches!(quote, b'\'' | b'"') || literal.last() != Some(&quote) {
        return None;
    }
    let mut path = Vec::with_capacity(literal.len().saturating_sub(2));
    let mut index = 1;
    while index + 1 < literal.len() {
        if literal[index] == quote && literal.get(index + 1) == Some(&quote) {
            path.push(quote);
            index += 2;
        } else {
            path.push(literal[index]);
            index += 1;
        }
    }
    (!path.is_empty()).then_some(path)
}

/// Statement forms whose entities are names but do not have a type-spec `::`.
fn auxiliary_declaration(text: &[u8], symbols: &mut CaseMap, unit_symbols: &mut CaseMap) {
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
            unit_symbols.insert(name.text);
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
                unit_symbols.insert(token.text);
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
        if token.depth > 0 {
            continue;
        }
        if token.kind == TokenKind::Comma {
            expect_name = true;
        } else if token.kind == TokenKind::Name && expect_name {
            symbols.insert(token.text);
            unit_symbols.insert(token.text);
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
