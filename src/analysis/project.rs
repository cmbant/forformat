//! Project-wide context: declarations plus the visibility graph connecting
//! Fortran scopes.
//!
//! Project-wide case tables remain useful for global namespaces such as module
//! names and owner-qualified components. Ordinary variables and derived-type
//! roots are resolved separately through scope-owned USE associations so a
//! private or unrelated module entity cannot influence a target merely by
//! existing elsewhere in the checkout.

use super::{
    declarations::{extract, FileFacts, HostUnit, UnitFacts, UseAssociation},
    names::{CaseMap, CaseResolver, CaseTables},
    scope::ScopeTree,
};
use crate::{config::MacroDefine, error::FormatError, transform::document::Document};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Visibility<T> {
    Absent,
    Value(T),
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TypeOrigin {
    Local(usize),
    Module(Vec<u8>),
    Submodule { ancestor: Vec<u8>, name: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ResolvedType {
    pub(crate) origin: TypeOrigin,
    pub(crate) name: Vec<u8>,
}

impl<T: Eq + Clone> Visibility<T> {
    fn merge(&mut self, other: Self) {
        match (&*self, other) {
            (Self::Ambiguous, _) => {}
            (_, Self::Ambiguous) => *self = Self::Ambiguous,
            (Self::Absent, value) => *self = value,
            (Self::Value(existing), Self::Value(candidate)) if existing != &candidate => {
                *self = Self::Ambiguous;
            }
            _ => {}
        }
    }

    fn into_option(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::Ambiguous => None,
        }
    }
}

/// Ask one question of every registered definition of a program unit and merge
/// the answers.
///
/// Each definition is an independent route to the name and so traverses from
/// its own copy of `visited`. One shared set would let the first definition
/// mark a (unit, name) pair on the way down and thereby silence the second's
/// answer, which turns a disagreement between two vendored copies of a module
/// into false agreement on whichever definition was registered first. Cycle
/// protection is unaffected: each branch still carries every pair its own
/// descent passed through.
///
/// The copies are made only where a name really has more than one definition,
/// which is the rare vendored-duplicate case; the ordinary single-definition
/// query threads `visited` straight through and allocates nothing.
fn merge_definitions<K, T>(
    units: &[UnitFacts],
    visited: &mut HashSet<K>,
    mut query: impl FnMut(&UnitFacts, &mut HashSet<K>) -> Visibility<T>,
) -> Visibility<T>
where
    K: Clone + Eq + std::hash::Hash,
    T: Clone + Eq,
{
    let shared = (units.len() > 1).then(|| visited.clone());
    let mut resolved = Visibility::Absent;
    for unit in units {
        match &shared {
            Some(shared) => {
                let mut branch = shared.clone();
                resolved.merge(query(unit, &mut branch));
            }
            None => resolved.merge(query(unit, visited)),
        }
    }
    resolved
}

/// The union of every project source's declarations, plus module export facts
/// used to build the namespace visible from each formatting target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectContext {
    /// Merged spellings for global/owner-qualified namespaces. A name spelled
    /// differently in two files is ambiguous project-wide and is left alone.
    pub cases: CaseTables,
    /// Legacy file-wide symbol summary. Scoped ordinary-name resolution does
    /// not use this map.
    pub file_symbols: CaseMap,
    /// Standalone external procedures, which really are project-visible without
    /// USE association.
    external_symbols: CaseMap,
    /// Generic aliases kept separate from ordinary type-procedure names.
    pub generic_type_procedures: CaseMap,
    /// Generic type-bound names keyed by their owner type.
    pub generic_bound_type_procedures: super::names::ComponentCaseMap,
    /// Derived-type definitions, kept separate from TYPE(...) use-site facts.
    pub declared_types: CaseMap,
    /// Macro names from every `#define` in the project, plus any `-D` names.
    pub macros: CaseMap,
    /// Merged owner-qualified type graph for `%` chain resolution.
    pub types: super::declarations::TypeMaps,
    /// The files that contributed, in the order analyzed.
    pub sources: Vec<PathBuf>,
    /// Restrict component ownership to declarations proven in the target file.
    pub target_local_component_resolution: bool,
    /// Every distinct definition registered under one module name. A checkout
    /// that vendors two copies of a module keeps both: a query answers from all
    /// of them and is ambiguous only where their answers actually differ.
    modules: HashMap<Vec<u8>, Vec<UnitFacts>>,
    submodules: HashMap<(Vec<u8>, Vec<u8>), Vec<UnitFacts>>,
    expanded_facts: HashMap<PathBuf, (u64, FileFacts)>,
    expanded_sources: HashMap<u64, Option<PathBuf>>,
}

impl ProjectContext {
    /// The context used when a caller formats a lone buffer with no project.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Analyze one source and fold it in. Relative Fortran INCLUDE statements
    /// are resolved from `path` when the files exist on disk.
    pub fn add_source(&mut self, path: &Path, source: &[u8]) -> Result<(), FormatError> {
        let facts = analyze_file_at(path, source)?;
        self.absorb(path, &facts);
        Ok(())
    }

    /// Fold already-extracted facts in. INCLUDE fragments are analyzed on
    /// demand from the filesystem, without making their declarations globally
    /// visible: they are merged into the scope containing the INCLUDE statement.
    ///
    /// This reads files. Every relative INCLUDE in `facts` is resolved against
    /// `path`'s directory and every absolute one as written, following the same
    /// text-substitution rule a compiler applies, so a caller that filters which
    /// sources reach the project does not thereby bound which files are opened.
    /// Unreadable and unparsable fragments are skipped rather than reported.
    /// Use [`ProjectContext::empty`] alone when no filesystem access is wanted.
    pub fn absorb(&mut self, path: &Path, facts: &FileFacts) {
        let expanded = expand_includes_with(path, facts, &mut |candidate| {
            fs::read(candidate)
                .ok()
                .and_then(|source| analyze_file_at(candidate, &source).ok())
        });
        self.absorb_expanded(path, facts, expanded);
    }

    /// `expanded` is consumed: it is retained verbatim as this path's scope
    /// facts, and every caller has already finished with it.
    fn absorb_expanded(&mut self, path: &Path, facts: &FileFacts, expanded: FileFacts) {
        self.cases.merge(&facts.cases);
        // Included owner-qualified members are safe to add to the project type
        // graph; included root declarations remain in expanded scope facts.
        self.cases.components.merge(&expanded.cases.components);
        self.cases
            .bound_type_procedures
            .merge(&expanded.cases.bound_type_procedures);
        self.file_symbols.merge(&facts.file_symbols);
        self.external_symbols.merge(&facts.external_symbols);
        self.generic_type_procedures
            .merge(&facts.generic_type_procedures);
        self.generic_bound_type_procedures
            .merge(&expanded.generic_bound_type_procedures);
        self.declared_types.merge(&facts.declared_types);
        self.macros.merge(&expanded.macros);
        self.types.merge(&facts.types);
        self.types.merge_non_roots(&expanded.types);
        self.register_modules(&expanded);
        let path = normalize_path(path);
        if let Some((previous_id, _)) = self.expanded_facts.get(&path) {
            if previous_id != &facts.source_id {
                self.expanded_sources.insert(*previous_id, None);
            }
        }
        match self.expanded_sources.get_mut(&facts.source_id) {
            None => {
                self.expanded_sources
                    .insert(facts.source_id, Some(path.clone()));
            }
            Some(Some(existing)) if existing == &path => {}
            Some(slot) => *slot = None,
        }
        self.expanded_facts
            .insert(path.clone(), (facts.source_id, expanded));
        self.sources.push(path);
    }

    /// Expand `facts`'s own INCLUDE directives when this context holds no
    /// expansion that describes them.
    ///
    /// [`ProjectContext::expanded`] matches a stored expansion by path *and*
    /// source fingerprint, so a buffer edited since the context was built — an
    /// editor's unsaved change — matches nothing and falls back to the local
    /// facts. Without this it would fall back to *unexpanded* local facts and
    /// silently lose every declaration its fragments contribute, so a file
    /// would resolve differently for the sole reason that it was dirty.
    ///
    /// This reads files, on the same terms as [`ProjectContext::absorb`].
    pub(crate) fn expand_uncached(&self, path: &Path, facts: FileFacts) -> FileFacts {
        if facts.includes.is_empty() || self.holds_expansion_of(&facts) {
            return facts;
        }
        expand_includes_with(path, &facts, &mut |candidate| {
            fs::read(candidate)
                .ok()
                .and_then(|source| analyze_file_at(candidate, &source).ok())
        })
    }

    /// Whether `expanded` would answer from this context rather than fall back
    /// to the caller's own facts.
    fn holds_expansion_of(&self, facts: &FileFacts) -> bool {
        facts.source_path.as_deref().is_some_and(|path| {
            self.expanded_facts
                .get(path)
                .is_some_and(|(source_id, _)| *source_id == facts.source_id)
        })
    }

    fn register_modules(&mut self, facts: &FileFacts) {
        // Scope order rather than map order, so the registered sequence — and
        // with it this context's identity — does not depend on hashing.
        let mut scopes = facts.units.keys().copied().collect::<Vec<_>>();
        scopes.sort_unstable();
        for scope in scopes {
            self.register_unit(&facts.units[&scope]);
        }
        // A module defined inside an INCLUDE fragment is a project entity even
        // though it has no place in the including file's scope tree. Several
        // files including one fragment register the identical unit, which
        // deduplicates below rather than reading as a vendored duplicate.
        for unit in &facts.included_units {
            self.register_unit(unit);
        }
    }

    fn register_unit(&mut self, unit: &UnitFacts) {
        let Some(host) = unit.project_host.as_ref() else {
            return;
        };
        let definitions = match host {
            HostUnit::Module(name) => self.modules.entry(name.clone()).or_default(),
            HostUnit::Submodule { ancestor, name } => self
                .submodules
                .entry((ancestor.clone(), name.clone()))
                .or_default(),
        };
        // Re-absorbing the same source must not register a second copy.
        if !definitions.iter().any(|existing| existing == unit) {
            definitions.push(unit.clone());
        }
    }

    /// Add command-line macro definitions. These are recorded exactly as
    /// spelled and outrank every declaration.
    pub fn define(&mut self, defines: &[MacroDefine]) {
        for define in defines {
            self.macros.insert(define.name.as_bytes());
        }
    }

    pub fn enable_target_local_component_resolution(&mut self) {
        self.target_local_component_resolution = true;
    }

    /// Bind this context to one file's own declarations for namespaces that
    /// remain genuinely file/project-wide.
    pub fn resolver<'a>(&'a self, local: &'a FileFacts) -> CaseResolver<'a> {
        CaseResolver {
            local: &local.cases,
            project: &self.cases,
            macros: &self.macros,
        }
    }

    fn expanded<'a>(&'a self, local: &'a FileFacts) -> &'a FileFacts {
        // Every name token of every query lands here, so borrow the stored
        // path rather than rebuilding it: `analyze_file_with_path` normalized
        // `source_path` when it produced these facts, and `absorb_expanded`
        // keyed `expanded_facts` by that same normalized path.
        let path = local.source_path.as_deref().or_else(|| {
            self.expanded_sources
                .get(&local.source_id)
                .and_then(|path| path.as_deref())
        });
        let Some(path) = path else {
            return local;
        };
        self.expanded_facts
            .get(path)
            .filter(|(source_id, _)| *source_id == local.source_id)
            .map(|(_, facts)| facts)
            .unwrap_or(local)
    }

    /// Resolve an ordinary symbol through the active construct/program unit,
    /// host association and USE association. Unrelated modules are not queried.
    pub(crate) fn visible_symbol_spelling(
        &self,
        local: &FileFacts,
        line: usize,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let local = self.expanded(local);
        let mut current = local.active_unit(line).map(|unit| unit.scope);
        let mut host_visited = HashSet::new();
        while let Some(scope) = current {
            let unit = local.units.get(&scope)?;
            if unit.symbols.contains(name) {
                return unit.symbols.get(name).map(ToOwned::to_owned);
            }
            let mut use_visited = HashSet::new();
            match self.imported_symbol(&unit.imports, name, false, &mut use_visited) {
                Visibility::Absent => {}
                found => return found.into_option(),
            }
            if !unit.host_access.allows(name) {
                return None;
            }
            if let Some(host) = unit.semantic_host.as_ref() {
                return self
                    .host_visible_symbol(host, name, &mut host_visited)
                    .into_option();
            }
            current = unit.parent;
        }
        None
    }

    /// Resolve a name written inside a USE list. Rename targets are visible on
    /// the USE statement even though the remote name is hidden by the rename
    /// everywhere else in the importing scope.
    pub(crate) fn visible_use_symbol_spelling(
        &self,
        local: &FileFacts,
        line: usize,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let local = self.expanded(local);
        let unit = local.unit_chain(line).into_iter().next()?;
        let mut visited = HashSet::new();
        self.imported_symbol(&unit.imports, name, true, &mut visited)
            .into_option()
    }

    /// Standalone external procedures are globally callable even without USE.
    pub(crate) fn external_symbol_spelling(&self, name: &[u8]) -> Option<Vec<u8>> {
        self.external_symbols.get(name).map(ToOwned::to_owned)
    }

    /// Resolve a derived-type name through exactly the same scope/USE graph as
    /// ordinary names.
    pub(crate) fn visible_type_spelling(
        &self,
        local: &FileFacts,
        line: usize,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let local = self.expanded(local);
        let mut current = local.active_unit(line).map(|unit| unit.scope);
        let mut host_visited = HashSet::new();
        while let Some(scope) = current {
            let unit = local.units.get(&scope)?;
            if unit.types.contains(name) {
                return unit.types.get(name).map(ToOwned::to_owned);
            }
            let mut use_visited = HashSet::new();
            match self.imported_type(&unit.imports, name, &mut use_visited) {
                Visibility::Absent => {}
                found => return found.into_option(),
            }
            if !unit.host_access.allows(name) {
                return None;
            }
            if let Some(host) = unit.semantic_host.as_ref() {
                return self
                    .host_visible_type_spelling(host, name, &mut host_visited)
                    .into_option();
            }
            current = unit.parent;
        }
        None
    }

    /// Resolve the declared derived type of a variable root in the active
    /// scope, retaining the unit that owns the type definition. This identity
    /// is the gate for `%` ownership: two unrelated modules may both define a
    /// type named `state`, and their members must never be merged merely because
    /// the unqualified type name agrees.
    pub(crate) fn visible_variable_type(
        &self,
        local: &FileFacts,
        line: usize,
        name: &[u8],
    ) -> Option<ResolvedType> {
        let local = self.expanded(local);
        let lower = name.to_ascii_lowercase();
        let mut current = local.active_unit(line).map(|unit| unit.scope);
        let mut host_visited = HashSet::new();
        while let Some(scope) = current {
            let unit = local.units.get(&scope)?;
            if unit.variable_types.contains_key(&lower) {
                let type_name = unit.variable_type(&lower)?;
                return self
                    .local_type_identity(local, unit.scope, type_name)
                    .into_option();
            }
            let mut use_visited = HashSet::new();
            match self.imported_variable_type_identity(&unit.imports, &lower, &mut use_visited) {
                Visibility::Absent => {}
                found => return found.into_option(),
            }
            if !unit.host_access.allows(&lower) {
                return None;
            }
            if let Some(host) = unit.semantic_host.as_ref() {
                return self
                    .host_visible_variable_type_identity(host, &lower, &mut host_visited)
                    .into_option();
            }
            current = unit.parent;
        }
        None
    }

    /// Spell a member using only the exact derived-type entity reached from the
    /// visible root. Owner-qualified project summaries are deliberately not a
    /// fallback here because their owner key is only an unqualified type name.
    pub(crate) fn visible_member_spelling(
        &self,
        local: &FileFacts,
        owner: &ResolvedType,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let local = self.expanded(local);
        self.member_spelling_from_type(local, owner, name, &mut HashSet::new())
    }

    /// Resolve the derived type of one component while preserving the component
    /// owner's entity identity. This is used for every intermediate `%` link.
    pub(crate) fn visible_component_type(
        &self,
        local: &FileFacts,
        owner: &ResolvedType,
        name: &[u8],
    ) -> Option<ResolvedType> {
        let local = self.expanded(local);
        self.component_type_from_type(local, owner, name, &mut HashSet::new())
    }

    /// Every definition registered for one module name, in registration order.
    fn module_units(&self, module: &[u8]) -> &[UnitFacts] {
        self.modules
            .get(module)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every definition registered for one host program unit. Empty when the
    /// project never supplied that module or submodule.
    fn host_units(&self, host: &HostUnit) -> &[UnitFacts] {
        match host {
            HostUnit::Module(name) => self.module_units(name),
            HostUnit::Submodule { ancestor, name } => self
                .submodules
                .get(&(ancestor.clone(), name.clone()))
                .map(Vec::as_slice)
                .unwrap_or_default(),
        }
    }

    fn host_origin(host: &HostUnit) -> TypeOrigin {
        match host {
            HostUnit::Module(module) => TypeOrigin::Module(module.clone()),
            HostUnit::Submodule { ancestor, name } => TypeOrigin::Submodule {
                ancestor: ancestor.clone(),
                name: name.clone(),
            },
        }
    }

    fn host_visible_symbol(
        &self,
        host: &HostUnit,
        name: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        let lower = name.to_ascii_lowercase();
        if !visited.insert((host.clone(), lower.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.host_units(host), visited, |unit, visited| {
            self.unit_visible_symbol(unit, &lower, visited)
        })
    }

    fn unit_visible_symbol(
        &self,
        unit: &UnitFacts,
        lower: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        if unit.symbols.contains(lower) {
            return unit
                .symbols
                .get(lower)
                .map(|spelling| Visibility::Value(spelling.to_vec()))
                .unwrap_or(Visibility::Ambiguous);
        }
        let mut use_visited = HashSet::new();
        match self.imported_symbol(&unit.imports, lower, false, &mut use_visited) {
            Visibility::Absent => {}
            found => return found,
        }
        if !unit.host_access.allows(lower) {
            return Visibility::Absent;
        }
        let Some(parent) = unit.semantic_host.as_ref() else {
            return Visibility::Absent;
        };
        self.host_visible_symbol(parent, lower, visited)
    }

    fn host_visible_type_spelling(
        &self,
        host: &HostUnit,
        name: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        let lower = name.to_ascii_lowercase();
        if !visited.insert((host.clone(), lower.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.host_units(host), visited, |unit, visited| {
            self.unit_visible_type_spelling(unit, &lower, visited)
        })
    }

    fn unit_visible_type_spelling(
        &self,
        unit: &UnitFacts,
        lower: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        if unit.types.contains(lower) {
            return unit
                .types
                .get(lower)
                .map(|spelling| Visibility::Value(spelling.to_vec()))
                .unwrap_or(Visibility::Ambiguous);
        }
        let mut use_visited = HashSet::new();
        match self.imported_type(&unit.imports, lower, &mut use_visited) {
            Visibility::Absent => {}
            found => return found,
        }
        if !unit.host_access.allows(lower) {
            return Visibility::Absent;
        }
        let Some(parent) = unit.semantic_host.as_ref() else {
            return Visibility::Absent;
        };
        self.host_visible_type_spelling(parent, lower, visited)
    }

    fn host_visible_type_identity(
        &self,
        host: &HostUnit,
        name: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let lower = name.to_ascii_lowercase();
        if !visited.insert((host.clone(), lower.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.host_units(host), visited, |unit, visited| {
            self.unit_visible_type_identity(host, unit, &lower, visited)
        })
    }

    fn unit_visible_type_identity(
        &self,
        host: &HostUnit,
        unit: &UnitFacts,
        lower: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        if unit.types.contains(lower) {
            return unit
                .types
                .get(lower)
                .map(|_| {
                    Visibility::Value(ResolvedType {
                        origin: Self::host_origin(host),
                        name: lower.to_vec(),
                    })
                })
                .unwrap_or(Visibility::Ambiguous);
        }
        let mut use_visited = HashSet::new();
        match self.imported_type_identity(&unit.imports, lower, &mut use_visited) {
            Visibility::Absent => {}
            found => return found,
        }
        if !unit.host_access.allows(lower) {
            return Visibility::Absent;
        }
        let Some(parent) = unit.semantic_host.as_ref() else {
            return Visibility::Absent;
        };
        self.host_visible_type_identity(parent, lower, visited)
    }

    fn host_visible_variable_type_identity(
        &self,
        host: &HostUnit,
        name: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let lower = name.to_ascii_lowercase();
        if !visited.insert((host.clone(), lower.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.host_units(host), visited, |unit, visited| {
            self.unit_visible_variable_type_identity(host, unit, &lower, visited)
        })
    }

    fn unit_visible_variable_type_identity(
        &self,
        host: &HostUnit,
        unit: &UnitFacts,
        lower: &[u8],
        visited: &mut HashSet<(HostUnit, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        if unit.variable_types.contains_key(lower) {
            let Some(type_name) = unit.variable_type(lower) else {
                return Visibility::Ambiguous;
            };
            let mut type_visited = HashSet::new();
            return self.host_visible_type_identity(host, type_name, &mut type_visited);
        }
        let mut use_visited = HashSet::new();
        match self.imported_variable_type_identity(&unit.imports, lower, &mut use_visited) {
            Visibility::Absent => {}
            found => return found,
        }
        if !unit.host_access.allows(lower) {
            return Visibility::Absent;
        }
        let Some(parent) = unit.semantic_host.as_ref() else {
            return Visibility::Absent;
        };
        self.host_visible_variable_type_identity(parent, lower, visited)
    }

    fn imported_symbol(
        &self,
        imports: &[UseAssociation],
        name: &[u8],
        include_remote_rename_targets: bool,
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        let mut resolved = Visibility::Absent;
        let lower = name.to_ascii_lowercase();
        for association in imports {
            for target in association.targets(name) {
                let remote =
                    self.module_export_symbol(&association.module, &target.remote, visited);
                let candidate = match (remote, target.alias_spelling) {
                    (Visibility::Value(_), Some(alias)) => Visibility::Value(alias),
                    (other, _) => other,
                };
                resolved.merge(candidate);
            }
            if include_remote_rename_targets {
                for item in association
                    .names
                    .iter()
                    .filter(|item| item.local != item.remote && item.remote == lower)
                {
                    resolved.merge(self.module_export_symbol(
                        &association.module,
                        &item.remote,
                        visited,
                    ));
                }
            }
        }
        resolved
    }

    fn imported_type(
        &self,
        imports: &[UseAssociation],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        let mut resolved = Visibility::Absent;
        for association in imports {
            for target in association.targets(name) {
                let remote = self.module_export_type(&association.module, &target.remote, visited);
                let candidate = match (remote, target.alias_spelling) {
                    (Visibility::Value(_), Some(alias)) => Visibility::Value(alias),
                    (other, _) => other,
                };
                resolved.merge(candidate);
            }
        }
        resolved
    }

    fn imported_type_identity(
        &self,
        imports: &[UseAssociation],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let mut resolved = Visibility::Absent;
        for association in imports {
            for target in association.targets(name) {
                resolved.merge(self.module_export_type_identity(
                    &association.module,
                    &target.remote,
                    visited,
                ));
            }
        }
        resolved
    }

    fn imported_variable_type_identity(
        &self,
        imports: &[UseAssociation],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let mut resolved = Visibility::Absent;
        for association in imports {
            for target in association.targets(name) {
                resolved.merge(self.module_export_variable_type_identity(
                    &association.module,
                    &target.remote,
                    visited,
                ));
            }
        }
        resolved
    }

    fn local_type_identity(
        &self,
        local: &FileFacts,
        start_scope: usize,
        name: &[u8],
    ) -> Visibility<ResolvedType> {
        let name = name.to_ascii_lowercase();
        let mut current = Some(start_scope);
        let mut host_visited = HashSet::new();
        while let Some(scope) = current {
            let Some(unit) = local.units.get(&scope) else {
                break;
            };
            if unit.types.contains(&name) {
                return unit
                    .types
                    .get(&name)
                    .map(|_| {
                        Visibility::Value(ResolvedType {
                            origin: TypeOrigin::Local(scope),
                            name: name.clone(),
                        })
                    })
                    .unwrap_or(Visibility::Ambiguous);
            }
            let mut use_visited = HashSet::new();
            match self.imported_type_identity(&unit.imports, &name, &mut use_visited) {
                Visibility::Absent => {}
                found => return found,
            }
            if !unit.host_access.allows(&name) {
                return Visibility::Absent;
            }
            if let Some(host) = unit.semantic_host.as_ref() {
                return self.host_visible_type_identity(host, &name, &mut host_visited);
            }
            current = unit.parent;
        }
        Visibility::Absent
    }

    fn module_visible_type_identity(
        &self,
        module: &[u8],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let module = module.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        if !visited.insert((module.clone(), name.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.module_units(&module), visited, |unit, visited| {
            self.unit_module_type_identity(&module, unit, &name, visited)
        })
    }

    fn unit_module_type_identity(
        &self,
        module: &[u8],
        unit: &UnitFacts,
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        if unit.types.contains(name) {
            return unit
                .types
                .get(name)
                .map(|_| {
                    Visibility::Value(ResolvedType {
                        origin: TypeOrigin::Module(module.to_vec()),
                        name: name.to_vec(),
                    })
                })
                .unwrap_or(Visibility::Ambiguous);
        }
        self.imported_type_identity(&unit.imports, name, visited)
    }

    fn module_export_type_identity(
        &self,
        module: &[u8],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let module = module.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        if !visited.insert((module.clone(), name.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.module_units(&module), visited, |unit, visited| {
            if !unit.access.is_public(&name) {
                return Visibility::Absent;
            }
            self.unit_module_type_identity(&module, unit, &name, visited)
        })
    }

    fn module_export_symbol(
        &self,
        module: &[u8],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        let module = module.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        if !visited.insert((module.clone(), name.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.module_units(&module), visited, |unit, visited| {
            if !unit.access.is_public(&name) {
                return Visibility::Absent;
            }
            self.unit_export_symbol(unit, &name, visited)
        })
    }

    fn unit_export_symbol(
        &self,
        unit: &UnitFacts,
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        if unit.symbols.contains(name) {
            return unit
                .symbols
                .get(name)
                .map(|spelling| Visibility::Value(spelling.to_vec()))
                .unwrap_or(Visibility::Ambiguous);
        }
        self.imported_symbol(&unit.imports, name, false, visited)
    }

    fn module_export_type(
        &self,
        module: &[u8],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        let module = module.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        if !visited.insert((module.clone(), name.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.module_units(&module), visited, |unit, visited| {
            if !unit.access.is_public(&name) {
                return Visibility::Absent;
            }
            self.unit_export_type(unit, &name, visited)
        })
    }

    fn unit_export_type(
        &self,
        unit: &UnitFacts,
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<Vec<u8>> {
        if unit.types.contains(name) {
            return unit
                .types
                .get(name)
                .map(|spelling| Visibility::Value(spelling.to_vec()))
                .unwrap_or(Visibility::Ambiguous);
        }
        self.imported_type(&unit.imports, name, visited)
    }

    fn module_export_variable_type_identity(
        &self,
        module: &[u8],
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let module = module.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        if !visited.insert((module.clone(), name.clone())) {
            return Visibility::Absent;
        }
        merge_definitions(self.module_units(&module), visited, |unit, visited| {
            if !unit.access.is_public(&name) {
                return Visibility::Absent;
            }
            self.unit_export_variable_type_identity(&module, unit, &name, visited)
        })
    }

    fn unit_export_variable_type_identity(
        &self,
        module: &[u8],
        unit: &UnitFacts,
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        if unit.variable_types.contains_key(name) {
            let Some(type_name) = unit.variable_type(name) else {
                return Visibility::Ambiguous;
            };
            let mut type_visited = HashSet::new();
            return self.module_visible_type_identity(module, type_name, &mut type_visited);
        }
        self.imported_variable_type_identity(&unit.imports, name, visited)
    }

    fn type_units<'a>(&'a self, local: &'a FileFacts, owner: &ResolvedType) -> &'a [UnitFacts] {
        match &owner.origin {
            TypeOrigin::Local(scope) => local
                .units
                .get(scope)
                .map(std::slice::from_ref)
                .unwrap_or_default(),
            TypeOrigin::Module(module) => self.module_units(module),
            TypeOrigin::Submodule { ancestor, name } => self
                .submodules
                .get(&(ancestor.clone(), name.clone()))
                .map(Vec::as_slice)
                .unwrap_or_default(),
        }
    }

    fn resolve_type_from_origin(
        &self,
        local: &FileFacts,
        origin: &TypeOrigin,
        name: &[u8],
    ) -> Option<ResolvedType> {
        match origin {
            TypeOrigin::Local(scope) => self.local_type_identity(local, *scope, name).into_option(),
            TypeOrigin::Module(module) => {
                let mut visited = HashSet::new();
                self.module_visible_type_identity(module, name, &mut visited)
                    .into_option()
            }
            TypeOrigin::Submodule {
                ancestor,
                name: submodule,
            } => {
                let host = HostUnit::Submodule {
                    ancestor: ancestor.clone(),
                    name: submodule.clone(),
                };
                let mut visited = HashSet::new();
                self.host_visible_type_identity(&host, name, &mut visited)
                    .into_option()
            }
        }
    }

    fn member_spelling_from_type(
        &self,
        local: &FileFacts,
        owner: &ResolvedType,
        name: &[u8],
        visited: &mut HashSet<ResolvedType>,
    ) -> Option<Vec<u8>> {
        if !visited.insert(owner.clone()) {
            return None;
        }
        let mut resolved = Visibility::Absent;
        for unit in self.type_units(local, owner) {
            resolved.merge(
                self.member_spelling_from_unit(local, unit, owner, name, visited)
                    .map_or(Visibility::Absent, Visibility::Value),
            );
        }
        resolved.into_option()
    }

    fn member_spelling_from_unit(
        &self,
        local: &FileFacts,
        unit: &UnitFacts,
        owner: &ResolvedType,
        name: &[u8],
        visited: &mut HashSet<ResolvedType>,
    ) -> Option<Vec<u8>> {
        if unit.components.contains(&owner.name, name) {
            return unit
                .components
                .get(&owner.name, name)
                .map(ToOwned::to_owned);
        }
        if unit.bound_type_procedures.contains(&owner.name, name) {
            return unit
                .bound_type_procedures
                .get(&owner.name, name)
                .map(ToOwned::to_owned);
        }
        if unit
            .generic_bound_type_procedures
            .contains(&owner.name, name)
        {
            return unit
                .generic_bound_type_procedures
                .get(&owner.name, name)
                .map(ToOwned::to_owned);
        }
        if unit.type_graph.parent_type_is_ambiguous(&owner.name) {
            return None;
        }
        let parent_name = unit.type_graph.parent_type(&owner.name)?;
        let parent = self.resolve_type_from_origin(local, &owner.origin, parent_name)?;
        self.member_spelling_from_type(local, &parent, name, visited)
    }

    fn component_type_from_type(
        &self,
        local: &FileFacts,
        owner: &ResolvedType,
        name: &[u8],
        visited: &mut HashSet<ResolvedType>,
    ) -> Option<ResolvedType> {
        if !visited.insert(owner.clone()) {
            return None;
        }
        let mut resolved = Visibility::Absent;
        for unit in self.type_units(local, owner) {
            resolved.merge(
                self.component_type_from_unit(local, unit, owner, name, visited)
                    .map_or(Visibility::Absent, Visibility::Value),
            );
        }
        resolved.into_option()
    }

    fn component_type_from_unit(
        &self,
        local: &FileFacts,
        unit: &UnitFacts,
        owner: &ResolvedType,
        name: &[u8],
        visited: &mut HashSet<ResolvedType>,
    ) -> Option<ResolvedType> {
        if unit.components.contains(&owner.name, name) {
            if unit
                .type_graph
                .direct_component_type_is_ambiguous(&owner.name, name)
            {
                return None;
            }
            let type_name = unit.type_graph.direct_component_type(&owner.name, name)?;
            return self.resolve_type_from_origin(local, &owner.origin, type_name);
        }
        if unit.type_graph.parent_type_is_ambiguous(&owner.name) {
            return None;
        }
        let parent_name = unit.type_graph.parent_type(&owner.name)?;
        let parent = self.resolve_type_from_origin(local, &owner.origin, parent_name)?;
        self.component_type_from_type(local, &parent, name, visited)
    }
}

fn expand_includes_with(
    path: &Path,
    facts: &FileFacts,
    loader: &mut impl FnMut(&Path) -> Option<FileFacts>,
) -> FileFacts {
    fn visit(
        path: &Path,
        facts: &FileFacts,
        loader: &mut impl FnMut(&Path) -> Option<FileFacts>,
        stack: &mut HashSet<PathBuf>,
    ) -> FileFacts {
        let mut expanded = facts.clone();
        for include in &facts.includes {
            let include_path = path_from_bytes(&include.path);
            let candidate = if include_path.is_absolute() {
                include_path
            } else {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(include_path)
            };
            let candidate = normalize_path(&candidate);
            if !stack.insert(candidate.clone()) {
                continue;
            }
            if let Some(included) = loader(&candidate) {
                let included = visit(&candidate, &included, loader, stack);
                expanded.merge_include_at(include.line, &included);
            }
            stack.remove(&candidate);
        }
        expanded
    }

    let mut stack = HashSet::new();
    stack.insert(normalize_path(path));
    visit(path, facts, loader, &mut stack)
}

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        PathBuf::from(OsString::from_vec(bytes.to_vec()))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let rooted = path.has_root();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::ParentDir) | Some(Component::Prefix(_)) | None => {
                    if !rooted {
                        normalized.push("..");
                    }
                }
                Some(Component::RootDir) => {}
                Some(Component::CurDir) => {
                    unreachable!("normalized paths do not retain current-directory components")
                }
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn fingerprint(source: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Extract the declaration facts of one source buffer.
pub fn analyze_file(source: &[u8]) -> Result<FileFacts, FormatError> {
    analyze_file_with_path(None, source)
}

/// Extract declaration facts for a source whose project path is known.
/// Relative INCLUDE expansion is path-sensitive, so callers formatting a
/// concrete project member should preserve this identity.
pub fn analyze_file_at(path: &Path, source: &[u8]) -> Result<FileFacts, FormatError> {
    analyze_file_with_path(Some(path), source)
}

fn analyze_file_with_path(path: Option<&Path>, source: &[u8]) -> Result<FileFacts, FormatError> {
    let document = Document::from_bytes(source);
    let analysis = document.analyze()?;
    let scopes = ScopeTree::build(&analysis);
    let mut facts = extract(&analysis, &scopes);
    facts.source_id = fingerprint(source);
    facts.source_path = path.map(normalize_path);
    Ok(facts)
}

/// Build a project context from every source in the project.
///
/// The source list is analyzed once up front. INCLUDE resolution first uses
/// that in-memory set (so `.inc` fragments can be supplied without touching
/// the filesystem), then falls back to the filesystem for ordinary CLI use.
pub fn analyze_project<'a, I>(sources: I) -> Result<ProjectContext, FormatError>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let inputs = sources
        .into_iter()
        .map(|(path, source)| (path.to_path_buf(), source))
        .collect::<Vec<_>>();
    let mut analyzed = Vec::with_capacity(inputs.len());
    // Index into `analyzed` rather than a second copy of the facts: only the
    // sources some file actually INCLUDEs are ever cloned out of it.
    let mut lookup = HashMap::with_capacity(inputs.len());
    for (path, source) in &inputs {
        let facts = analyze_file_at(path, source)?;
        lookup.insert(normalize_path(path), analyzed.len());
        analyzed.push((path.clone(), facts));
    }

    let mut context = ProjectContext::empty();
    // One fragment is typically included by many sources, and a nested include
    // tree multiplies that again. Analyze each fragment once and hand out
    // copies, so a shared `constants.inc` is not re-read and re-tokenized once
    // per including file. `expand_includes_with` normalizes before it asks, so
    // the path it passes is already the cache key.
    let mut fragments: HashMap<PathBuf, Option<FileFacts>> = HashMap::new();
    for (path, facts) in &analyzed {
        let expanded = expand_includes_with(path, facts, &mut |candidate| {
            fragments
                .entry(candidate.to_path_buf())
                .or_insert_with(|| {
                    lookup
                        .get(&normalize_path(candidate))
                        .map(|index| analyzed[*index].1.clone())
                        .or_else(|| {
                            fs::read(candidate)
                                .ok()
                                .and_then(|source| analyze_file_at(candidate, &source).ok())
                        })
                })
                .clone()
        });
        context.absorb_expanded(path, facts, expanded);
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::{analyze_file, analyze_file_at, analyze_project, normalize_path, ProjectContext};
    use crate::{
        analysis::names::NameSpace,
        config::{FormatConfig, FormatMode, MacroDefine},
        format_source_with_context,
    };
    use std::path::Path;

    const MODULE: &[u8] = b"module Precision\ninteger, parameter :: dl = 8\nend module Precision\n";
    const USER: &[u8] = b"program p\nuse Precision\nend program p\n";
    const SHOUTER: &[u8] = b"module PRECISION\nend module PRECISION\n";

    #[test]
    fn lexical_path_normalization_preserves_unmatched_parents() {
        assert_eq!(
            normalize_path(Path::new("../foo.f90")),
            Path::new("../foo.f90")
        );
        assert_ne!(
            normalize_path(Path::new("../foo.f90")),
            normalize_path(Path::new("foo.f90"))
        );
        assert_eq!(
            normalize_path(Path::new("src/sub/../../../defs.inc")),
            Path::new("../defs.inc")
        );
    }

    #[test]
    fn include_paths_above_relative_source_directory_do_not_alias_siblings() {
        let target = b"program p\ninclude '../../../defs.inc'\nprint *, valuecase\nend program p\n";
        let project = analyze_project([
            (Path::new("src/sub/target.f90"), target.as_slice()),
            (
                Path::new("../defs.inc"),
                b"integer :: ValueCase\n".as_slice(),
            ),
            (Path::new("defs.inc"), b"integer :: VALUECASE\n".as_slice()),
        ])
        .unwrap();
        let local = analyze_file_at(Path::new("src/sub/target.f90"), target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 2, b"valuecase"),
            Some(b"ValueCase".to_vec())
        );
    }

    #[test]
    fn a_project_wide_agreement_applies_to_a_file_that_does_not_declare_the_name() {
        let project = analyze_project([
            (Path::new("precision.f90"), MODULE),
            (Path::new("user.f90"), USER),
        ])
        .unwrap();
        let local = analyze_file(b"program r\nend program r\n").unwrap();
        let resolver = project.resolver(&local);
        assert_eq!(
            resolver.spelling(NameSpace::Module, b"precision"),
            Some(b"Precision".as_slice())
        );
        assert_eq!(
            project.sources,
            vec![Path::new("precision.f90"), Path::new("user.f90")]
        );
    }

    #[test]
    fn project_wide_disagreement_leaves_the_name_alone() {
        let project =
            analyze_project([(Path::new("a.f90"), MODULE), (Path::new("b.f90"), SHOUTER)]).unwrap();
        let local = analyze_file(b"program r\nend program r\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Module, b"precision"),
            None
        );
    }

    #[test]
    fn a_local_spelling_still_wins_over_the_project() {
        let project = analyze_project([(Path::new("a.f90"), MODULE)]).unwrap();
        let local = analyze_file(b"module PRECISION\nend module PRECISION\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Module, b"precision"),
            Some(b"PRECISION".as_slice())
        );
    }

    #[test]
    fn merging_is_order_independent_for_the_case_tables() {
        let forward = analyze_project([(Path::new("a"), MODULE), (Path::new("b"), USER)]).unwrap();
        let backward = analyze_project([(Path::new("b"), USER), (Path::new("a"), MODULE)]).unwrap();
        assert_eq!(forward.cases, backward.cases);
    }

    #[test]
    fn command_line_defines_join_the_macro_table() {
        let mut project = ProjectContext::empty();
        project.define(&[MacroDefine {
            name: "MPI_Enabled".to_string(),
            value: None,
        }]);
        let local = analyze_file(b"program p\nend\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Symbol, b"mpi_enabled"),
            Some(b"MPI_Enabled".as_slice())
        );
    }

    #[test]
    fn synthetic_project_cases_cover_local_and_project_precedence() {
        let declared = analyze_project([(
            Path::new("declared.f90"),
            b"module SharedName\nend module SharedName\n".as_slice(),
        )])
        .unwrap();
        let no_local = analyze_file(b"program p\nend program p\n").unwrap();
        assert_eq!(
            declared
                .resolver(&no_local)
                .spelling(NameSpace::Module, b"sharedname"),
            Some(b"SharedName".as_slice())
        );

        let split = analyze_project([
            (
                Path::new("a.f90"),
                b"module SplitName\nend module\n".as_slice(),
            ),
            (
                Path::new("b.f90"),
                b"module SPLITNAME\nend module\n".as_slice(),
            ),
        ])
        .unwrap();
        assert_eq!(
            split
                .resolver(&no_local)
                .spelling(NameSpace::Module, b"splitname"),
            None
        );

        let project = analyze_project([(
            Path::new("global.f90"),
            b"module M\ninteger :: Colliding\nend module M\n".as_slice(),
        )])
        .unwrap();
        let local =
            analyze_file(b"module Local\ninteger :: COLLIDING\nend module Local\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Symbol, b"colliding"),
            Some(b"COLLIDING".as_slice())
        );

        let component_project = analyze_project([(
            Path::new("component.f90"),
            b"module C\ntype :: T\ninteger :: Component\nend type T\nend module C\n".as_slice(),
        )])
        .unwrap();
        let component_local = analyze_file(
            b"module L\ntype :: T\ninteger :: COMPONENT\nend type T\ninteger :: Component\nend module L\n",
        )
        .unwrap();
        let resolver = component_project.resolver(&component_local);
        assert_eq!(
            resolver.component_spelling(b"t", b"component"),
            Some(b"COMPONENT".as_slice())
        );
    }

    #[test]
    fn program_top_level_spelling_still_wins_over_a_module() {
        let program = b"program validation\ninteger, parameter :: BJL_RECURRENCE_MAX_L = 25\ncontains\nsubroutine check\ninteger :: value\nvalue = bjl_recurrence_max_l\nend subroutine check\nend program validation\n";
        let module = b"module bessel\ninteger, parameter :: BJL_recurrence_MAX_L = 25\ncontains\nsubroutine check\ninteger :: value\nvalue = bjl_recurrence_max_l\nend subroutine check\nend module bessel\n";
        let project = analyze_project([
            (Path::new("program.f90"), program.as_slice()),
            (Path::new("module.f90"), module.as_slice()),
        ])
        .unwrap();
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            ..FormatConfig::default()
        };
        let program_output = format_source_with_context(program, &project, &config)
            .unwrap()
            .bytes;
        let module_output = format_source_with_context(module, &project, &config)
            .unwrap()
            .bytes;
        let program_use = program_output
            .split(|byte| *byte == b'\n')
            .find(|line| line.starts_with(b"value ="))
            .unwrap();
        let module_use = module_output
            .split(|byte| *byte == b'\n')
            .find(|line| line.starts_with(b"value ="))
            .unwrap();
        assert_eq!(program_use, b"value = BJL_RECURRENCE_MAX_L");
        assert_eq!(module_use, b"value = BJL_recurrence_MAX_L");
    }

    #[test]
    fn private_and_unrelated_modules_do_not_enter_visible_symbol_scope() {
        let api = b"module api\ninteger :: PublicName\nend module api\n";
        let hidden = b"module hidden\nprivate\ninteger :: PUBLICNAME\nend module hidden\n";
        let unrelated = b"module unrelated\ninteger :: publicNAME\nend module unrelated\n";
        let target = b"program p\nuse api\nimplicit none\nprint *, publicname\nend program p\n";
        let project = analyze_project([
            (Path::new("api.f90"), api.as_slice()),
            (Path::new("hidden.f90"), hidden.as_slice()),
            (Path::new("unrelated.f90"), unrelated.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ])
        .unwrap();
        let local = analyze_file(target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 3, b"publicname"),
            Some(b"PublicName".to_vec())
        );
    }

    #[test]
    fn module_exports_follow_transitive_use_and_intermediate_private() {
        let base = b"module base\ninteger :: ExportedName\nend module base\n";
        let middle = b"module middle\nuse base\nend module middle\n";
        let top = b"module top\nuse middle\nend module top\n";
        let target = b"program p\nuse top\nprint *, exportedname\nend program p\n";
        let project = analyze_project([
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("top.f90"), top.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ])
        .unwrap();
        let local = analyze_file(target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 2, b"exportedname"),
            Some(b"ExportedName".to_vec())
        );

        let middle_private =
            b"module middle\nuse base\nprivate :: ExportedName\nend module middle\n";
        let project = analyze_project([
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle_private.as_slice()),
            (Path::new("top.f90"), top.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ])
        .unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 2, b"exportedname"),
            None
        );
    }

    #[test]
    fn transitive_renames_keep_the_local_export_spelling() {
        let base = b"module base\ninteger :: OriginalName\nend module base\n";
        let middle =
            b"module middle\nuse base, only: MiddleName => OriginalName\nend module middle\n";
        let top = b"module top\nuse middle, only: TopName => MiddleName\nend module top\n";
        let target = b"program p\nuse top\nprint *, topname\nend program p\n";
        let project = analyze_project([
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("top.f90"), top.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ])
        .unwrap();
        let local = analyze_file(target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 2, b"topname"),
            Some(b"TopName".to_vec())
        );
    }

    #[test]
    fn uses_are_scoped_to_the_program_unit_that_owns_them() {
        let first = b"module first\ninteger :: SharedName\nend module first\n";
        let second = b"module second\ninteger :: SHAREDNAME\nend module second\n";
        let target = b"module left\nuse first\ncontains\nsubroutine l\nprint *, sharedname\nend\nend module left\nmodule right\nuse second\ncontains\nsubroutine r\nprint *, sharedname\nend\nend module right\n";
        let project = analyze_project([
            (Path::new("first.f90"), first.as_slice()),
            (Path::new("second.f90"), second.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ])
        .unwrap();
        let local = analyze_file(target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 4, b"sharedname"),
            Some(b"SharedName".to_vec())
        );
        assert_eq!(
            project.visible_symbol_spelling(&local, 10, b"sharedname"),
            Some(b"SHAREDNAME".to_vec())
        );
    }

    #[test]
    fn block_variable_types_do_not_escape_the_construct() {
        let source = b"module m\ntype :: First\ninteger :: A\nend type First\ntype :: Second\ninteger :: B\nend type Second\ncontains\nsubroutine s\ntype(Second) :: item\nblock\ntype(First) :: item\nitem%A = 1\nend block\nitem%B = 2\nend subroutine s\nend module m\n";
        let project = analyze_project([(Path::new("scope.f90"), source.as_slice())]).unwrap();
        let local = analyze_file(source).unwrap();
        assert_eq!(
            project
                .visible_variable_type(&local, 12, b"item")
                .map(|ty| ty.name),
            Some(b"first".to_vec())
        );
        assert_eq!(
            project
                .visible_variable_type(&local, 14, b"item")
                .map(|ty| ty.name),
            Some(b"second".to_vec())
        );
    }

    #[test]
    fn include_fragments_join_the_host_scope_and_obey_module_accessibility() {
        let host = b"module host\nprivate\ninclude 'parts/decls.inc'\npublic :: IncludedName\nend module host\n";
        let decls = b"include '../nested.inc'\ninteger :: HiddenName\n";
        let nested = b"integer :: IncludedName\n";
        let target =
            b"program p\nuse host\nprint *, includedname\nprint *, hiddenname\nend program p\n";
        let project = analyze_project([
            (Path::new("src/host.f90"), host.as_slice()),
            (Path::new("src/parts/decls.inc"), decls.as_slice()),
            (Path::new("src/nested.inc"), nested.as_slice()),
            (Path::new("src/target.f90"), target.as_slice()),
        ])
        .unwrap();
        let local = analyze_file(target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 2, b"includedname"),
            Some(b"IncludedName".to_vec())
        );
        assert_eq!(
            project.visible_symbol_spelling(&local, 3, b"hiddenname"),
            None
        );
    }
}
