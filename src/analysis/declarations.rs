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
use std::collections::{HashMap, HashSet};

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
    /// Names owned by each procedure scope; line lookup selects the
    /// innermost active procedure without copying the map.
    local_names: Vec<CaseMap>,
    /// Names owned by each enclosing module/program/procedure scope.
    file_declared_names: Vec<CaseMap>,
    /// The small ancestor list for each physical line.  Keeping indices here
    /// avoids a hash insertion for every visible name on every line.
    scopes_by_line: Vec<Vec<usize>>,
    /// The innermost procedure scope active on each physical line.
    procedures_by_line: Vec<Option<usize>>,
    /// Procedure-local names indexed by owning scope.  This is a compact
    /// reverse index used by case application to avoid leaking a local from a
    /// different procedure through the file-wide declaration table.
    local_scope_names: HashMap<Vec<u8>, Vec<usize>>,
}

impl DeclaredNameIndex {
    /// The spelling map for the innermost procedure active on `line`.
    ///
    /// The map is stored once per procedure scope; callers must not materialize
    /// a visible-name map for every physical line (that is quadratic on large
    /// modules).  `None` means the line is outside every procedure.
    pub fn local_at(&self, line: usize) -> Option<&CaseMap> {
        self.procedures_by_line
            .get(line)
            .and_then(|index| *index)
            .and_then(|index| self.local_names.get(index))
    }

    pub fn local_contains(&self, line: usize, name: &[u8]) -> bool {
        self.local_at(line)
            .is_some_and(|names| names.contains(name))
    }

    pub fn file_declared_contains(&self, line: usize, name: &[u8]) -> bool {
        self.scopes_by_line
            .get(line)
            .into_iter()
            .flatten()
            .filter_map(|index| self.file_declared_names.get(*index))
            .any(|names| names.contains(name))
    }

    /// True when the file-declared set has this name but no agreed spelling.
    /// The nested option distinguishes "not declared here" from "declared
    /// ambiguously here" without exposing the storage maps.
    pub fn file_declared_case(&self, line: usize, name: &[u8]) -> Option<Option<&[u8]>> {
        self.case_in_maps(
            self.scopes_by_line
                .get(line)
                .into_iter()
                .flatten()
                .filter_map(|index| self.file_declared_names.get(*index)),
            name,
        )
    }

    /// The unique spelling contributed by any enclosing file scope, without
    /// requiring that scope to be visible at `line`.  This is the file-local
    /// equivalent of the reference's `symbol_cases`: it includes sibling
    /// module declarations, but never procedure-body locals.
    pub fn file_declared_anywhere(&self, name: &[u8]) -> Option<Option<&[u8]>> {
        self.case_in_maps(self.file_declared_names.iter(), name)
    }

