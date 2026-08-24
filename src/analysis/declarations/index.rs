use super::syntax::{declared_binding_names, declared_variable_names, procedure_header_names};
use crate::{
    analysis::{
        implicit::{is_implicit_statement, ImplicitPolicy},
        names::CaseMap,
        scope::{ScopeKind, ScopeTree},
    },
    transform::document::Analysis,
};
use std::collections::HashMap;

/// The declaration name sets consulted by line-sensitive keyword and case rules.
///
/// These deliberately are not derived from file/project case tables: this
/// index answers which names are visible at one physical line, keeping local
/// and enclosing file declarations distinct without materializing a full map
/// for every line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredNameIndex {
    local_names: Vec<CaseMap>,
    file_declared_names: Vec<CaseMap>,
    scopes_by_line: Vec<Vec<usize>>,
    procedures_by_line: Vec<Option<usize>>,
    local_owners_by_line: Vec<Option<usize>>,
    local_scope_names: HashMap<Vec<u8>, Vec<usize>>,
    implicit_policies: Vec<ImplicitPolicy>,
    implicit_scopes_by_line: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredSpelling<'a> {
    Absent,
    Ambiguous,
    Spelling(&'a [u8]),
}

impl DeclaredSpelling<'_> {
    pub fn is_declared(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

fn declared_spelling<'a>(names: Option<&'a CaseMap>, name: &[u8]) -> DeclaredSpelling<'a> {
    let Some(names) = names else {
        return DeclaredSpelling::Absent;
    };
    names
        .get(name)
        .map_or(DeclaredSpelling::Ambiguous, DeclaredSpelling::Spelling)
}

impl DeclaredNameIndex {
    /// The spelling map of the innermost local scope active on `line`.
    pub fn local_at(&self, line: usize) -> Option<&CaseMap> {
        self.local_owners_by_line
            .get(line)
            .and_then(|index| *index)
            .and_then(|index| self.local_names.get(index))
    }

    pub fn local_contains(&self, line: usize, name: &[u8]) -> bool {
        self.local_at(line)
            .is_some_and(|names| names.contains(name))
    }

    /// Return the nearest local or host-associated declaration governing
    /// `name` at `line`, including an explicit ambiguous result.
    pub fn governing_local_case(&self, line: usize, name: &[u8]) -> DeclaredSpelling<'_> {
        let names = self
            .scopes_by_line
            .get(line)
            .into_iter()
            .flatten()
            .filter_map(|index| self.local_names.get(*index))
            .find(|names| names.contains(name));
        declared_spelling(names, name)
    }

    pub fn file_declared_contains(&self, line: usize, name: &[u8]) -> bool {
        self.scopes_by_line
            .get(line)
            .into_iter()
            .flatten()
            .filter_map(|index| self.file_declared_names.get(*index))
            .any(|names| names.contains(name))
    }

    /// Resolve an enclosing file declaration without conflating absence with
    /// an ambiguous spelling.
    pub fn file_declared_case(&self, line: usize, name: &[u8]) -> DeclaredSpelling<'_> {
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
    /// requiring that scope to be visible at `line`.
    pub fn file_declared_anywhere(&self, name: &[u8]) -> DeclaredSpelling<'_> {
        self.case_in_maps(self.file_declared_names.iter(), name)
    }

    fn case_in_maps<'a>(
        &'a self,
        maps: impl Iterator<Item = &'a CaseMap>,
        name: &[u8],
    ) -> DeclaredSpelling<'a> {
        let mut found = false;
        let mut spelling = None;
        for names in maps {
            if !names.contains(name) {
                continue;
            }
            found = true;
            let Some(candidate) = names.get(name) else {
                return DeclaredSpelling::Ambiguous;
            };
            if spelling.is_some_and(|existing: &[u8]| existing != candidate) {
                return DeclaredSpelling::Ambiguous;
            }
            spelling = Some(candidate);
        }
        if !found {
            DeclaredSpelling::Absent
        } else if let Some(spelling) = spelling {
            DeclaredSpelling::Spelling(spelling)
        } else {
            DeclaredSpelling::Ambiguous
        }
    }

    /// True when a name belongs to some procedure other than the active one.
    pub fn local_declared_outside(&self, line: usize, name: &[u8]) -> bool {
        let active = self.procedures_by_line.get(line).and_then(|index| *index);
        self.local_scope_names
            .get(&name.to_ascii_lowercase())
            .into_iter()
            .flatten()
            .any(|index| Some(*index) != active)
    }

    /// Whether the active scoping unit permits an implicitly typed entity
    /// whose name begins with the same letter as `name`.
    pub fn implicit_allows(&self, line: usize, name: &[u8]) -> bool {
        let scope = self.implicit_scopes_by_line.get(line).copied().unwrap_or(0);
        self.implicit_policies
            .get(scope)
            .copied()
            .unwrap_or(ImplicitPolicy::ALL)
            .permits(name)
    }

    /// Match keyword lowering's local/enclosing declaration guards.
    pub fn suppresses_keyword(&self, line: usize, name: &[u8], specifier_argument: bool) -> bool {
        self.local_contains(line, name)
            || (!specifier_argument && self.file_declared_contains(line, name))
    }
}

