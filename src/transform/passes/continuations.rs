//! Steps 12-13: continuation markers and OpenMP sentinels.

use crate::{
    error::FormatError,
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
    },
};

/// Step 12-13 driver.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let changed = normalize_continuations(document, cx)?;
    Ok(changed.or(normalize_openmp_continuation_sentinels(document, cx)?))
}

/// Step 12: normalize continuation markers.
///
/// The reference strips a *leading* `&` from continuation lines.  Rust keeps
/// that rule for pre-existing markers and never emits one, which is what makes
/// findent's `-K` (`--indent_ampersand`) inert on already-formatted source
/// rather than contradictory: `-K` governs where an existing leading `&` sits,
/// and the wrapper simply never creates one (§7.1 of the port plan).
///
/// Port target: `normalize_continuations`.
pub fn normalize_continuations(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}

/// Step 13: OpenMP continuation sentinels.
///
/// A continued directive needs a repeated `!$OMP` on each physical line with
/// valid `&` markers, and the available width has to account for the sentinel.
/// Note that `--openmp=0` in the CAMB profile disables findent's OpenMP
/// *indentation* while directive *text* normalization stays on: two concerns,
/// two config fields, never one flag.
///
/// Port target: `normalize_openmp_continuation_sentinels`,
/// `join_openmp_directive`, `wrap_openmp_directive`.
pub fn normalize_openmp_continuation_sentinels(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}
