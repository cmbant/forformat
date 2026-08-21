//! Steps 17-20: passes that run after the layout engine has placed every line.
//!
//! Alignment owns the only post-layout width-changing rules and their strict
//! no-padding contract. Program-unit separation, generic blank-line limiting,
//! and final output cleanup are independent invariants and live separately.

mod alignment;
mod blank_lines;
mod output;
mod unit_spacing;

pub(crate) use alignment::declaration_separator_info;
pub use alignment::{declaration_separator_alignment, trailing_comment_alignment};
pub use blank_lines::limit_blank_lines;
pub use output::{output_whitespace, trim_trailing_horizontal};
pub use unit_spacing::program_unit_spacing;

#[cfg(test)]
mod tests;