/// Build the line-indexed name sets used by per-line keyword and case rules.
pub fn scoped_declared_names(analysis: &Analysis, scopes: &ScopeTree) -> DeclaredNameIndex {
    let line_count = analysis.buffer.lines.len();
    let mut file_by_scope = vec![CaseMap::default(); scopes.scopes.len()];

    // Opening names belong to their enclosing scope, except interface
    // signatures, whose procedure names are not ordinary host declarations.
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
            continue;
        }
        if is_scoped_declared_owner(scopes.scopes[parent].kind) {
            file_by_scope[parent].insert(name);
        }
    }

    // Module/program specification declarations are file declarations.
    for group in &analysis.groups {
        let line = group.lines.start;
        let Some(owner_index) = scopes
            .ancestors(scopes.index_of_line(line))
            .into_iter()
            .find(|scope| {
                matches!(
                    scopes.scopes[*scope].kind,
                    ScopeKind::Module | ScopeKind::Program
                )
            })
        else {
            continue;
        };
        let owner = &scopes.scopes[owner_index];
        if !owner.is_specification(line)
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
                file_by_scope[owner_index].insert(&name);
            }
        }
    }

    // Type-bound names suppress keyword lowering on their declaration line,
    // while their case remains in the type-procedure namespace.
    for group in &analysis.groups {
        let line = group.lines.start;
        let Some(owner_index) = scopes
            .ancestors(scopes.index_of_line(line))
            .into_iter()
            .find(|scope| scopes.scopes[*scope].kind == ScopeKind::DerivedType)
        else {
            continue;
        };
        if scopes.in_interface(line) {
            continue;
        }
        for statement in &group.statements {
            for name in declared_binding_names(&statement.text) {
                file_by_scope[owner_index].insert(&name);
            }
        }
    }

    let scopes_by_line: Vec<Vec<usize>> = (0..line_count)
        .map(|line| scopes.ancestors(scopes.index_of_line(line)))
        .collect();

    let mut locals_by_scope = vec![CaseMap::default(); scopes.scopes.len()];
    let local_owners_by_line: Vec<Option<usize>> = scopes_by_line
        .iter()
        .map(|ancestors| {
            ancestors
                .iter()
                .copied()
                .find(|index| owns_locals(scopes.scopes[*index].kind))
        })
        .collect();

    for (index, scope) in scopes.scopes.iter().enumerate() {
        if !owns_locals(scope.kind) {
            continue;
        }
        let mut header_names = Vec::new();
        if is_procedure_scope(scope.kind) {
            // A procedure's own name is a name in its own scope: for a
            // function it is the result variable, and for either kind it is
            // what a recursive reference spells. A procedure held by a module
            // or a program already gets this from its parent's declarations,
            // but an external one has no such parent, so a file-level
            // `subroutine erf(x)` had only its header read as the intrinsic
            // `erf` — the header uppercased while the body and the END kept
            // the declared spelling, and the next pass propagated the header.
            //
            // An interface body is excluded for the same reason it is excluded
            // from the enclosing-scope registration above: it describes a
            // signature the project defines elsewhere, and its name has to stay
            // resolvable against that definition.
            if let Some(name) = scope
                .name
                .as_deref()
                .filter(|_| !scopes.in_interface(scope.lines.start))
            {
                header_names.push(name.to_vec());
            }
            if let Some(group) = analysis
                .groups
                .iter()
                .find(|group| group.lines.start == scope.lines.start)
            {
                for statement in &group.statements {
                    header_names.extend(procedure_header_names(&statement.text));
                }
            }
        }
        for group in &analysis.groups {
            let line = group.lines.start;
            if local_owners_by_line.get(line).copied().flatten() != Some(index)
                || !scope.is_specification(line)
            {
                continue;
            }
            for statement in &group.statements {
                for name in declared_variable_names(&statement.text) {
                    locals_by_scope[index].insert(&name);
                }
            }
        }
        // Explicit declarations determine spelling when a continued header
        // uses a different case; header names still provide membership.
        for name in header_names {
            if !locals_by_scope[index].contains(&name) {
                locals_by_scope[index].insert(&name);
            }
        }
    }

    // Reverse-index locals by their owning procedure. Construct declarations
    // have shorter lifetime but still belong to that procedure's territory.
    let mut local_scope_names: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
    for (scope, names) in locals_by_scope.iter().enumerate() {
        let owner = scopes
            .ancestors(scope)
            .into_iter()
            .find(|index| is_procedure_scope(scopes.scopes[*index].kind));
        let Some(owner) = owner else {
            continue;
        };
        for name in names.keys() {
            local_scope_names
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(owner);
        }
    }

    // Complete construct maps with host names. ScopeTree emits parents before
    // children, so each parent map is final when read.
    for index in 0..scopes.scopes.len() {
        if scopes.scopes[index].kind != ScopeKind::Construct {
            continue;
        }
        let Some(parent) = scopes.scopes[index].parent.filter(|parent| *parent < index) else {
            continue;
        };
        if !owns_locals(scopes.scopes[parent].kind) {
            continue;
        }
        let inherited = locals_by_scope[parent].clone();
        locals_by_scope[index].overlay(&inherited);
    }

    let procedures_by_line = scopes_by_line
        .iter()
        .map(|ancestors| {
            ancestors
                .iter()
                .copied()
                .find(|index| is_procedure_scope(scopes.scopes[*index].kind))
        })
        .collect();

    let mut implicit_statements = vec![Vec::new(); scopes.scopes.len()];
    for group in &analysis.groups {
        let owner = scopes
            .ancestors(scopes.index_of_line(group.lines.start))
            .into_iter()
            .find(|index| owns_implicit_policy(scopes.scopes[*index].kind))
            .unwrap_or(0);
        for statement in &group.statements {
            if is_implicit_statement(&statement.text) {
                implicit_statements[owner].push(statement.text.as_slice());
            }
        }
    }

    // Policies inherit down ScopeTree except interface bodies, which restart
    // from the language default.
    let mut implicit_policies = vec![ImplicitPolicy::ALL; scopes.scopes.len()];
    for index in 0..scopes.scopes.len() {
        let scope = &scopes.scopes[index];
        let mut policy = if scope.kind == ScopeKind::Interface || index == 0 {
            ImplicitPolicy::ALL
        } else {
            debug_assert!(scope.parent.is_some_and(|parent| parent < index));
            scope
                .parent
                .and_then(|parent| implicit_policies.get(parent).copied())
                .unwrap_or(ImplicitPolicy::ALL)
        };
        for statement in &implicit_statements[index] {
            policy = policy.apply(statement);
        }
        implicit_policies[index] = policy;
    }
    let implicit_scopes_by_line = scopes_by_line
        .iter()
        .map(|ancestors| {
            ancestors
                .iter()
                .copied()
                .find(|index| owns_implicit_policy(scopes.scopes[*index].kind))
                .unwrap_or(0)
        })
        .collect();

    DeclaredNameIndex {
        local_names: locals_by_scope,
        file_declared_names: file_by_scope,
        scopes_by_line,
        procedures_by_line,
        local_owners_by_line,
        local_scope_names,
        implicit_policies,
        implicit_scopes_by_line,
    }
}

fn owns_implicit_policy(kind: ScopeKind) -> bool {
    matches!(
        kind,
        ScopeKind::File
            | ScopeKind::Module
            | ScopeKind::Submodule
            | ScopeKind::Program
            | ScopeKind::Procedure
            | ScopeKind::Interface
    )
}

fn is_scoped_declared_owner(kind: ScopeKind) -> bool {
    matches!(
        kind,
        ScopeKind::Module | ScopeKind::Program | ScopeKind::Procedure
    )
}

fn is_procedure_scope(kind: ScopeKind) -> bool {
    matches!(kind, ScopeKind::Program | ScopeKind::Procedure)
}

fn owns_locals(kind: ScopeKind) -> bool {
    is_procedure_scope(kind) || kind == ScopeKind::Construct
}
