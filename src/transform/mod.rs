//! Text normalization: everything the formatter changes that is not a column.
//!
//! The layers below are ordered from mechanism to policy: `document` is the
//! text a pass mutates, `edit` is how a pass expresses a change, `vocab` is the
//! generated Fortran word lists, and the rule modules are the policy.

pub mod document;
pub mod edit;
pub mod passes;
pub mod pipeline;
pub mod refactor_end;
pub mod vocab;
pub mod vocab_2023;
pub mod whitespace;