    fn case_in_maps<'a>(
        &'a self,
        maps: impl Iterator<Item = &'a CaseMap>,
        name: &[u8],
    ) -> Option<Option<&'a [u8]>> {
        let mut found = false;
        let mut spelling = None;
        for names in maps {
            if !names.contains(name) {
                continue;
            }
            found = true;
            let Some(candidate) = names.get(name) else {
                return Some(None);
            };
            if spelling.is_some_and(|existing: &[u8]| existing != candidate) {
                return Some(None);
            }
            spelling = Some(candidate);
        }
        found.then_some(spelling)
    }

    /// True when a name belongs to some procedure other than the active one.
    /// Such a name is not a file-wide symbol and must not be propagated into
    /// unrelated procedures.
    pub fn local_declared_outside(&self, line: usize, name: &[u8]) -> bool {
        let active = self.procedures_by_line.get(line).and_then(|index| *index);
        self.local_scope_names
            .get(&name.to_ascii_lowercase())
            .into_iter()
            .flatten()
            .any(|index| Some(*index) != active)
    }

    /// Match `lowercase_keyword`'s two guards: a procedure-local name always
    /// wins, while a name from an enclosing scope yields to a `KEYWORD=`
    /// specifier argument.  The latter exception is deliberately not shared
    /// with the local set.
    pub fn suppresses_keyword(&self, line: usize, name: &[u8], specifier_argument: bool) -> bool {
        self.local_contains(line, name)
            || (!specifier_argument && self.file_declared_contains(line, name))
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
    /// A compatibility map for unscoped local facts.  New extraction stores
    /// procedure locals in `procedure_local_types`, because names such as
    /// `this` are routinely reused for different derived types.
    pub local_types: HashMap<Vec<u8>, Vec<u8>>,
    local_type_ambiguities: HashSet<Vec<u8>>,
    /// Types known for names local to each procedure, keyed by procedure name.
    pub procedure_local_types: HashMap<Vec<u8>, HashMap<Vec<u8>, Vec<u8>>>,
    procedure_local_type_ambiguities: HashMap<Vec<u8>, HashSet<Vec<u8>>>,
    /// Variable or component name (lowercase) to its derived type (lowercase).
    pub variable_types: HashMap<Vec<u8>, Vec<u8>>,
    variable_type_ambiguities: HashSet<Vec<u8>>,
    /// `(type, component)` to the component's own derived type, all lowercase.
    /// This is what resolves the second and later links of an `a%b%c` chain.
    pub component_types: HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
    component_type_ambiguities: HashSet<(Vec<u8>, Vec<u8>)>,
    /// A derived type's direct parent, when its declaration uses `EXTENDS`.
    /// An absent parent declaration remains an unresolved chain boundary.
    pub parent_types: HashMap<Vec<u8>, Vec<u8>>,
    parent_type_ambiguities: HashSet<Vec<u8>>,
}

impl TypeMaps {
    /// Later files never override an earlier disagreeing entry.  A name whose
    /// type is not agreed on project-wide is tombstoned, which keeps a later
    /// third spelling from making an ambiguous component chain resolvable.
    pub fn merge(&mut self, other: &TypeMaps) {
        merge_type_map(
            &mut self.local_types,
            &mut self.local_type_ambiguities,
            &other.local_types,
            &other.local_type_ambiguities,
        );
        for (procedure, types) in &other.procedure_local_types {
            let local = self
                .procedure_local_types
                .entry(procedure.clone())
                .or_default();
            let ambiguities = self
                .procedure_local_type_ambiguities
                .entry(procedure.clone())
                .or_default();
            merge_type_map(
                local,
                ambiguities,
                types,
                other
                    .procedure_local_type_ambiguities
                    .get(procedure)
                    .unwrap_or(&HashSet::new()),
            );
        }
        for (procedure, ambiguities) in &other.procedure_local_type_ambiguities {
            let local = self
                .procedure_local_types
                .entry(procedure.clone())
                .or_default();
            let own_ambiguities = self
                .procedure_local_type_ambiguities
                .entry(procedure.clone())
                .or_default();
            merge_type_map(local, own_ambiguities, &HashMap::new(), ambiguities);
        }
        merge_type_map(
            &mut self.variable_types,
            &mut self.variable_type_ambiguities,
            &other.variable_types,
            &other.variable_type_ambiguities,
        );
        merge_component_type_map(
            &mut self.component_types,
            &mut self.component_type_ambiguities,
            &other.component_types,
            &other.component_type_ambiguities,
        );
        merge_type_map(
            &mut self.parent_types,
            &mut self.parent_type_ambiguities,
            &other.parent_types,
            &other.parent_type_ambiguities,
        );
    }

    pub fn insert_local(&mut self, name: &[u8], type_name: &[u8]) {
        insert_agreed_type(
            &mut self.local_types,
            &mut self.local_type_ambiguities,
            name,
            type_name,
        );
    }

    pub fn insert_procedure_local(&mut self, procedure: &[u8], name: &[u8], type_name: &[u8]) {
        insert_agreed_type(
            self.procedure_local_types
                .entry(procedure.to_ascii_lowercase())
                .or_default(),
            self.procedure_local_type_ambiguities
                .entry(procedure.to_ascii_lowercase())
                .or_default(),
            name,
            type_name,
        );
    }

