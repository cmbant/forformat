//! Declaration extraction: what names a file defines, and how it spells them.
//!
//! This is the hard core of the port (~1,250 Python lines).  The design keeps
//! two things separate on purpose:
//!
//! * **where a declaration is** — [`super::scope::ScopeTree`], derived from the
//!   classifier, so scopes and indentation can never disagree;
//! * **what a declaration says** — the extractors below, which read
//!   `LogicalGroup` statement text rather than re-scanning raw files with
//!   regexes, so a continued or semicolon-separated declaration is seen exactly
//!   once and in its assembled form.
//!
//! Extractors are added one statement form at a time.  Each one only ever
//! *inserts* into a [`CaseMap`]; ambiguity handling and the resolution order
//! live in [`super::names`] and are already complete.

use super::{
    names::{CaseMap, CaseTables},
    scope::{ScopeKind, ScopeTree},
};
use crate::{
    source::{
        tokens::{tokenize, Token, TokenKind},
        LexState, PhysicalLineKind,
    },
    transform::document::Analysis,
};
use std::collections::HashMap;

/// Everything one file contributes to the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// Spellings this file declares, per name space.
    pub cases: CaseTables,
    /// Macro names defined by `#define` in this file.
    pub macros: CaseMap,
    /// The declared type of each name, used to resolve `a%b%c` chains.
    pub types: TypeMaps,
}

/// The two declaration name sets consulted by the reference keyword pass.
///
/// These are intentionally not derived from [`FileFacts::cases`].  The case
/// tables are file/project-wide because the later case pass needs that view;
/// keyword lowering instead needs the names visible at one physical line in
/// this file only.  Keeping the indexes as parallel per-line maps also makes
/// the distinction between a procedure's local names and enclosing
/// file-declared names explicit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredNameIndex {
    local_names: Vec<CaseMap>,
    file_declared_names: Vec<CaseMap>,
}

impl DeclaredNameIndex {
    pub fn local_at(&self, line: usize) -> Option<&CaseMap> {
        self.local_names.get(line)
    }

    pub fn file_declared_at(&self, line: usize) -> Option<&CaseMap> {
        self.file_declared_names.get(line)
    }

    /// Match `lowercase_keyword`'s two guards: a procedure-local name always
    /// wins, while a name from an enclosing scope yields to a `KEYWORD=`
    /// specifier argument.  The latter exception is deliberately not shared
    /// with the local set.
    pub fn suppresses_keyword(&self, line: usize, name: &[u8], specifier_argument: bool) -> bool {
        self.local_at(line)
            .is_some_and(|names| names.contains(name))
            || (!specifier_argument
                && self
                    .file_declared_at(line)
                    .is_some_and(|names| names.contains(name)))
    }
}

impl FileFacts {
    pub fn merge(&mut self, other: &FileFacts) {
        self.cases.merge(&other.cases);
        self.macros.merge(&other.macros);
        self.types.merge(&other.types);
    }
}

/// Name-to-type mappings.  Unlike the case maps, these are keyed and valued
/// case-insensitively: they answer "what type is this?", not "how is it spelt?".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeMaps {
    /// Variable or component name (lowercase) to its derived type (lowercase).
    pub variable_types: HashMap<Vec<u8>, Vec<u8>>,
    /// `(type, component)` to the component's own derived type, all lowercase.
    /// This is what resolves the second and later links of an `a%b%c` chain.
    pub component_types: HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
}

impl TypeMaps {
    /// Later files never override an earlier disagreeing entry: a name whose
    /// type is not agreed on project-wide is simply dropped, which keeps the
    /// component chain resolver from guessing.
    pub fn merge(&mut self, other: &TypeMaps) {
        for (name, ty) in &other.variable_types {
            match self.variable_types.get(name) {
                None => {
                    self.variable_types.insert(name.clone(), ty.clone());
                }
                Some(existing) if existing != ty => {
                    self.variable_types.remove(name);
                }
                _ => {}
            }
        }
        for (key, ty) in &other.component_types {
            match self.component_types.get(key) {
                None => {
                    self.component_types.insert(key.clone(), ty.clone());
                }
                Some(existing) if existing != ty => {
                    self.component_types.remove(key);
                }
                _ => {}
            }
        }
    }

