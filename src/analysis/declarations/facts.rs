use super::{types::TypeMaps, UnitFacts};
use crate::analysis::{
    names::{CaseMap, CaseTables, ComponentCaseMap},
    scope::ScopeKind,
};
use std::{cmp::Reverse, collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IncludeDirective {
    pub(crate) line: usize,
    pub(crate) path: Vec<u8>,
}

/// Everything one file contributes to the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// Spellings this file declares, per name space.
    pub cases: CaseTables,
    /// File-wide symbol declarations, excluding procedure locals.
    pub file_symbols: CaseMap,
    /// Standalone external procedures declared at file scope.
    pub(crate) external_symbols: CaseMap,
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
    /// Program-unit/construct-owned declarations and USE associations. Scope
    /// indices are those of the ScopeTree used for extraction.
    pub(crate) units: HashMap<usize, UnitFacts>,
    /// Fortran INCLUDE statements, resolved by project construction where a
    /// source path is available.
    pub(crate) includes: Vec<IncludeDirective>,
    /// Stable fingerprint of the authored source. ProjectContext uses this as
    /// a conservative fallback when no path identity is available.
    pub(crate) source_id: u64,
    /// Normalized source path when analysis was performed for a concrete
    /// project member. Identical buffers in different directories must remain
    /// distinct because their relative INCLUDE statements can resolve
    /// differently.
    pub(crate) source_path: Option<PathBuf>,
}

impl FileFacts {
    pub fn merge(&mut self, other: &FileFacts) {
        self.cases.merge(&other.cases);
        self.file_symbols.merge(&other.file_symbols);
        self.external_symbols.merge(&other.external_symbols);
        self.generic_type_procedures
            .merge(&other.generic_type_procedures);
        self.generic_bound_type_procedures
            .merge(&other.generic_bound_type_procedures);
        self.declared_types.merge(&other.declared_types);
        self.macros.merge(&other.macros);
        self.types.merge(&other.types);
        for (scope, unit) in &other.units {
            match self.units.get_mut(scope) {
                Some(existing) => existing.merge_fragment(unit),
                None => {
                    self.units.insert(*scope, unit.clone());
                }
            }
        }
        self.includes.extend(other.includes.iter().cloned());
    }

    /// The narrowest unit covering `line`.
    ///
    /// This is on the per-token query path, so a linear scan here is an
    /// O(tokens x units) factor in principle. In practice the largest file in
    /// the reference corpora owns 74 units and a precomputed line index
    /// measured within noise of the scan while costing a vector per file, so
    /// the scan stays.
    pub(crate) fn active_unit(&self, line: usize) -> Option<&UnitFacts> {
        self.units
            .values()
            .filter(|unit| unit.lines.start <= line && line < unit.lines.end)
            // A top-level program unit can have exactly the same line range as
            // the synthetic file scope. ScopeTree allocates parents before
            // children, so prefer the larger scope id when spans tie.
            .min_by_key(|unit| {
                (
                    unit.lines.end.saturating_sub(unit.lines.start),
                    Reverse(unit.scope),
                )
            })
    }

    pub(crate) fn unit_chain(&self, line: usize) -> Vec<&UnitFacts> {
        let mut chain = Vec::new();
        let mut current = self.active_unit(line).map(|unit| unit.scope);
        while let Some(scope) = current {
            let Some(unit) = self.units.get(&scope) else {
                break;
            };
            chain.push(unit);
            current = unit.parent;
        }
        chain
    }

    /// Compatibility path for the existing case pass while scope-aware lookup
    /// is wired in. It intentionally folds all units and therefore must not be
    /// used by new relevance-sensitive code.
    pub(crate) fn imported_variable_type(
        &self,
        project: &TypeMaps,
        name: &[u8],
    ) -> Option<Vec<u8>> {
        let mut resolved: Option<Vec<u8>> = None;
        for unit in self.units.values() {
            for association in &unit.imports {
                for target in association.targets(name) {
                    let Some(candidate) =
                        project.module_variable_type(&association.module, &target.remote)
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
        }
        resolved
    }

    /// Merge a textually included fragment into the scope containing its
    /// INCLUDE statement. The fragment's file scope becomes that host scope;
    /// nested INCLUDEs have already been folded into the fragment's file scope.
    pub(crate) fn merge_include_at(&mut self, line: usize, included: &FileFacts) {
        let host_scope = self.active_unit(line).map(|unit| unit.scope).unwrap_or(0);
        let Some(fragment) = included.units.get(&0).cloned() else {
            return;
        };
        let host_is_file = self
            .units
            .get(&host_scope)
            .is_some_and(|unit| unit.kind == ScopeKind::File);
        if let Some(host) = self.units.get_mut(&host_scope) {
            host.merge_fragment(&fragment);
        }

        // Type members and inheritance are owner-qualified and remain useful
        // wherever the included type is visible. Root variable/type visibility
        // stays scope-owned above and is deliberately not promoted file-wide.
        self.cases.components.merge(&included.cases.components);
        self.cases
            .bound_type_procedures
            .merge(&included.cases.bound_type_procedures);
        self.cases
            .type_procedures
            .merge(&included.cases.type_procedures);
        self.generic_type_procedures
            .merge(&included.generic_type_procedures);
        self.generic_bound_type_procedures
            .merge(&included.generic_bound_type_procedures);
        self.types.merge_non_roots(&included.types);
        self.macros.merge(&included.macros);

        // A fragment included at true file scope has no narrower host to protect
        // it, so retain the legacy file-wide compatibility maps there.
        if host_is_file {
            self.file_symbols.merge(&included.file_symbols);
            self.external_symbols.merge(&included.external_symbols);
            self.declared_types.merge(&included.declared_types);
            self.cases.symbols.merge(&included.cases.symbols);
            self.cases.types.merge(&included.cases.types);
            self.types.merge(&included.types);
        }
    }
}
