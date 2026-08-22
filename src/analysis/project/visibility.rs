use super::ProjectContext;
use crate::analysis::declarations::{FileFacts, HostUnit, UnitFacts, UseAssociation};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Visibility<T> {
    Absent,
    Value(T),
    Ambiguous,
}

impl<T: Eq + Clone> Visibility<T> {
    pub(super) fn merge(&mut self, other: Self) {
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

    pub(super) fn into_option(self) -> Option<T> {
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
pub(super) fn merge_definitions<K, T>(
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

impl ProjectContext {
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
}
