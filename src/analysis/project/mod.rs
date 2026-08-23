//! Project-wide context: declarations plus the visibility graph connecting
//! Fortran scopes.
//!
//! Project-wide case tables remain useful for global namespaces such as module
//! names and owner-qualified components. Ordinary variables and derived-type
//! roots are resolved separately through scope-owned USE associations so a
//! private or unrelated module entity cannot influence a target merely by
//! existing elsewhere in the checkout.

use super::{
    declarations::{FileFacts, UnitFacts},
    names::{CaseMap, CaseTables},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

mod context;
mod includes;
mod type_resolution;
mod visibility;

pub use includes::{analyze_file, analyze_file_at, analyze_project, analyze_project_with_includes};
pub(crate) use type_resolution::ResolvedType;

pub(crate) fn absorb_analyzed<'a, I>(context: &mut ProjectContext, sources: I)
where
    I: IntoIterator<Item = (&'a Path, &'a FileFacts)>,
{
    includes::absorb_analyzed(context, sources);
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

#[cfg(test)]
mod tests;
