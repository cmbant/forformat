//! Semantic analysis: what a file and a project declare.
//!
//! Nothing in this layer rewrites text.  It answers questions — which scope is
//! this line in, what spelling did the project agree on for this name — and the
//! `transform` layer decides what to do about the answers.

pub mod declarations;
mod implicit;
pub mod names;
pub mod project;
pub mod scope;

pub use declarations::{
    scoped_declared_names, DeclaredNameIndex, DeclaredSpelling, FileFacts, TypeMaps,
};
pub(crate) use implicit::is_implicit_letter_name;
pub use names::{CaseMap, CaseResolver, CaseTables, ComponentCaseMap, NameSpace};
pub use project::{
    analyze_file, analyze_file_at, analyze_project, analyze_project_with_includes, ProjectContext,
};
pub use scope::{Scope, ScopeKind, ScopeTree};
