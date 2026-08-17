use std::collections::{HashMap, HashSet};

/// Name-to-type mappings. Unlike case maps, keys and values are normalized:
/// these answer "what type is this?", not "how is it spelt?".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeMaps {
    /// Compatibility map for unscoped local facts.
    pub local_types: HashMap<Vec<u8>, Vec<u8>>,
    local_type_ambiguities: HashSet<Vec<u8>>,
    /// Types known for names local to each procedure, keyed by procedure name.
    pub procedure_local_types: HashMap<Vec<u8>, HashMap<Vec<u8>, Vec<u8>>>,
    procedure_local_type_ambiguities: HashMap<Vec<u8>, HashSet<Vec<u8>>>,
    /// Variable name (lowercase) to derived type (lowercase).
    pub variable_types: HashMap<Vec<u8>, Vec<u8>>,
    variable_type_ambiguities: HashSet<Vec<u8>>,
    /// Module-qualified variables retain the namespace needed by USE lookup.
    module_variable_types: HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
    module_variable_type_ambiguities: HashSet<(Vec<u8>, Vec<u8>)>,
    /// `(type, component)` to the component's own derived type.
    pub component_types: HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
    component_type_ambiguities: HashSet<(Vec<u8>, Vec<u8>)>,
    /// A derived type's direct parent when declared with EXTENDS.
    pub parent_types: HashMap<Vec<u8>, Vec<u8>>,
    parent_type_ambiguities: HashSet<Vec<u8>>,
}

impl TypeMaps {
    /// Later files never override an earlier disagreement. Ambiguous entries
    /// are tombstoned so later evidence cannot make them resolvable again.
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
            &mut self.module_variable_types,
            &mut self.module_variable_type_ambiguities,
            &other.module_variable_types,
            &other.module_variable_type_ambiguities,
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

    pub(super) fn insert_module_variable(&mut self, module: &[u8], name: &[u8], type_name: &[u8]) {
        insert_agreed_component_type(
            &mut self.module_variable_types,
            &mut self.module_variable_type_ambiguities,
            module,
            name,
            type_name,
        );
    }

    pub(super) fn module_variable_type(&self, module: &[u8], name: &[u8]) -> Option<&[u8]> {
        let key = (module.to_ascii_lowercase(), name.to_ascii_lowercase());
        if self.module_variable_type_ambiguities.contains(&key) {
            return None;
        }
        self.module_variable_types.get(&key).map(Vec::as_slice)
    }

    pub fn insert_component(&mut self, owner: &[u8], name: &[u8], type_name: &[u8]) {
        insert_agreed_component_type(
            &mut self.component_types,
            &mut self.component_type_ambiguities,
            owner,
            name,
            type_name,
        );
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
    /// components. A cycle or ambiguous parent relation is unresolved.
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

    /// Whether any procedure-local declaration in this file owns `root`.
    pub fn has_procedure_local_root(&self, root: &[u8]) -> bool {
        let root = root.to_ascii_lowercase();
        self.procedure_local_types
            .values()
            .any(|types| types.contains_key(&root))
            || self
                .procedure_local_type_ambiguities
                .values()
                .any(|names| names.contains(&root))
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

fn insert_agreed_component_type(
    map: &mut HashMap<(Vec<u8>, Vec<u8>), Vec<u8>>,
    ambiguities: &mut HashSet<(Vec<u8>, Vec<u8>)>,
    owner: &[u8],
    name: &[u8],
    type_name: &[u8],
) {
    let key = (owner.to_ascii_lowercase(), name.to_ascii_lowercase());
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