    /// Follow a `%` chain from a root name to the type of its last link.
    pub fn resolve_chain(&self, root: &[u8], links: &[&[u8]]) -> Option<Vec<u8>> {
        let mut current = self.variable_types.get(&root.to_ascii_lowercase())?.clone();
        for link in links {
            current = self
                .component_types
                .get(&(current.clone(), link.to_ascii_lowercase()))?
                .clone();
        }
        Some(current)
    }
}

/// Extract every declaration fact from one analyzed file.
pub fn extract(analysis: &Analysis, scopes: &ScopeTree) -> FileFacts {
    let mut facts = FileFacts::default();
    scope_names(scopes, &mut facts.cases);
    for (index, group) in analysis.groups.iter().enumerate() {
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
        for statement in &group.statements {
            use_statement(&statement.text, &mut facts.cases.modules);
            entity_declaration(&statement.text, owner.as_deref(), &mut facts);
        }
        let _ = index;
    }
    facts
}

/// Build the line-indexed name sets used by the per-line keyword rules.
///
/// This is a separate pass over the already assembled statements.  It is
/// built once for each `Analysis` view, rather than once per physical line;
/// that matters after a structure pass has changed the document's line map.
pub fn scoped_declared_names(analysis: &Analysis, scopes: &ScopeTree) -> DeclaredNameIndex {
    let line_count = analysis.buffer.lines.len();
    let mut file_by_scope = vec![CaseMap::default(); scopes.scopes.len()];

    // A scope's own opening name belongs to its enclosing scope.  In
    // particular, a top-level module/program/procedure is not a file-wide
    // declared name.  Derived types follow the same rule; their components do
    // not enter this index at all.
    for (index, scope) in scopes.scopes.iter().enumerate().skip(1) {
        let Some(name) = scope.name.as_deref() else {
            continue;
        };
        if !matches!(
            scope.kind,
            ScopeKind::Module | ScopeKind::Program | ScopeKind::Procedure | ScopeKind::DerivedType
        ) {
            continue;
        }
        let Some(parent) = scope.parent else {
            continue;
        };
        if scopes.in_interface(scope.lines.start) {
            // Interface signatures are not declarations in the enclosing
            // module's ordinary name set.
            continue;
        }
        if is_scoped_declared_owner(scopes.scopes[parent].kind) {
            file_by_scope[parent].insert(name);
        }
        let _ = index;
    }

    // Only variables in a module specification part contribute to the
    // file-declared set.  The derived-type and interface checks are separate
    // because both can occur before the module's own CONTAINS.
    for group in &analysis.groups {
        let line = group.lines.start;
        let Some(module_index) = enclosing_scope_of_kind(scopes, line, ScopeKind::Module) else {
            continue;
        };
        let module = &scopes.scopes[module_index];
        if !module.is_specification(line)
            || scopes.in_interface(line)
            || scopes
                .ancestors(scopes.index_of_line(line))
                .iter()
                .any(|index| scopes.scopes[*index].kind == ScopeKind::DerivedType)
        {
            continue;
        }
        for statement in &group.statements {
            for name in declared_variable_names(&statement.text) {
                file_by_scope[module_index].insert(&name);
            }
        }
    }

    let mut file_declared_names = vec![CaseMap::default(); line_count];
    for (line, visible) in file_declared_names.iter_mut().enumerate() {
        for (index, scope) in scopes.scopes.iter().enumerate() {
            if !is_scoped_declared_owner(scope.kind)
                || !scope_contains(scope, line)
                || file_by_scope[index].is_empty()
            {
                continue;
            }
            visible.merge(&file_by_scope[index]);
        }
    }

    // Procedure-local names are the names from declarations before that
    // procedure's own CONTAINS, plus header dummy/result names and SELECT
    // TYPE aliases.  Choose the innermost procedure for each physical line;
    // enclosing procedure locals are intentionally not unioned here because
    // that is what `active_procedure_at` does in the reference.
    let mut locals_by_scope = vec![CaseMap::default(); scopes.scopes.len()];
    for (index, scope) in scopes.scopes.iter().enumerate() {
        if scope.kind != ScopeKind::Procedure {
            continue;
        }
        if let Some(group) = analysis
            .groups
            .iter()
            .find(|group| group.lines.start == scope.lines.start)
        {
            for statement in &group.statements {
                for name in procedure_header_names(&statement.text) {
                    locals_by_scope[index].insert(&name);
                }
            }
        }
        for group in &analysis.groups {
            let line = group.lines.start;
            if !scope_contains(scope, line) || !scope.is_specification(line) {
                continue;
            }
            for statement in &group.statements {
                for name in declared_variable_names(&statement.text) {
                    locals_by_scope[index].insert(&name);
                }
                if let Some(alias) = select_type_alias(&statement.text) {
                    locals_by_scope[index].insert(&alias);
                }
            }
        }
    }

    let mut local_names = vec![CaseMap::default(); line_count];
    for (line, visible) in local_names.iter_mut().enumerate() {
        let mut selected = None;
        let mut selected_size = usize::MAX;
        for (index, scope) in scopes.scopes.iter().enumerate() {
            if scope.kind != ScopeKind::Procedure
                || !scope_contains(scope, line)
                || locals_by_scope[index].is_empty()
            {
                continue;
            }
            let size = scope.lines.end.saturating_sub(scope.lines.start);
            if size < selected_size {
                selected = Some(index);
                selected_size = size;
            }
        }
        if let Some(index) = selected {
            *visible = locals_by_scope[index].clone();
        }
    }

    DeclaredNameIndex {
        local_names,
        file_declared_names,
    }
}