    pub fn insert_variable(&mut self, name: &[u8], type_name: &[u8]) {
        insert_agreed_type(
            &mut self.variable_types,
            &mut self.variable_type_ambiguities,
            name,
            type_name,
        );
    }

    pub fn insert_component(&mut self, owner: &[u8], name: &[u8], type_name: &[u8]) {
        let key = (owner.to_ascii_lowercase(), name.to_ascii_lowercase());
        if self.component_type_ambiguities.contains(&key) {
            return;
        }
        match self.component_types.get(&key) {
            None => {
                self.component_types
                    .insert(key, type_name.to_ascii_lowercase());
            }
            Some(existing) if existing != &type_name.to_ascii_lowercase() => {
                self.component_types.remove(&key);
                self.component_type_ambiguities.insert(key);
            }
            _ => {}
        }
    }

    pub fn insert_parent(&mut self, child: &[u8], parent: &[u8]) {
        insert_agreed_type(
            &mut self.parent_types,
            &mut self.parent_type_ambiguities,
            child,
            parent,
        );
    }

    pub fn parent_type(&self, child: &[u8]) -> Option<&[u8]> {
        self.parent_types
            .get(&child.to_ascii_lowercase())
            .map(Vec::as_slice)
    }

    pub fn parent_type_is_ambiguous(&self, child: &[u8]) -> bool {
        self.parent_type_ambiguities
            .contains(&child.to_ascii_lowercase())
    }

    /// Resolve a component's declared derived type, including inherited
    /// components. A cycle or an ambiguous parent relation is unresolved.
    pub fn component_type(&self, owner: &[u8], name: &[u8]) -> Option<Vec<u8>> {
        let mut current = owner.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return None;
            }
            let key = (current.clone(), name.clone());
            if self.component_type_ambiguities.contains(&key) {
                return None;
            }
            if let Some(type_name) = self.component_types.get(&key) {
                return Some(type_name.clone());
            }
            if self.parent_type_is_ambiguous(&current) {
                return None;
            }
            let parent = self.parent_type(&current)?;
            current = parent.to_vec();
        }
    }

    /// Follow a `%` chain from a root name to the type of its last link.
    pub fn resolve_chain(&self, root: &[u8], links: &[&[u8]]) -> Option<Vec<u8>> {
        self.resolve_chain_with_locals(None, root, links)
    }

    pub fn resolve_chain_with_locals(
        &self,
        procedure: Option<&[u8]>,
        root: &[u8],
        links: &[&[u8]],
    ) -> Option<Vec<u8>> {
        let root = root.to_ascii_lowercase();
        let local_types =
            procedure.and_then(|name| self.procedure_local_types.get(&name.to_ascii_lowercase()));
        let local_ambiguities = procedure.and_then(|name| {
            self.procedure_local_type_ambiguities
                .get(&name.to_ascii_lowercase())
        });
        if local_ambiguities.is_some_and(|names| names.contains(&root)) {
            return None;
        }
        if self.local_type_ambiguities.contains(&root) {
            return None;
        }
        let mut current = local_types
            .and_then(|types| types.get(&root))
            .or_else(|| self.local_types.get(&root))
            .or_else(|| {
                if self.variable_type_ambiguities.contains(&root) {
                    None
                } else {
                    self.variable_types.get(&root)
                }
            })?
            .clone();
        for link in links {
            current = self.component_type(&current, link)?;
        }
        Some(current)
    }
}

fn insert_agreed_type(
    map: &mut HashMap<Vec<u8>, Vec<u8>>,
    ambiguities: &mut HashSet<Vec<u8>>,
    name: &[u8],
    type_name: &[u8],
) {
    let key = name.to_ascii_lowercase();
    let value = type_name.to_ascii_lowercase();
    if ambiguities.contains(&key) {
        return;
    }
    match map.get(&key) {
        None => {
            map.insert(key, value);
        }
        Some(existing) if existing != &value => {
            map.remove(&key);
            ambiguities.insert(key);
        }
        _ => {}
    }
}

