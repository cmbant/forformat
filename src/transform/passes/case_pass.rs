//! Steps 1-5: macro casing and declared-name spelling.
//!
//! The case pass is split by concern so association tracking, syntax
//! recognition, component-owner resolution, and spelling policy can evolve
//! independently without changing the formatter pipeline boundary.

mod associations;
mod declared;
mod macros;
mod members;
mod syntax;

#[cfg(test)]
mod tests;

pub(crate) use declared::{declared, restore_declined_component_spellings};
pub(super) use declared::{declared_with_names_and_evidence, CaseEvidence};
pub(crate) use macros::macros;