fn is_scoped_declared_owner(kind: ScopeKind) -> bool {
    matches!(
        kind,
        ScopeKind::Module | ScopeKind::Program | ScopeKind::Procedure
    )
}

fn scope_contains(scope: &super::scope::Scope, line: usize) -> bool {
    scope.lines.start <= line && line < scope.lines.end
}

fn enclosing_scope_of_kind(scopes: &ScopeTree, line: usize, kind: ScopeKind) -> Option<usize> {
    scopes
        .ancestors(scopes.index_of_line(line))
        .into_iter()
        .find(|index| scopes.scopes[*index].kind == kind)
}

/// The subset of declaration extraction used by the reference's
/// `_declared_variable_names`.  It excludes derived-type declarations and
/// procedure bindings, which are declarations in other namespaces.
fn declared_variable_names(text: &[u8]) -> Vec<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return Vec::new();
    };
    let first = &tokens[first_index];
    if first.kind != TokenKind::Name || first.is(b"use") {
        return Vec::new();
    }
    let separator = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::");
    let Some(separator) = separator else {
        return old_style_variable_names(&tokens, first_index);
    };
    if (first.is(b"type") || first.is(b"class"))
        && tokens
            .get(first_index + 1)
            .is_none_or(|token| token.kind != TokenKind::LParen)
    {
        return Vec::new();
    }
    if matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"procedure" | b"generic" | b"final"
    ) {
        return Vec::new();
    }
    declaration_entity_names(&tokens, separator + 1)
}

