//! The normalization pass order.
//!
//! The order is part of the format contract, and the dependencies between
//! passes are real. Case replacement runs *before* the lexical joins, and every
//! pass that changes the line count forces the statement and scope view to be
//! rebuilt (§5.2 of the port plan).
//!
//! Two rules govern any future change here:
//!
//! * a pass that changes a line's **width** must either run before wrapping or
//!   be accounted for by the wrapper. Step 17
//!   ([`passes::layout_post::declaration_separator_alignment`]) is the one
//!   post-layout pass that changes width, in both directions: it adds the
//!   single space a `::` is entitled to on either side, and it compresses an
//!   over-wide authored alignment column. `format::full` pays for both by
//!   measuring the laid-out, step-17-applied document rather than the authored
//!   one — see the budget comments there. Any *new* post-layout pass that
//!   changes width has to extend that measurement, not assume it does not
//!   matter;
//! * a deviation from the Python order must be deliberate and written down,
//!   not discovered by a failing fixture.
//!
//! `--canonicalize-only` is a normalize-only preset, not a parallel formatter.
//! It keeps token/spelling rewrites while suppressing whitespace-only and
//! structural presentation changes. `--rewrap` similarly prepares safe authored
//! continuations for the existing full wrapper rather than choosing breakpoints
//! in a second implementation.

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
    /// built. A pass that returns [`Changed::Structure`] invalidates it.
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
/// Re-analysis is required before a pass that reads statement or scope data
/// whenever an earlier pass may have changed the text it describes. Passes
/// that only need stable configuration/project data, or deliberately ignore
/// `PassContext`, can share the preceding snapshot without observing it. This
/// keeps the conservative freshness rule while avoiding parses that no code in
/// the grouped follow-up passes can consume.
pub fn normalize(
    document: &mut Document,
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let normalize_whitespace = config.mode.normalizes_whitespace();
    if !normalize_whitespace {
        document.preserve_original_line_endings();
    }

    // `!$&...` is only a conditional-compilation sentinel while that stream has
    // an open continuation. Resolve that contextual spelling before the first
    // `SourceBuffer` analysis, then let every later pass use the stable `!$ `
    // spelling. This is a deliberate ordering exception for step 12: the later
    // continuation pass still owns removal of the body-leading `&`.
    if normalize_whitespace && config.style.continuation_markers {
        passes::conditional_continuations::run(document)?;
    }

    // Steps 1-3: macro-name casing, from `-D` and from `#define`.
    with_context(document, project, local, config, passes::case_pass::macros)?;

    // Step 5 needs a fresh statement view after macro casing. Steps 6 and 7 do
    // not inspect their PassContext at all, so they can follow on the same
    // snapshot even though they mutate the document. Canonicalization-only
    // keeps physical line structure, so it deliberately does not join tokens
    // across authored continuation boundaries.
    with_context(document, project, local, config, |document, cx| {
        let mut stage = passes::scoped_case::declared(document, cx)?;
        if normalize_whitespace {
            stage = stage.or(passes::structure::join_lexical_token_continuations(
                document, cx,
            )?);
        }
        if config.style.remove_redundant_parens {
            stage = stage.or(passes::structure::remove_redundant_nested_parentheses(
                document, cx,
            )?);
        }
        Ok(stage)
    })?;

    // A named END restates the name its header already decided, so it is
    // settled from the scope tree rather than from the case tables — and that
    // tree has to be built after the case pass above moved the header. Sharing
    // the separator snapshot is safe in the other direction: this pass only
    // ever rewrites a name in place, byte for byte, so the statement view stays
    // exact for the pass that follows.
    //
    // Redundant statement separators are syntax normalization rather than a
    // wrapping policy, so this stays active when presentation whitespace is
    // preserved: `x = 1;;` and `x = 1;` are spelling choices, not layout. Run
    // it after structural lexical cleanup, and rebuild the statement view
    // afterwards because deleting separators changes source offsets even
    // though it does not change the non-empty statements.
    with_context(document, project, local, config, |document, cx| {
        let mut stage = passes::named_end::sync_names(document, cx)?;
        if config.style.normalize_semicolons {
            stage = stage.or(passes::semicolons::run(document, cx)?);
        }
        Ok(stage)
    })?;

    // Step 11 consumes statement/scope data, so rebuild after the lexical,
    // parenthesis, and separator edits above. Steps 12-13 read only
    // config/project fields from PassContext; they intentionally share this
    // snapshot after line rules.
    with_context(document, project, local, config, |document, cx| {
        let mut stage = passes::line_rules::run(document, cx)?;
        // How a reserved OpenMP directive is spelled is canonicalization,
        // not presentation, so it is not gated on either whitespace policy
        // or `--continuation-markers`; see
        // [`passes::continuations::case_openmp_directives`].
        stage = stage.or(passes::continuations::case_openmp_directives(document, cx)?);
        if normalize_whitespace && config.style.continuation_markers {
            stage = stage.or(passes::continuations::run(document, cx)?);
        }
        Ok(stage)
    })?;

    // Steps 14-15 consume scope and statement structure, so they get a fresh
    // view after every preceding text transformation. Removing a whole RETURN
    // line is presentation/structure policy and therefore outside the
    // canonicalization-only contract.
    if normalize_whitespace && config.style.remove_terminal_return {
        with_context(
            document,
            project,
            local,
            config,
            passes::structure::remove_terminal_procedure_returns,
        )?;
    }

    // Full mode normally obtains END completion from the layout planner. The
    // normalize-only early return used by canonicalization needs the same
    // scope-aware replacement without taking ownership of the authored column.
    if !normalize_whitespace && config.refactor_end {
        passes::canonical_end::run(document, config)?;
    }

    // Rewrap only prepares authored continuations. The existing full wrapper
    // remains the sole owner of final break decisions and its fixed-point
    // width measurement. Re-run line rules after a successful join so spacing
    // at the old continuation seam is normalized by the same rule chain as any
    // ordinary one-line statement.
    if config.mode.wraps() && config.wrap.enabled && config.rewrap {
        let rejoined = with_context(document, project, local, config, passes::rewrap::prepare)?;
        if rejoined == Changed::Structure {
            with_context(document, project, local, config, passes::line_rules::run)?;
        }
    }

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
    let mut widths_changed =
        passes::layout_post::declaration_separator_alignment(document, config)?;
    // After step 17, because a comment's column is measured from the code it
    // follows and step 17 is what settles where that code ends.
    passes::layout_post::trailing_comment_alignment(document, config)?;
    if config.style.program_unit_spacing {
        passes::layout_post::program_unit_spacing(document, config)?;
    }
    if config.style.max_blank_lines.is_some() {
        passes::layout_post::limit_blank_lines(document, config)?;
    }
    // Blank-line limiting can join declaration carriers that step 17 saw as
    // separate blocks. Re-run the width-changing alignment after that merge
    // so the next formatter invocation has no newly introduced padding to add.
    widths_changed |= passes::layout_post::declaration_separator_alignment(document, config)?;
    passes::layout_post::output_whitespace(document, config)?;
    Ok(widths_changed)
}

/// Rebuild the statement view, run one stage, and report what it changed.
///
/// A stage may contain context-free follow-up passes, but every pass that reads
/// `analysis` or `scopes` must see a snapshot built after the most recent text
/// mutation that can affect those facts.
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
