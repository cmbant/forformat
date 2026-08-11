//! One module per group of normalization passes, ordered as in
//! [`super::pipeline`].
//!
//! Every pass has its final signature and its Python reference recorded in the
//! doc comment, whether or not its body is written yet.  A pass that is not
//! implemented returns [`super::pipeline::Changed::No`] and leaves the document
//! untouched, so full mode is always *correct on what it does and inert on what
//! it does not* — never half-transformed.

pub mod case_pass;
pub mod continuations;
pub mod layout_post;
pub mod line_rules;
pub mod structure;
