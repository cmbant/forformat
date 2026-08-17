use super::types::TypeMaps;
use crate::analysis::names::{CaseMap, CaseTables, ComponentCaseMap};

/// Everything one file contributes to the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// Spellings this file declares, per name space.
    pub cases: CaseTables,
    /// File-wide symbol declarations, excluding procedure locals.
    pub file_symbols: CaseMap,
    /// Generic bindings, separate from explicit PROCEDURE bindings.
    pub generic_type_procedures: CaseMap,
    /// Generic bindings keyed by owner type.
    pub generic_bound_type_procedures: ComponentCaseMap,
    /// Derived-type definitions only (not TYPE(...) use sites).
    pub declared_types: CaseMap,
    /// Macro names defined by `#define` in this file.
    pub macros: CaseMap,
    /// Declared type relationships used to resolve component chains.
    pub types: TypeMaps,
    /// Module associations retain enough USE information to resolve imported
    /// derived-type values without treating common names as project-global.
    pub(super) imports: Vec<UseAssociation>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct UseAssociation {
    pub(super) module: Vec<u8>,
    pub(super) only: bool,
    /// `(local_name, remote_name)`, normalized to lowercase.
    pub(super) names: Vec<(Vec<u8>, Vec<u8>)>,
}

impl FileFacts {
    pub fn merge(&mut self, other: &FileFacts) {
        self.cases.merge(&other.cases);
        self.file_symbols.merge(&other.file_symbols);
        self.generic_type_procedures
            .merge(&other.generic_type_procedures);
        self.generic_bound_type_procedures
            .merge(&other.generic_bound_type_procedures);
        self.declared_types.merge(&other.declared_types);
        self.macros.merge(&other.macros);
        self.types.merge(&other.types);
        self.imports.extend(other.imports.iter().cloned());
    }

    /// Resolve a root name through this file's USE associations. Multiple
    /// imports are accepted only when they agree on the derived type.
    pub(crate) fn imported_variable_type(
        &self,
        project: &TypeMaps,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let name = name.to_ascii_lowercase();
        let mut resolved: Option<Vec<u8>> = None;
        for association in &self.imports {
            let explicit = association
                .names
                .iter()
                .filter(|(local, _)| local == &name)
                .map(|(_, remote)| remote.as_slice())
                .collect::<Vec<_>>();
            let hidden_by_rename = association
                .names
                .iter()
                .any(|(local, remote)| local != remote && remote == &name);
            let remotes: Vec<&[u8]> = if !explicit.is_empty() {
                explicit
            } else if !association.only && !hidden_by_rename {
                vec![name.as_slice()]
            } else {
                Vec::new()
            };
            for remote in remotes {
                let Some(candidate) = project.module_variable_type(&association.module, remote)
                else {
                    continue;
                };
                if resolved
                    .as_ref()
                    .is_some_and(|existing| existing.as_slice() != candidate)
                {
                    return None;
                }
                resolved = Some(candidate.to_vec());
            }
        }
        resolved
    }
}
