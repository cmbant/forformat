//! Steps 1-5: macro casing and the declared-case engine.
//!
//! These run **before** the lexical joins, not after — `format_text` step 5
//! precedes step 6 — because joining tokens changes offsets that case
//! replacement was computed against.

use crate::{
    error::FormatError,
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
    },
};

/// Steps 1-3: apply the spelling of every known macro name.
///
/// Sources of macro names, in the order the reference collects them:
/// `-D NAME[=VALUE]` from the command line, then every `#define NAME` in the
/// project.  Both are already gathered into `ProjectContext::macros`; what is
/// missing is the replacement itself, in unquoted code only.
///
/// Port target: `standardize_fortran.py:3900-3920` and `CPP_DEFINE`.
pub fn macros(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}

/// Step 5: `replace_declared_cases`, the whole case-normalization engine.
///
/// The resolution policy is already implemented and tested in
/// [`crate::analysis::names`]; what this pass adds is *finding the occurrences*:
/// every identifier in unquoted code, classified into the right name space —
/// module names in `USE`, type names after `TYPE(`/`CLASS(`, components after
/// `%` resolved through the type maps, type-bound procedure names, and plain
/// symbols everywhere else.
///
/// Port target: `replace_declared_cases` and `_case_for_file`
/// (`standardize_fortran.py:1589`).
pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}
