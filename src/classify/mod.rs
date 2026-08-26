mod recognizers;
pub mod statement;

pub use statement::{StatementClass, StatementInfo, StatementKind};

/// Classify a statement, including Fortran 2023 structural spellings.
pub fn classify(input: &[u8]) -> StatementInfo {
    recognizers::classify(input)
}