fn old_style_variable_names(tokens: &[Token<'_>], first_index: usize) -> Vec<Vec<u8>> {
    let first = &tokens[first_index];
    let declaration = matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer" | b"real" | b"complex" | b"logical" | b"character" | b"type" | b"class"
    ) || first.is(b"double")
        && tokens
            .get(first_index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    if !declaration {
        return Vec::new();
    }
    let start = first_index
        + 1
        + usize::from(
            first.is(b"double")
                && tokens
                    .get(first_index + 1)
                    .is_some_and(|token| token.is_name(b"precision")),
        );
    if tokens.iter().skip(start).any(|token| {
        token.kind == TokenKind::Name && token.depth == 0 && token.is_name(b"function")
    }) {
        return Vec::new();
    }
    declaration_entity_names(tokens, start)
}

fn declaration_entity_names(tokens: &[Token<'_>], start: usize) -> Vec<Vec<u8>> {
    let mut names = Vec::new();
    let mut expect_name = true;
    let mut initializer = false;
    for token in tokens.iter().skip(start) {
        if token.depth > 0 {
            continue;
        }
        if token.text == b"=" || token.text == b"=>" {
            initializer = true;
            expect_name = false;
            continue;
        }
        if token.kind == TokenKind::Comma {
            initializer = false;
            expect_name = true;
            continue;
        }
        if !initializer && expect_name && token.kind == TokenKind::Name {
            names.push(token.text.to_vec());
            expect_name = false;
        }
    }
    names
}

fn procedure_header_names(text: &[u8]) -> Vec<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(kind) = tokens.iter().position(|token| {
        token.kind == TokenKind::Name && (token.is(b"function") || token.is(b"subroutine"))
    }) else {
        return Vec::new();
    };
    let Some(name_index) = tokens
        .iter()
        .enumerate()
        .skip(kind + 1)
        .find(|(_, token)| token.kind == TokenKind::Name)
        .map(|(index, _)| index)
    else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut index = name_index + 1;
    if tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        index += 1;
        let mut expect_name = true;
        while let Some(token) = tokens.get(index) {
            if token.kind == TokenKind::RParen && token.depth == 0 {
                index += 1;
                break;
            }
            if token.kind == TokenKind::Comma && token.depth == 1 {
                expect_name = true;
            } else if expect_name && token.kind == TokenKind::Name && token.depth == 1 {
                names.push(token.text.to_vec());
                expect_name = false;
            }
            index += 1;
        }
    }
    for window in tokens[index..].windows(2) {
        if window[0].is_name(b"result") && window[1].kind == TokenKind::LParen {
            if let Some(name) = tokens
                .iter()
                .skip(index)
                .skip_while(|token| !token.is_name(b"result"))
                .nth(2)
                .filter(|token| token.kind == TokenKind::Name)
            {
                names.push(name.text.to_vec());
            }
            break;
        }
    }
    names
}

fn select_type_alias(text: &[u8]) -> Option<Vec<u8>> {
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
    let alias = tokens.get(select + 3)?;
    let arrow = tokens.get(select + 4)?;
    (alias.kind == TokenKind::Name && arrow.text == b"=>").then(|| alias.text.to_vec())
}

/// A declaration written with `::`: `<type-spec>[, attrs] :: a, b(3), c = 1`.
///
/// This is the form that protects declared identifiers from being mistaken for
/// keywords, which is why it is the first extractor: a component called
/// `Source` or a variable called `Data` must keep its spelling (I4), and every
/// case rule asks these tables before it touches a name.
///
/// `owner` is the lowercase name of the derived type being defined, when the
/// statement is inside one; its entities are components rather than symbols.
///
/// Not yet handled: `EXTERNAL`/`INTRINSIC`
/// lists, `COMMON` blocks, statement functions, and `ENTRY` names.
/// The type name inside a `TYPE(...)` / `CLASS(...)` specification.
///
/// The scan has to stay inside the specification's own parentheses.
/// `CLASS(*)` and `TYPE(*)` are unlimited polymorphic and name no type; a scan
/// that runs past the closing paren finds the first *attribute* instead and
/// records it as a declared type name.  By I4 a declared name outranks the
/// keyword tables, so one `CLASS(*), INTENT(IN) :: x` used to stop `intent`
/// being lowercased anywhere in the file.
fn type_spec_name<'a>(tokens: &[Token<'a>], start: usize, limit: usize) -> Option<&'a [u8]> {
    let open = tokens.get(start + 1)?;
    if open.kind != TokenKind::LParen {
        return None;
    }
    tokens
        .get(start + 2..limit)?
        .iter()
        .take_while(|token| token.depth > open.depth)
        .find(|token| token.kind == TokenKind::Name)
        .map(|token| token.text)
}

