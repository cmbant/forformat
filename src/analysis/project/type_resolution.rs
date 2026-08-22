use super::{
    visibility::{export_entity_is_public, export_route_is_public, merge_definitions, Visibility},
    ProjectContext,
};
use crate::analysis::declarations::{FileFacts, HostUnit, ModuleNature, UnitFacts, UseAssociation};
use std::collections::HashSet;

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

impl ProjectContext {
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
            match self.imported_variable_type_identity(
                &unit.imports,
                &lower,
                None,
                &mut use_visited,
            ) {
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
        line: usize,
        owner: &ResolvedType,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let local = self.expanded(local);
        self.member_spelling_from_type(local, line, owner, name, &mut HashSet::new())
    }

    /// Resolve the derived type of one component while preserving the component
    /// owner's entity identity. This is used for every intermediate `%` link.
    pub(crate) fn visible_component_type(
        &self,
        local: &FileFacts,
        line: usize,
        owner: &ResolvedType,
        name: &[u8],
    ) -> Option<ResolvedType> {
        let local = self.expanded(local);
        self.component_type_from_type(local, line, owner, name, &mut HashSet::new())
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
        match self.imported_type_identity(&unit.imports, lower, None, &mut use_visited) {
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
        match self.imported_variable_type_identity(&unit.imports, lower, None, &mut use_visited) {
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

    fn imported_type_identity(
        &self,
        imports: &[UseAssociation],
        name: &[u8],
        exporting_unit: Option<&UnitFacts>,
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let mut resolved = Visibility::Absent;
        for association in imports {
            if association.nature == ModuleNature::Intrinsic {
                continue;
            }
            if exporting_unit.is_some_and(|unit| !export_route_is_public(unit, name, association)) {
                continue;
            }
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
        exporting_unit: Option<&UnitFacts>,
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        let mut resolved = Visibility::Absent;
        for association in imports {
            if association.nature == ModuleNature::Intrinsic {
                continue;
            }
            if exporting_unit.is_some_and(|unit| !export_route_is_public(unit, name, association)) {
                continue;
            }
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
            match self.imported_type_identity(&unit.imports, &name, None, &mut use_visited) {
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
        self.imported_type_identity(&unit.imports, name, None, visited)
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
            self.unit_export_type_identity(&module, unit, &name, visited)
        })
    }

    fn unit_export_type_identity(
        &self,
        module: &[u8],
        unit: &UnitFacts,
        name: &[u8],
        visited: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    ) -> Visibility<ResolvedType> {
        if unit.types.contains(name) {
            if !export_entity_is_public(unit, name) {
                return Visibility::Absent;
            }
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
        self.imported_type_identity(&unit.imports, name, Some(unit), visited)
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
            if !export_entity_is_public(unit, name) {
                return Visibility::Absent;
            }
            let Some(type_name) = unit.variable_type(name) else {
                return Visibility::Ambiguous;
            };
            let mut type_visited = HashSet::new();
            return self.module_visible_type_identity(module, type_name, &mut type_visited);
        }
        self.imported_variable_type_identity(&unit.imports, name, Some(unit), visited)
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

    fn member_accessible(
        &self,
        local: &FileFacts,
        line: usize,
        unit: &UnitFacts,
        owner: &ResolvedType,
        name: &[u8],
    ) -> bool {
        let Some(access) = unit.member_access.get(&owner.name) else {
            return true;
        };
        if access.is_public(name) {
            return true;
        }
        self.requester_is_owner(local, line, &owner.origin)
    }

    fn requester_is_owner(&self, local: &FileFacts, line: usize, origin: &TypeOrigin) -> bool {
        if matches!(origin, TypeOrigin::Local(_)) {
            return true;
        }
        let mut current = local.active_unit(line).map(|unit| unit.scope);
        while let Some(scope) = current {
            let Some(unit) = local.units.get(&scope) else {
                break;
            };
            let matches = match origin {
                TypeOrigin::Local(owner_scope) => *owner_scope == scope,
                TypeOrigin::Module(module) => {
                    unit.project_host.as_ref() == Some(&HostUnit::Module(module.clone()))
                        || match unit.semantic_host.as_ref() {
                            Some(HostUnit::Module(host)) => host == module,
                            Some(HostUnit::Submodule { ancestor, .. }) => ancestor == module,
                            None => false,
                        }
                }
                TypeOrigin::Submodule { ancestor, name } => {
                    unit.project_host.as_ref()
                        == Some(&HostUnit::Submodule {
                            ancestor: ancestor.clone(),
                            name: name.clone(),
                        })
                }
            };
            if matches {
                return true;
            }
            current = unit.parent;
        }
        false
    }

    fn member_spelling_from_type(
        &self,
        local: &FileFacts,
        line: usize,
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
                self.member_spelling_from_unit(local, line, unit, owner, name, visited)
                    .map_or(Visibility::Absent, Visibility::Value),
            );
        }
        resolved.into_option()
    }

    fn member_spelling_from_unit(
        &self,
        local: &FileFacts,
        line: usize,
        unit: &UnitFacts,
        owner: &ResolvedType,
        name: &[u8],
        visited: &mut HashSet<ResolvedType>,
    ) -> Option<Vec<u8>> {
        if unit.components.contains(&owner.name, name) {
            if !self.member_accessible(local, line, unit, owner, name) {
                return None;
            }
            return unit
                .components
                .get(&owner.name, name)
                .map(ToOwned::to_owned);
        }
        if unit.bound_type_procedures.contains(&owner.name, name) {
            if !self.member_accessible(local, line, unit, owner, name) {
                return None;
            }
            return unit
                .bound_type_procedures
                .get(&owner.name, name)
                .map(ToOwned::to_owned);
        }
        if unit
            .generic_bound_type_procedures
            .contains(&owner.name, name)
        {
            if !self.member_accessible(local, line, unit, owner, name) {
                return None;
            }
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
        self.member_spelling_from_type(local, line, &parent, name, visited)
    }

    fn component_type_from_type(
        &self,
        local: &FileFacts,
        line: usize,
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
                self.component_type_from_unit(local, line, unit, owner, name, visited)
                    .map_or(Visibility::Absent, Visibility::Value),
            );
        }
        resolved.into_option()
    }

    fn component_type_from_unit(
        &self,
        local: &FileFacts,
        line: usize,
        unit: &UnitFacts,
        owner: &ResolvedType,
        name: &[u8],
        visited: &mut HashSet<ResolvedType>,
    ) -> Option<ResolvedType> {
        if unit.components.contains(&owner.name, name) {
            if !self.member_accessible(local, line, unit, owner, name) {
                return None;
            }
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
        self.component_type_from_type(local, line, &parent, name, visited)
    }
}