fn merge_type_map(
    into: &mut HashMap<Vec<u8>, Vec<u8>>,
    ambiguities: &mut HashSet<Vec<u8>>,
    other: &HashMap<Vec<u8>, Vec<u8>>,
    other_ambiguities: &HashSet<Vec<u8>>,
) {
    for name in other_ambiguities {
        into.remove(name);
        ambiguities.insert(name.clone());
    }
    for (name, ty) in other {
        if ambiguities.contains(name) {
            continue;
        }
        match into.get(name) {
            None => {
                into.insert(name.clone(), ty.clone());
            }
            Some(existing) if existing != ty => {
                into.remove(name);
                ambiguities.insert(name.clone());
            }
            _ => {}
        }
    }
}

fn merge_component_type_map(
    into: &mut HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
    ambiguities: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    other: &HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
    other_ambiguities: &HashSet<(Vec<u8>, Vec<u8>)>,
) {
    for key in other_ambiguities {
        into.remove(key);
        ambiguities.insert(key.clone());
    }
    for (key, ty) in other {
        if ambiguities.contains(key) {
            continue;
        }
        match into.get(key) {
            None => {
                into.insert(key.clone(), ty.clone());
            }
            Some(existing) if existing != ty => {
                into.remove(key);
                ambiguities.insert(key.clone());
            }
            _ => {}
        }
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
        let procedure = scopes
            .program_unit_of_line(group.lines.start)
            .filter(|scope| scope.kind == ScopeKind::Procedure)
            .and_then(|scope| scope.name.as_deref())
            .map(|name| name.to_ascii_lowercase());
        // Interface bodies are procedure signatures, not module variables.
        // The reference nevertheless sends their headers and declarations
        // through extract_procedure_cases, so keep their dummies and RESULT
        // names in the procedure-local tables.
        for statement in &group.statements {
            if let Some((child, parent)) = type_definition_parent(&statement.text) {
                facts.types.insert_parent(child, parent);
            }
            use_statement(&statement.text, &mut facts.cases.symbols);
            auxiliary_declaration(&statement.text, &mut facts.cases.symbols);
            entity_declaration(
                &statement.text,
                owner.as_deref(),
                procedure.as_deref(),
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
    for scope in scopes.scopes.iter().skip(1) {
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

    let scopes_by_line: Vec<Vec<usize>> = (0..line_count)
        .map(|line| scopes.ancestors(scopes.index_of_line(line)))
        .collect();

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
        let mut header_names = Vec::new();
        if let Some(group) = analysis
            .groups
            .iter()
            .find(|group| group.lines.start == scope.lines.start)
        {
            for statement in &group.statements {
                header_names.extend(procedure_header_names(&statement.text));
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
        // Explicit declarations determine the spelling when a continued
        // procedure header uses a different case.  Header names still supply
        // membership (and their spelling when no declaration repeats them),
        // matching ProcedureDeclarationCases.local_cases.
        for name in header_names {
            if !locals_by_scope[index].contains(&name) {
                locals_by_scope[index].insert(&name);
            }
        }
    }

    let procedures_by_line = scopes_by_line
        .iter()
        .map(|ancestors| {
            ancestors
                .iter()
                .copied()
                .find(|index| scopes.scopes[*index].kind == ScopeKind::Procedure)
        })
        .collect();

    let mut local_scope_names: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
    for (scope, names) in locals_by_scope.iter().enumerate() {
        for name in names.keys() {
            local_scope_names
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(scope);
        }
    }

    DeclaredNameIndex {
        local_names: locals_by_scope,
        file_declared_names: file_by_scope,
        scopes_by_line,
        procedures_by_line,
        local_scope_names,
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
/// Statement-function syntax, executable calls that merely contain a
/// declaration keyword, initializer expressions, and binding targets after
/// `=>` are intentionally not entities here.
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

fn entity_declaration(
    text: &[u8],
    owner: Option<&[u8]>,
    procedure: Option<&[u8]>,
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
    if first.kind != TokenKind::Name {
        return;
    }
    // `USE, INTRINSIC :: m` also carries `::`, but its entity is a module.
    if first.is(b"use") {
        return;
    }
    // `TYPE :: name` and `TYPE, EXTENDS(parent) :: name` open a derived-type
    // scope.  The scope extractor already records `name` in the type map; it
    // is not an ordinary symbol declaration here.
    if is_type_definition(&tokens, first_index) {
        return;
    }
    let Some(separator) = tokens
        .iter()
        .position(|t| t.depth == 0 && t.kind == TokenKind::Operator && t.text == b"::")
    else {
        old_style_declaration(&tokens, first_index, owner, procedure, facts);
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
                        facts.cases.components.insert(owner, token.text);
                        if let Some(declared) = declared {
                            facts.types.insert_component(owner, token.text, declared);
                        }
                    }
                    (None, declared) => {
                        facts.cases.symbols.insert(token.text);
                        if let Some(declared) = declared {
                            facts.types.insert_variable(token.text, declared);
                            if let Some(procedure) = procedure {
                                facts
                                    .types
                                    .insert_procedure_local(procedure, token.text, declared);
                            }
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
    procedure: Option<&[u8]>,
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
                if let Some(declared_type) = &declared_type {
                    facts
                        .types
                        .insert_component(owner, token.text, declared_type);
                }
            } else {
                facts.cases.symbols.insert(token.text);
                if let Some(declared_type) = &declared_type {
                    facts.types.insert_variable(token.text, declared_type);
                    if let Some(procedure) = procedure {
                        facts
                            .types
                            .insert_procedure_local(procedure, token.text, declared_type);
                    }
                }
            }
        }
    }
}

fn is_type_definition(tokens: &[Token<'_>], first_index: usize) -> bool {
    let first = &tokens[first_index];
    if !first.is(b"type") {
        return false;
    }
    !tokens
        .get(first_index + 1)
        .is_some_and(|token| token.kind == TokenKind::LParen)
        && tokens
            .get(first_index + 1)
            .is_none_or(|token| token.text == b"::" || token.kind == TokenKind::Comma)
}

/// Return the child and direct parent from `TYPE, EXTENDS(parent) :: child`.
/// Both names are returned exactly as written; TypeMaps normalizes them when
/// recording the relationship.
fn type_definition_parent(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let tokens = tokenize(text, &mut LexState::default());
    let first_index = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !is_type_definition(&tokens, first_index) {
        return None;
    }
    let separator = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")?;
    let extends = tokens
        .iter()
        .enumerate()
        .skip(first_index + 1)
        .take_while(|(index, _)| *index < separator)
        .find(|(_, token)| token.depth == 0 && token.is_name(b"extends"))
        .map(|(index, _)| index)?;
    let parent = tokens
        .get(extends + 1..separator)?
        .iter()
        .find(|token| token.kind == TokenKind::Name)?
        .text;
    let child = tokens
        .get(separator + 1..)?
        .iter()
        .find(|token| token.kind == TokenKind::Name)?
        .text;
    Some((child, parent))
}

fn is_old_style_type_context(tokens: &[Token<'_>], first_index: usize) -> bool {
    let first = &tokens[first_index];
    if !(first.is(b"type") || first.is(b"class")) {
        return false;
    }
    tokens
        .get(first_index + 1)
        .is_none_or(|token| token.is_name(b"is") || token.is_name(b"default"))
}

fn old_style_type_name<'a>(tokens: &'a [Token<'a>], first_index: usize) -> Option<&'a [u8]> {
    let open = tokens.get(first_index + 1)?;
    if open.kind != TokenKind::LParen {
        return None;
    }
    tokens
        .get(first_index + 2..)?
        .iter()
        .take_while(|token| token.depth > open.depth)
        .find(|token| token.kind == TokenKind::Name)
        .map(|token| token.text)
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

/// The local names in a `USE ... ONLY:` list.  The remote side of a rename is
/// recorded too: it is an authored identifier in the declaration and supplies
/// spelling evidence for uses in this file.  The module name itself is not a
/// declaration; only a separately declared module contributes to `modules`.
fn use_statement(text: &[u8], symbols: &mut CaseMap) {
    let tokens = tokenize(text, &mut LexState::default());
    // A leading numeric statement label is not part of the statement.
    let first = usize::from(tokens.first().is_some_and(|t| t.kind == TokenKind::Number));
    if !tokens.get(first).is_some_and(|t| t.is_name(b"use")) {
        return;
    }
    let Some(only) = tokens
        .iter()
        .position(|token| token.depth == 0 && token.is_name(b"only"))
    else {
        return;
    };
    if tokens.get(only + 1).is_none_or(|token| token.text != b":") {
        return;
    }
    let mut expect_name = true;
    for token in tokens.iter().skip(only + 2) {
        if token.depth != 0 {
            continue;
        }
        if token.kind == TokenKind::Comma {
            expect_name = true;
        } else if token.kind == TokenKind::Name && expect_name {
            symbols.insert(token.text);
            expect_name = false;
        } else if token.text == b"=>" {
            expect_name = true;
        }
    }
}

/// Statement forms whose entities are names but do not have a type-spec `::`.
/// Keep the recognizer anchored at the first code word: an initializer or an
/// executable call containing `external` must never manufacture a declaration.
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

/// Resolve the selector of `SELECT TYPE (alias => selector)` using the maps
/// accumulated earlier in this file.  An unknown selector is deliberately
/// left untyped; the case pass must not guess its component owner.
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
    types.resolve_chain_with_locals(procedure, root.text, &links)
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
        assert!(names.file_declared_contains(0, b"status"));
        assert!(names.file_declared_contains(5, b"status"));
        assert!(names.local_contains(3, b"size"));
        assert!(names.local_contains(5, b"local"));
        assert!(!names.local_contains(7, b"size"));
        assert!(!names.file_declared_contains(0, b"size"));
    }

    #[test]
    fn procedure_header_names_survive_a_logical_continuation() {
        let names = scoped(
            b"subroutine s(first, second, &\nthird)\ncall f(FIRST, SECOND, THIRD)\nend subroutine s\n",
        );
        for name in [b"first".as_slice(), b"second", b"third"] {
            assert!(names.local_contains(0, name));
            assert!(names.local_contains(1, name));
        }
    }

    #[test]
    fn scoped_declared_names_exclude_components_and_interface_bodies() {
        let names = scoped(
            b"module m\ninterface\nsubroutine signature(Status)\ninteger :: Status\nend subroutine signature\nend interface\ntype :: t\ninteger :: Component\nend type t\ninteger :: Visible\nend module m\n",
        );
        for line in 0..11 {
            assert!(!names.file_declared_contains(line, b"component"));
            assert!(!names.file_declared_contains(line, b"signature"));
        }
        assert!(names.file_declared_contains(9, b"visible"));
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
    fn use_statements_do_not_invent_module_declarations() {
        let facts = facts(
            b"program p\n\
            use Precision\n\
            use, intrinsic :: ISO_Fortran_env\n\
            use :: Results, only: x\n\
            end program\n",
        );
        assert!(!facts.cases.modules.contains(b"precision"));
        assert!(!facts.cases.modules.contains(b"iso_fortran_env"));
        assert!(!facts.cases.modules.contains(b"results"));
    }

    #[test]
    fn conflicting_spellings_in_one_file_make_the_name_untouchable() {
        let facts = facts(
            b"module Precision\nend module Precision\n\
            module PRECISION\nend module PRECISION\n",
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
            facts.cases.components.get(b"limberrec", b"source"),
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

        let mut c = TypeMaps::default();
        c.variable_types.insert(b"x".to_vec(), b"t3".to_vec());
        a.merge(&c);
        assert!(a.variable_types.is_empty());
        assert_eq!(a.resolve_chain(b"x", &[]), None);
    }

    #[test]
    fn old_style_class_locals_and_components_feed_the_same_chain() {
        let facts = facts(
            b"type :: T\nreal :: X\nend type T\n\
              subroutine s(this)\nclass(T) this\nthis%x = 1\nend subroutine s\n",
        );
        assert_eq!(
            facts
                .types
                .procedure_local_types
                .get(b"s".as_slice())
                .and_then(|types| types.get(b"this".as_slice())),
            Some(&b"t".to_vec())
        );
        assert_eq!(
            facts.cases.components.get(b"t", b"x"),
            Some(b"X".as_slice())
        );
    }

    #[test]
    fn continued_parameter_declarations_are_procedure_locals() {
        let source = b"module m\ncontains\nfunction f(t)\ninteger, parameter :: n_table = 27\ninteger, dimension(n_table), parameter :: Temps = &\n [1, 2]\nreal, dimension(n_table), parameter :: rates = &\n [1.0, 2.0]\nx = RATES + TEMPS(1)\nend function f\nend module m\n";
        let document = Document::from_bytes(source);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = scoped_declared_names(&analysis, &scopes);
        assert!(names.local_contains(8, b"rates"));
        assert!(names.local_contains(8, b"temps"));
    }

    #[test]
    fn numeric_function_names_do_not_hide_later_locals() {
        let source = b"module m\ncontains\nfunction kappa_HH_21cm(T, deriv)\ninteger, parameter :: n_table = 27\ninteger, dimension(n_table), parameter :: Temps = &\n [1, 2]\nreal, dimension(n_table), parameter :: rates = &\n [1.0, 2.0]\nx = RATES + TEMPS(1)\nend function kappa_HH_21cm\nend module m\n";
        let document = Document::from_bytes(source);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = scoped_declared_names(&analysis, &scopes);
        assert!(names.local_contains(8, b"rates"));
        assert!(names.local_contains(8, b"temps"));
    }

    #[test]
    fn old_style_declarations_register_entities_but_not_type_statement_words() {
        let facts = facts(b"program p\nreal x, y(3)\ninteger*4 n\ntype is (t)\nend program p\n");
        for name in [b"x".as_slice(), b"y", b"n"] {
            assert!(facts.cases.symbols.contains(name));
        }
        assert!(!facts.cases.symbols.contains(b"is"));
        assert!(!facts.cases.symbols.contains(b"function"));
    }

    #[test]
    fn procedure_headers_results_and_interface_dummies_are_local() {
        let names = scoped(
            b"module m\ninterface\nfunction sig(arg) result(answer)\nreal :: arg, answer\nend function sig\nend interface\ncontains\nfunction real_name(value) result(ResultValue)\nreal :: value, ResultValue\nend function real_name\nend module m\n",
        );
        assert!(names.local_contains(2, b"arg"));
        assert!(names.local_contains(2, b"answer"));
        assert!(names.local_contains(7, b"value"));
        assert!(names.local_contains(7, b"resultvalue"));
        assert!(!names.file_declared_contains(2, b"arg"));
    }

    #[test]
    fn use_only_renames_and_auxiliary_name_lists_feed_symbols() {
        let facts = facts(
            b"program p\nuse M, only: Local => Remote, Plain\nexternal :: Ext1, Ext2\nintrinsic sin, cos\ncommon /Block/ A, B\nnamelist /Group/ C, D\nentry Enter(X)\nend program p\n",
        );
        for name in [
            b"local".as_slice(),
            b"remote",
            b"plain",
            b"ext1",
            b"ext2",
            b"sin",
            b"cos",
            b"block",
            b"a",
            b"b",
            b"group",
            b"c",
            b"d",
            b"enter",
        ] {
            assert!(facts.cases.symbols.contains(name), "missing {name:?}");
        }
        assert!(!facts.cases.symbols.contains(b"x"));
    }

    #[test]
    fn type_bound_binding_targets_are_not_registered_as_bindings() {
        let facts = facts(
            b"type :: T\nprocedure(iface), pass :: Run => Run_impl\ngeneric :: Op(+) => Add\nfinal :: Cleanup\nend type T\n",
        );
        for name in [b"run".as_slice(), b"op", b"cleanup"] {
            assert!(facts.cases.type_procedures.contains(name));
        }
        for name in [b"run_impl".as_slice(), b"add"] {
            assert!(!facts.cases.type_procedures.contains(name));
            assert!(!facts.cases.symbols.contains(name));
        }
    }

    #[test]
    fn select_type_alias_uses_the_selector_type_for_chains() {
        let facts = facts(
            b"module m\ntype :: T\ninteger :: Value\nend type T\ncontains\nsubroutine s(obj)\nclass(T) :: obj\nselect type (Alias => obj)\nAlias%VALUE = 1\nend select\nend subroutine s\nend module m\n",
        );
        assert_eq!(
            facts
                .types
                .procedure_local_types
                .get(b"s".as_slice())
                .and_then(|types| types.get(b"alias".as_slice())),
            Some(&b"t".to_vec())
        );
        assert_eq!(
            facts.cases.components.get(b"t", b"value"),
            Some(b"Value".as_slice())
        );
    }

    #[test]
    fn component_case_keys_keep_same_names_in_different_types_separate() {
        let facts = facts(
            b"module m\ntype :: First\ninteger :: Tcmb\nend type First\n\
              type :: Second\ninteger :: tcMB\nend type Second\n",
        );
        assert_eq!(
            facts.cases.components.get(b"first", b"tcmb"),
            Some(b"Tcmb".as_slice())
        );
        assert_eq!(
            facts.cases.components.get(b"second", b"tcmb"),
            Some(b"tcMB".as_slice())
        );
    }

    #[test]
    fn extends_records_parentage_without_registering_inherited_components() {
        let facts = facts(
            b"module m\n\
              type :: Leaf\n\
                real :: Value\n\
              end type Leaf\n\
              type :: Parent\n\
                real :: INTEGRATE_TOL\n\
                type(Leaf) :: Nested\n\
              end type Parent\n\
              type, extends(Parent) :: Child\n\
                real :: VALUE\n\
              end type Child\n",
        );
        assert_eq!(
            facts.types.parent_types.get(b"child".as_slice()),
            Some(&b"parent".to_vec())
        );
        assert_eq!(
            facts.types.component_type(b"child", b"nested"),
            Some(b"leaf".to_vec())
        );
        assert!(!facts.cases.components.contains(b"child", b"integrate_tol"));
        assert!(!facts.cases.components.contains(b"child", b"nested"));
    }

    #[test]
    fn inherited_component_type_cycles_and_unknown_parents_are_unresolved() {
        let mut types = TypeMaps::default();
        types.insert_parent(b"A", b"B");
        types.insert_parent(b"B", b"A");
        types.insert_component(b"A", b"nested", b"Leaf");
        assert_eq!(types.component_type(b"A", b"missing"), None);
        assert_eq!(
            types.component_type(b"A", b"nested"),
            Some(b"leaf".to_vec())
        );
        types.insert_parent(b"Ambig", b"A");
        types.insert_parent(b"Ambig", b"B");
        types.insert_component(b"A", b"value", b"Leaf");
        assert_eq!(types.component_type(b"Ambig", b"value"), None);

        let mut unknown = TypeMaps::default();
        unknown.insert_parent(b"Child", b"External");
        assert_eq!(unknown.component_type(b"Child", b"value"), None);
    }
}