fn entity_declaration(text: &[u8], owner: Option<&[u8]>, facts: &mut FileFacts) {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return;
    };
    let first = &tokens[first_index];
    if first.kind != TokenKind::Name {
        return;
    }
    // `USE, INTRINSIC :: m` also carries `::`, but its entity is a module.
    if first.is(b"use") {
        return;
    }
    let Some(separator) = tokens
        .iter()
        .position(|t| t.depth == 0 && t.kind == TokenKind::Operator && t.text == b"::")
    else {
        old_style_declaration(&tokens, first_index, owner, facts);
        return;
    };

    // `TYPE(Foo)` / `CLASS(Foo)` names a type and gives every entity its type.
    let declared_type = (first.is(b"type") || first.is(b"class"))
        .then(|| type_spec_name(&tokens, first_index, separator))
        .flatten()
        .map(|name| {
            facts.cases.types.insert(name);
            name.to_ascii_lowercase()
        });

    // A binding inside a derived type is a type-bound procedure, not a
    // component: `procedure :: run`, `generic :: assignment(=) => copy`.
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
                    continue;
                }
                match (owner, &declared_type) {
                    (Some(owner), declared) => {
                        facts.cases.components.insert(token.text);
                        if let Some(declared) = declared {
                            facts.types.component_types.insert(
                                (owner.to_vec(), token.text.to_ascii_lowercase()),
                                declared.clone(),
                            );
                        }
                    }
                    (None, declared) => {
                        facts.cases.symbols.insert(token.text);
                        if let Some(declared) = declared {
                            facts
                                .types
                                .variable_types
                                .insert(token.text.to_ascii_lowercase(), declared.clone());
                        }
                    }
                }
            }
            // Everything after `=` or `=>` is an initializer until the next
            // top-level comma.
            _ => {}
        }
    }
}

