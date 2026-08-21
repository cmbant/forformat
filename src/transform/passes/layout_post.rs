//! Steps 17-20: passes that run after the layout engine has placed every line.
//!
//! Alignment owns the post-layout rules that can widen code and their strict
//! no-padding contract. Program-unit separation and generic blank-line limiting
//! are independent invariants. Final output cleanup may shrink overlong visual
//! separator comments to the wrap budget; because those lines are comments and
//! the change is shrink-only, it cannot invalidate statement wrap decisions.

mod alignment;
mod blank_lines;
mod output;
mod unit_spacing;

pub(crate) use alignment::declaration_separator_info;
pub use alignment::{declaration_separator_alignment, trailing_comment_alignment};
pub use blank_lines::limit_blank_lines;
pub use output::output_whitespace;
pub use unit_spacing::program_unit_spacing;

#[cfg(test)]
mod tests;
