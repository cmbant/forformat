//! Declaration analysis: persistent facts, scoped visibility, type resolution,
//! and statement-level extraction.
//!
//! Scope structure comes from [`super::scope::ScopeTree`]; these modules keep
//! the independent questions of what a file declares, what is visible at a
//! physical line, and how declaration syntax is extracted separate.

mod extract;
mod facts;
mod index;
mod syntax;
mod types;
mod units;

pub use extract::extract;
pub use facts::FileFacts;
pub use index::{scoped_declared_names, DeclaredNameIndex, DeclaredSpelling};
pub use types::TypeMaps;
pub(crate) use units::{
    Accessibility, HostAccess, HostUnit, ModuleNature, UnitFacts, UseAssociation, UseName,
};

#[cfg(test)]
mod tests;