fn old_style_declaration(
    tokens: &[crate::source::Token<'_>],
    first_index: usize,
    owner: Option<&[u8]>,
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
    if !declaration {
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
    // `DOUBLE PRECISION FUNCTION G()` opens a procedure; it declares no entity
    // here.  Reading it as one registers `FUNCTION` as a symbol, and by I4 a
    // declared name outranks the keyword tables, so the word stops being
    // lowercased anywhere in the file.
    if tokens
        .iter()
        .skip(entity_start)
        .find(|token| token.kind == TokenKind::Name && token.depth == 0)
        .is_some_and(|token| token.is_name(b"function"))
    {
        return;
    }
    let declared_type = if first.is(b"type") || first.is(b"class") {
        tokens
            .get(first_index + 1..)
            .and_then(|rest| rest.iter().find(|token| token.kind == TokenKind::Name))
            .map(|token| {
                facts.cases.types.insert(token.text);
                token.text.to_ascii_lowercase()
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
                facts.cases.components.insert(token.text);
                if let Some(declared_type) = &declared_type {
                    facts.types.component_types.insert(
                        (owner.to_vec(), token.text.to_ascii_lowercase()),
                        declared_type.clone(),
                    );
                }
            } else {
                facts.cases.symbols.insert(token.text);
                if let Some(declared_type) = &declared_type {
                    facts
                        .types
                        .variable_types
                        .insert(token.text.to_ascii_lowercase(), declared_type.clone());
                }
            }
        }
    }
}

/// Names that the scope structure itself carries: module, submodule, program,
/// procedure and derived-type names, each in its own name space.
fn scope_names(scopes: &ScopeTree, cases: &mut CaseTables) {
    for scope in &scopes.scopes {
        let Some(name) = scope.name.as_deref() else {
            continue;
        };
        match scope.kind {
            ScopeKind::Module | ScopeKind::Submodule => cases.modules.insert(name),
            ScopeKind::Program | ScopeKind::Procedure => cases.symbols.insert(name),
            ScopeKind::DerivedType => cases.types.insert(name),
            ScopeKind::File | ScopeKind::Interface => {}
        }
    }
}

/// `USE m`, `USE, INTRINSIC :: m`, `USE m, ONLY: x` — only the module name is
/// taken here; the `ONLY` list is a separate extractor.
fn use_statement(text: &[u8], modules: &mut CaseMap) {
    let tokens = tokenize(text, &mut LexState::default());
    let mut index = 0;
    // A leading numeric statement label is not part of the statement.
    if tokens.first().is_some_and(|t| t.kind == TokenKind::Number) {
        index += 1;
    }
    if !tokens.get(index).is_some_and(|t| t.is_name(b"use")) {
        return;
    }
    index += 1;
    // Skip `, intrinsic ::` / `, non_intrinsic ::`.
    if tokens
        .get(index)
        .is_some_and(|t| t.kind == TokenKind::Comma)
    {
        while index < tokens.len() && tokens[index].text != b"::" {
            index += 1;
        }
        index += 1;
    } else if tokens.get(index).is_some_and(|t| t.text == b"::") {
        index += 1;
    }
    if let Some(token) = tokens.get(index) {
        if token.kind == TokenKind::Name {
            modules.insert(token.text);
        }
    }
}

/// `#define NAME` and `#define NAME(args)`.  Macro spellings outrank every
/// other case rule (I4), so they are collected from every project file.
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

#[cfg(test)]
mod tests {
    use super::{extract, scoped_declared_names, TypeMaps};
    use crate::{analysis::scope::ScopeTree, transform::document::Document};

    fn facts(source: &[u8]) -> super::FileFacts {
        let document = Document::from_bytes(source);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        extract(&analysis, &scopes)
    }

    fn scoped(source: &[u8]) -> super::DeclaredNameIndex {
        let document = Document::from_bytes(source);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        scoped_declared_names(&analysis, &scopes)
    }

    #[test]
    fn scoped_name_indexes_use_zero_based_physical_line_indices() {
        let names = scoped(
            b"module m\ninteger :: Status\ncontains\nsubroutine s(Size)\ninteger :: Local\nx = Size\nend subroutine s\nend module m\n",
        );
        assert!(names.file_declared_at(0).unwrap().contains(b"status"));
        assert!(names.file_declared_at(5).unwrap().contains(b"status"));
        assert!(names.local_at(3).unwrap().contains(b"size"));
        assert!(names.local_at(5).unwrap().contains(b"local"));
        assert!(!names.local_at(7).unwrap().contains(b"size"));
        assert!(!names.file_declared_at(0).unwrap().contains(b"size"));
    }

    #[test]
    fn procedure_header_names_survive_a_logical_continuation() {
        let names = scoped(
            b"subroutine s(first, second, &\nthird)\ncall f(FIRST, SECOND, THIRD)\nend subroutine s\n",
        );
        for name in [b"first".as_slice(), b"second", b"third"] {
            assert!(names.local_at(0).unwrap().contains(name));
            assert!(names.local_at(1).unwrap().contains(name));
        }
    }

    #[test]
    fn scoped_declared_names_exclude_components_and_interface_bodies() {
        let names = scoped(
            b"module m\ninterface\nsubroutine signature(Status)\ninteger :: Status\nend subroutine signature\nend interface\ntype :: t\ninteger :: Component\nend type t\ninteger :: Visible\nend module m\n",
        );
        for line in 0..11 {
            assert!(!names.file_declared_at(line).unwrap().contains(b"component"));
            assert!(!names.file_declared_at(line).unwrap().contains(b"signature"));
        }
        assert!(names.file_declared_at(9).unwrap().contains(b"visible"));
    }

    #[test]
    fn an_unlimited_polymorphic_declaration_names_no_type() {
        // `CLASS(*)` has no type name inside its parentheses.  A scan that runs
        // past the closing paren picks up `intent` instead and, by I4, silences
        // the keyword rule for that word across the whole file.
        let facts = facts(b"subroutine s(r)\nclass(*), intent(in) :: r\nend subroutine s\n");
        assert!(!facts.cases.types.contains(b"intent"));
        assert!(!facts.cases.symbols.contains(b"intent"));
        assert!(facts.cases.symbols.contains(b"r"));
    }

    #[test]
    fn a_function_statement_is_not_an_old_style_declaration() {
        // `DOUBLE PRECISION FUNCTION G()` opens a procedure and declares no
        // entity; reading it as one registers `FUNCTION` as a symbol.
        let facts = facts(
            b"module m\ncontains\ndouble precision function G()\nG = 1\nend function G\nend module m\n",
        );
        assert!(!facts.cases.symbols.contains(b"function"));
        assert!(facts.cases.symbols.contains(b"G"));
    }

    #[test]
    fn scope_names_land_in_their_own_name_spaces() {
        let facts = facts(
            b"module MyModule\n\
              type :: MyType\n\
              end type MyType\n\
            contains\n\
              subroutine DoThing()\n\
              end subroutine DoThing\n\
            end module MyModule\n",
        );
        assert_eq!(
            facts.cases.modules.get(b"mymodule"),
            Some(b"MyModule".as_slice())
        );
        assert_eq!(facts.cases.types.get(b"mytype"), Some(b"MyType".as_slice()));
        assert_eq!(
            facts.cases.symbols.get(b"dothing"),
            Some(b"DoThing".as_slice())
        );
        assert!(facts.cases.symbols.get(b"mymodule").is_none());
    }

    #[test]
    fn use_statements_contribute_module_spellings_in_every_form() {
        let facts = facts(
            b"program p\n\
            use Precision\n\
            use, intrinsic :: ISO_Fortran_env\n\
            use :: Results, only: x\n\
            end program\n",
        );
        assert_eq!(
            facts.cases.modules.get(b"precision"),
            Some(b"Precision".as_slice())
        );
        assert_eq!(
            facts.cases.modules.get(b"iso_fortran_env"),
            Some(b"ISO_Fortran_env".as_slice())
        );
        assert_eq!(
            facts.cases.modules.get(b"results"),
            Some(b"Results".as_slice())
        );
    }

    #[test]
    fn conflicting_spellings_in_one_file_make_the_name_untouchable() {
        let facts = facts(
            b"module M\n\
            use Precision\n\
            use PRECISION\n\
            end module\n",
        );
        assert!(facts.cases.modules.is_ambiguous(b"precision"));
        assert_eq!(facts.cases.modules.get(b"precision"), None);
    }

    #[test]
    fn define_directives_contribute_macro_spellings() {
        let facts =
            facts(b"#define CAMB_DEBUG 1\n#  define Has_Fun(x) (x)\n#undef NOPE\nprogram p\nend\n");
        assert_eq!(
            facts.macros.get(b"camb_debug"),
            Some(b"CAMB_DEBUG".as_slice())
        );
        assert_eq!(facts.macros.get(b"has_fun"), Some(b"Has_Fun".as_slice()));
        assert!(!facts.macros.contains(b"nope"));
    }

    #[test]
    fn declared_entities_are_protected_and_typed() {
        let facts = facts(
            b"module M\n\
              type :: LimberRec\n\
                real(dl), dimension(:), allocatable :: Source\n\
                type(CAMBparams) :: Params\n\
              contains\n\
                procedure :: Run\n\
              end type LimberRec\n\
              integer :: Data, Count = 0\n\
            end module M\n",
        );
        assert_eq!(
            facts.cases.components.get(b"source"),
            Some(b"Source".as_slice())
        );
        assert_eq!(
            facts.cases.type_procedures.get(b"run"),
            Some(b"Run".as_slice())
        );
        assert_eq!(facts.cases.symbols.get(b"data"), Some(b"Data".as_slice()));
        assert_eq!(facts.cases.symbols.get(b"count"), Some(b"Count".as_slice()));
        assert_eq!(
            facts.cases.types.get(b"cambparams"),
            Some(b"CAMBparams".as_slice())
        );
        assert_eq!(
            facts
                .types
                .component_types
                .get(&(b"limberrec".to_vec(), b"params".to_vec())),
            Some(&b"cambparams".to_vec())
        );
    }

    #[test]
    fn an_initializer_does_not_contribute_names() {
        let facts = facts(b"program p\ninteger :: n = size(Other), m\nend program p\n");
        assert_eq!(facts.cases.symbols.get(b"n"), Some(b"n".as_slice()));
        assert_eq!(facts.cases.symbols.get(b"m"), Some(b"m".as_slice()));
        assert!(!facts.cases.symbols.contains(b"other"));
        assert!(!facts.cases.symbols.contains(b"size"));
    }

    #[test]
    fn component_chains_resolve_through_the_type_maps() {
        let mut types = TypeMaps::default();
        types
            .variable_types
            .insert(b"state".to_vec(), b"cambdata".to_vec());
        types.component_types.insert(
            (b"cambdata".to_vec(), b"params".to_vec()),
            b"cambparams".to_vec(),
        );
        assert_eq!(
            types.resolve_chain(b"State", &[b"Params"]),
            Some(b"cambparams".to_vec())
        );
        assert_eq!(types.resolve_chain(b"state", &[b"missing"]), None);
        assert_eq!(types.resolve_chain(b"unknown", &[]), None);
    }

    #[test]
    fn disagreeing_types_are_dropped_rather_than_guessed_when_merging() {
        let mut a = TypeMaps::default();
        a.variable_types.insert(b"x".to_vec(), b"t1".to_vec());
        let mut b = TypeMaps::default();
        b.variable_types.insert(b"x".to_vec(), b"t2".to_vec());
        a.merge(&b);
        assert!(a.variable_types.is_empty());
    }
}
