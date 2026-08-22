use super::types::TypeMaps;
use crate::analysis::{
    names::{CaseMap, ComponentCaseMap},
    scope::ScopeKind,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

/// What a PUBLIC/PRIVATE statement or attribute says about a name. Named
/// rather than a bare `bool` because the two spellings read identically at
/// a call site and only one of them is the module default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Accessibility {
    #[default]
    Public,
    Private,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AccessFacts {
    default_access: Accessibility,
    explicit_private: HashSet<Vec<u8>>,
    explicit_public: HashSet<Vec<u8>>,
}

impl AccessFacts {
    pub(crate) fn set_default(&mut self, access: Accessibility) {
        self.default_access = access;
    }

    pub(crate) fn mark(&mut self, name: &[u8], access: Accessibility) {
        let name = name.to_ascii_lowercase();
        match access {
            Accessibility::Private => {
                self.explicit_public.remove(&name);
                self.explicit_private.insert(name);
            }
            Accessibility::Public => {
                self.explicit_private.remove(&name);
                self.explicit_public.insert(name);
            }
        }
    }

    pub(crate) fn explicit(&self, name: &[u8]) -> Option<Accessibility> {
        let name = name.to_ascii_lowercase();
        if self.explicit_private.contains(&name) {
            Some(Accessibility::Private)
        } else if self.explicit_public.contains(&name) {
            Some(Accessibility::Public)
        } else {
            None
        }
    }

    pub(crate) fn default_is_public(&self) -> bool {
        self.default_access == Accessibility::Public
    }

    pub(crate) fn is_public(&self, name: &[u8]) -> bool {
        match self.explicit(name) {
            Some(Accessibility::Private) => false,
            Some(Accessibility::Public) => true,
            None => self.default_is_public(),
        }
    }

    pub(crate) fn merge(&mut self, other: &Self) {
        if other.default_access == Accessibility::Private {
            self.default_access = Accessibility::Private;
        }
        for name in &other.explicit_private {
            self.mark(name, Accessibility::Private);
        }
        for name in &other.explicit_public {
            self.mark(name, Accessibility::Public);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum HostUnit {
    Module(Vec<u8>),
    Submodule { ancestor: Vec<u8>, name: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostAccessMode {
    All,
    None,
    Only(HashSet<Vec<u8>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostAccess {
    mode: HostAccessMode,
    explicit: bool,
}

impl Default for HostAccess {
    fn default() -> Self {
        Self {
            mode: HostAccessMode::All,
            explicit: false,
        }
    }
}

impl HostAccess {
    pub(crate) fn none_by_default() -> Self {
        Self {
            mode: HostAccessMode::None,
            explicit: false,
        }
    }

    pub(crate) fn set_default_all(&mut self) {
        if !self.explicit {
            self.mode = HostAccessMode::All;
        }
    }

    pub(crate) fn allows(&self, name: &[u8]) -> bool {
        match &self.mode {
            HostAccessMode::All => true,
            HostAccessMode::None => false,
            HostAccessMode::Only(names) => names.contains(&name.to_ascii_lowercase()),
        }
    }

    pub(crate) fn import_all(&mut self) {
        self.mode = HostAccessMode::All;
        self.explicit = true;
    }

    pub(crate) fn import_none(&mut self) {
        self.mode = HostAccessMode::None;
        self.explicit = true;
    }

    pub(crate) fn import_only(&mut self, names: Vec<Vec<u8>>) {
        let names = names
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        if self.explicit {
            if let HostAccessMode::Only(existing) = &mut self.mode {
                existing.extend(names);
                return;
            }
        }
        self.mode = HostAccessMode::Only(names);
        self.explicit = true;
    }

    fn merge(&mut self, other: &Self) {
        if !other.explicit {
            return;
        }
        match &other.mode {
            HostAccessMode::All => self.import_all(),
            HostAccessMode::None => self.import_none(),
            HostAccessMode::Only(names) => self.import_only(names.iter().cloned().collect()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UseName {
    pub(crate) local: Vec<u8>,
    pub(crate) remote: Vec<u8>,
    pub(crate) local_spelling: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UseTarget {
    pub(crate) remote: Vec<u8>,
    pub(crate) alias_spelling: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UseAssociation {
    pub(crate) module: Vec<u8>,
    pub(crate) only: bool,
    pub(crate) names: Vec<UseName>,
}

impl UseAssociation {
    pub(crate) fn targets(&self, name: &[u8]) -> Vec<UseTarget> {
        let name = name.to_ascii_lowercase();
        let explicit = self
            .names
            .iter()
            .filter(|item| item.local == name)
            .map(|item| UseTarget {
                remote: item.remote.clone(),
                alias_spelling: (item.local != item.remote).then(|| item.local_spelling.clone()),
            })
            .collect::<Vec<_>>();
        if !explicit.is_empty() {
            return explicit;
        }
        let hidden_by_rename = self
            .names
            .iter()
            .any(|item| item.local != item.remote && item.remote == name);
        if self.only || hidden_by_rename {
            Vec::new()
        } else {
            vec![UseTarget {
                remote: name,
                alias_spelling: None,
            }]
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnitFacts {
    pub(crate) scope: usize,
    pub(crate) kind: ScopeKind,
    pub(crate) name: Option<Vec<u8>>,
    pub(crate) parent: Option<usize>,
    pub(crate) project_host: Option<HostUnit>,
    pub(crate) semantic_host: Option<HostUnit>,
    pub(crate) host_access: HostAccess,
    pub(crate) lines: Range<usize>,
    pub(crate) symbols: CaseMap,
    pub(crate) types: CaseMap,
    pub(crate) components: ComponentCaseMap,
    pub(crate) bound_type_procedures: ComponentCaseMap,
    pub(crate) generic_bound_type_procedures: ComponentCaseMap,
    pub(crate) type_graph: TypeMaps,
    pub(crate) variable_types: HashMap<Vec<u8>, Option<Vec<u8>>>,
    pub(crate) imports: Vec<UseAssociation>,
    pub(crate) access: AccessFacts,
}

impl UnitFacts {
    pub(crate) fn new(
        scope: usize,
        kind: ScopeKind,
        name: Option<Vec<u8>>,
        parent: Option<usize>,
        lines: Range<usize>,
    ) -> Self {
        let project_host = if kind == ScopeKind::Module {
            name.as_ref().map(|name| HostUnit::Module(name.clone()))
        } else {
            None
        };
        Self {
            scope,
            kind,
            name,
            parent,
            project_host,
            semantic_host: None,
            host_access: HostAccess::default(),
            lines,
            symbols: CaseMap::default(),
            types: CaseMap::default(),
            components: ComponentCaseMap::default(),
            bound_type_procedures: ComponentCaseMap::default(),
            generic_bound_type_procedures: ComponentCaseMap::default(),
            type_graph: TypeMaps::default(),
            variable_types: HashMap::default(),
            imports: Vec::new(),
            access: AccessFacts::default(),
        }
    }

    pub(crate) fn insert_variable_type(&mut self, name: &[u8], type_name: &[u8]) {
        let name = name.to_ascii_lowercase();
        let type_name = type_name.to_ascii_lowercase();
        match self.variable_types.get(&name) {
            None => {
                self.variable_types.insert(name, Some(type_name));
            }
            Some(Some(existing)) if existing != &type_name => {
                self.variable_types.insert(name, None);
            }
            _ => {}
        }
    }

    pub(crate) fn variable_type(&self, name: &[u8]) -> Option<&[u8]> {
        self.variable_types
            .get(&name.to_ascii_lowercase())
            .and_then(Option::as_deref)
    }

    pub(crate) fn merge_fragment(&mut self, other: &Self) {
        self.symbols.merge(&other.symbols);
        self.types.merge(&other.types);
        self.components.merge(&other.components);
        self.bound_type_procedures
            .merge(&other.bound_type_procedures);
        self.generic_bound_type_procedures
            .merge(&other.generic_bound_type_procedures);
        self.type_graph.merge_non_roots(&other.type_graph);
        for (name, type_name) in &other.variable_types {
            match type_name {
                Some(type_name) => self.insert_variable_type(name, type_name),
                None => {
                    self.variable_types.insert(name.clone(), None);
                }
            }
        }
        self.imports.extend(other.imports.iter().cloned());
        self.access.merge(&other.access);
        self.host_access.merge(&other.host_access);
    }
}
