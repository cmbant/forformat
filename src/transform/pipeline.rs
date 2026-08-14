//! The normalization pass order.
//!
//! The order is not a design choice we are free to make: it is the frozen
//! Python reference's `format_text` (`standardize_fortran.py:3887-4006`), and
//! the dependencies between passes are real.  Case replacement runs *before*
//! the lexical joins, and every pass that changes the line count forces the
//! statement and scope view to be rebuilt (§5.2 of the port plan).
//!
//! Two rules govern any future change here:
//!
//! * a pass that changes a line's **width** must either run before wrapping or
//!   be accounted for by the wrapper.  Step 17
//!   ([`passes::layout_post::declaration_separator_alignment`]) is the one
//!   post-layout pass that changes width, in both directions: it adds the
//!   single space a `::` is entitled to on either side, and it compresses an
//!   over-wide authored alignment column.  `format::full` pays for both by
//!   measuring the laid-out, step-17-applied document rather than the authored
//!   one — see the budget comments there.  Any *new* post-layout pass that
//!   changes width has to extend that measurement, not assume it does not
//!   matter;
//! * a deviation from the Python order must be deliberate and written down,
//!   not discovered by a failing fixture.

use super::{document::Document, passes};
use crate::{
    analysis::{names::CaseResolver, FileFacts, ProjectContext, ScopeTree},
    config::FormatConfig,
    error::FormatError,
    transform::document::Analysis,
};

/// Whether a pass changed anything, and whether the change invalidated the
/// statement view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Changed {
    /// Nothing changed.
    No,
    /// Line contents changed, but the line count did not.
    Text,
    /// Lines were added or removed: scope and statement metadata are stale.
    Structure,
}

impl Changed {
    pub fn or(self, other: Changed) -> Changed {
        match (self, other) {
            (Changed::Structure, _) | (_, Changed::Structure) => Changed::Structure,
            (Changed::Text, _) | (_, Changed::Text) => Changed::Text,
            _ => Changed::No,
        }
    }
}

/// Everything a pass may read.
pub struct PassContext<'a> {
    pub config: &'a FormatConfig,
    pub project: &'a ProjectContext,
    /// This file's own declarations, which outrank the project's (I4).
    pub local: &'a FileFacts,
    /// The statement view of the document as it was when this context was
    /// built.  A pass that returns [`Changed::Structure`] invalidates it.
    pub analysis: &'a Analysis,
    pub scopes: &'a ScopeTree,
}

impl<'a> PassContext<'a> {
    pub fn resolver(&self) -> CaseResolver<'a> {
        CaseResolver {
            local: &self.local.cases,
            project: &self.project.cases,
            macros: &self.project.macros,
        }
    }
}

/// Steps 1-15: everything before wrapping.
///
/// The re-analysis points are the load-bearing part.  A pass that removes a
/// `RETURN` or joins a split token changes which physical line a statement
/// starts on, and every later pass that consults scope information would
/// otherwise read stale line numbers.
pub fn normalize(
    document: &mut Document,
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let mut changed;

    // Steps 1-3: macro-name casing, from `-D` and from `#define`.
    changed = with_context(document, project, local, config, passes::case_pass::macros)?;

    // Step 5: the whole declared-case engine, which runs before the joins.
    changed = changed.or(with_context(
        document,
        project,
        local,
        config,
        passes::case_pass::declared,
    )?);

    // Step 6: rejoin `&` splits that cut a token in half.
    changed = changed.or(with_context(
        document,
        project,
        local,
        config,
        passes::structure::join_lexical_token_continuations,
    )?);

    // Step 7: drop redundant nested parentheses where it is safe.
    changed = changed.or(with_context(
        document,
        project,
        local,
        config,
        passes::structure::remove_redundant_nested_parentheses,
    )?);

    // Step 11: the per-line rule chain, in this exact order.
    changed = changed.or(with_context(
        document,
        project,
        local,
        config,
        passes::line_rules::run,
    )?);

    // Steps 12-13: continuation markers and OpenMP sentinels.
    changed = changed.or(with_context(
        document,
        project,
        local,
        config,
        passes::continuations::run,
    )?);

    // Steps 14-15: terminal `RETURN` removal, which changes the line count.
    let _ = changed.or(with_context(
        document,
        project,
        local,
        config,
        passes::structure::remove_terminal_procedure_returns,
    )?);
    Ok(())
}

/// Steps 17-20, run on the laid-out text.
///
/// A pass here may change width only when wrapping has already measured its
/// emitted spelling. Step 17 can add the spaces owed around a compact `::` or
/// compress authored alignment; the caller lays the text out again whenever
/// that changes a width. Returns whether step 17 changed any line's width — see
/// [`passes::layout_post::declaration_separator_alignment`] and the caller in
/// `format::full`, which has to lay the text out again when it did.
pub fn post_layout(document: &mut Document, config: &FormatConfig) -> Result<bool, FormatError> {
    let widths_changed = passes::layout_post::declaration_separator_alignment(document, config)?;
    // After step 17, because a comment's column is measured from the code it
    // follows and step 17 is what settles where that code ends.
    passes::layout_post::trailing_comment_alignment(document, config)?;
    passes::layout_post::program_unit_spacing(document, config)?;
    passes::layout_post::limit_blank_lines(document, config)?;
    passes::layout_post::output_whitespace(document, config)?;
    Ok(widths_changed)
}

/// Rebuild the statement view, run one pass, and report what it changed.
///
/// Rebuilding before every pass is deliberately simple rather than clever: a
/// pass that needs no context pays a parse it does not use, but no pass can
/// ever read a stale one.  Passes that measurably matter can take a cached
/// context later, once the corpus check can prove the two agree.
fn with_context<F>(
    document: &mut Document,
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
    pass: F,
) -> Result<Changed, FormatError>
where
    F: FnOnce(&mut Document, &PassContext) -> Result<Changed, FormatError>,
{
    let analysis = document.analyze()?;
    let scopes = ScopeTree::build(&analysis);
    let context = PassContext {
        config,
        project,
        local,
        analysis: &analysis,
        scopes: &scopes,
    };
    pass(document, &context)
}

#[cfg(test)]
mod tests {
    use super::Changed;

    #[test]
    fn change_levels_combine_to_the_strongest() {
        assert_eq!(Changed::No.or(Changed::No), Changed::No);
        assert_eq!(Changed::No.or(Changed::Text), Changed::Text);
        assert_eq!(Changed::Text.or(Changed::Structure), Changed::Structure);
        assert_eq!(Changed::Structure.or(Changed::No), Changed::Structure);
    }
}
