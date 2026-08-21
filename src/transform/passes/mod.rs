//! One module per group of normalization passes, ordered as in
//! [`super::pipeline`].
//!
//! Every pass has a narrow signature and leaves the document untouched when it
//! has nothing to change. A pass that is not implemented returns
//! [`super::pipeline::Changed::No`], so full mode is never half-transformed.

pub mod canonical_end;
pub mod case_pass;
pub mod conditional_continuations;
pub mod continuations;
pub mod layout_post;
pub mod line_rules;
pub mod rewrap;
pub mod scoped_case;
pub mod semicolons;
pub mod structure;
