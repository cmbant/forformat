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
    /// built. Any text or structural change invalidates it before the next
    /// context-consuming pass runs.
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

/// Lazily owns the analysis/scope snapshot shared by consecutive no-op stages.
///
/// `Analysis` owns its source buffer, so it does not borrow `Document`: keeping
/// the snapshot is safe while a stage reports [`Changed::No`]. Any text or
/// structural edit drops it immediately, preserving the existing conservative
/// freshness rule for every later stage that reads statement or scope facts.
struct PassContextCache<'a> {
    project: &'a ProjectContext,
    local: &'a FileFacts,
    config: &'a FormatConfig,
    snapshot: Option<(Analysis, ScopeTree)>,
}

impl<'a> PassContextCache<'a> {
    fn new(project: &'a ProjectContext, local: &'a FileFacts, config: &'a FormatConfig) -> Self {
        Self {
            project,
            local,
            config,
            snapshot: None,
        }
    }

    fn run<F>(&mut self, document: &mut Document, pass: F) -> Result<Changed, FormatError>
    where
        F: FnOnce(&mut Document, &PassContext) -> Result<Changed, FormatError>,
    {
        if self.snapshot.is_none() {
            let analysis = document.analyze()?;
            let scopes = ScopeTree::build(&analysis);
            self.snapshot = Some((analysis, scopes));
        }

        let (analysis, scopes) = self.snapshot.as_ref().expect("snapshot was initialized");
        let context = PassContext {
            config: self.config,
            project: self.project,
            local: self.local,
            analysis,
            scopes,
        };
        let changed = pass(document, &context)?;
        if changed != Changed::No {
            self.snapshot = None;
        }
        Ok(changed)
    }

    fn note_change(&mut self, changed: Changed) {
        if changed != Changed::No {
            self.snapshot = None;
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

    let mut contexts = PassContextCache::new(project, local, config);

    // Steps 1-3: macro-name casing, from `-D` and from `#define`.
    contexts.run(document, passes::case_pass::macros)?;

    // Step 5 needs a fresh statement view after macro casing. Steps 6 and 7 do
    // not inspect their PassContext at all, so they can follow on the same
    // snapshot even though they mutate the document. Canonicalization-only
    // keeps physical line structure, so it deliberately does not join tokens
    // across authored continuation boundaries.
    contexts.run(document, |document, cx| {
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
    contexts.run(document, |document, cx| {
        let mut stage = passes::named_end::sync_names(document, cx)?;
        if config.style.normalize_semicolons {
            stage = stage.or(passes::semicolons::run(document, cx)?);
        }
        Ok(stage)
    })?;

    // Declaration modernization changes syntax and width before wrapping. Use
    // the logical statement provenance here so semicolon-separated and
    // continued declarations receive the same rule, then let line rules own
    // the ordinary spacing around the inserted separator.
    if config.style.modernize_declarations {
        contexts.run(document, passes::declaration_separators::run)?;
    }

    // Step 11 consumes statement/scope data, so rebuild after the lexical,
    // parenthesis, separator, and optional declaration edits above. Steps 12-13
    // read only config/project fields from PassContext; they intentionally share
    // this snapshot after line rules.
    contexts.run(document, |document, cx| {
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
        contexts.run(
            document,
            passes::structure::remove_terminal_procedure_returns,
        )?;
    }

    // Full mode normally obtains END completion from the layout planner. The
    // normalize-only early return used by canonicalization needs the same
    // scope-aware replacement without taking ownership of the authored column.
    if !normalize_whitespace && config.refactor_end {
        let changed = passes::canonical_end::run(document, config)?;
        contexts.note_change(changed);
    }

    // Rewrap only prepares authored continuations. The existing full wrapper
    // remains the sole owner of final break decisions and its fixed-point
    // width measurement. Re-run line rules after a successful join so spacing
    // at the old continuation seam is normalized by the same rule chain as any
    // ordinary one-line statement.
    if config.mode.wraps() && config.wrap.enabled && config.rewrap {
        let rejoined = contexts.run(document, passes::rewrap::prepare)?;
        if rejoined == Changed::Structure {
            contexts.run(document, passes::line_rules::run)?;
        }
    }

    // Trailing horizontal whitespace is invisible output policy, but wrapping
    // and layout must not make decisions from bytes that step 20 will delete.
    // Normalize it here as well; the stream-aware helper preserves literal and
    // Hollerith payload blanks that are real source bytes.
    passes::layout_post::trim_trailing_horizontal(document);

    Ok(())
}

/// Prepare only wrapper-generated continuations for an internal rewrap round.
///
/// Full normalization has already reached a fixed point before wrapping. The
/// only new normalization evidence is at the continuation seams the wrapper
/// itself emitted, and [`passes::rewrap::prepare_settlement`] canonicalizes
/// those while preserving every unrelated line and its project-case decision.
pub(crate) fn prepare_rewrap_settlement(
    document: &mut Document,
    project: &ProjectContext,
    local: &FileFacts,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let mut contexts = PassContextCache::new(project, local, config);
    contexts.run(document, passes::rewrap::prepare_settlement)?;
    passes::layout_post::trim_trailing_horizontal(document);
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

#[cfg(test)]
mod tests {
    use super::{Changed, PassContextCache};
    use crate::{
        analysis::{FileFacts, ProjectContext},
        config::FormatConfig,
        format_source,
        transform::document::Document,
    };

    #[test]
    fn change_levels_combine_to_the_strongest() {
        assert_eq!(Changed::No.or(Changed::No), Changed::No);
        assert_eq!(Changed::No.or(Changed::Text), Changed::Text);
        assert_eq!(Changed::Text.or(Changed::Structure), Changed::Structure);
        assert_eq!(Changed::Structure.or(Changed::No), Changed::Structure);
    }

    #[test]
    fn context_snapshot_is_rebuilt_after_text_change() {
        let mut document = Document::from_bytes(b"x = 1\n");
        let project = ProjectContext::empty();
        let local = FileFacts::default();
        let config = FormatConfig::default();
        let mut contexts = PassContextCache::new(&project, &local, &config);

        contexts
            .run(&mut document, |_, cx| {
                assert_eq!(cx.analysis.groups[0].statements[0].text, b"x = 1");
                Ok(Changed::No)
            })
            .unwrap();
        contexts
            .run(&mut document, |document, cx| {
                assert_eq!(cx.analysis.groups[0].statements[0].text, b"x = 1");
                document.lines[0] = b"x = 2".to_vec();
                Ok(Changed::Text)
            })
            .unwrap();
        contexts
            .run(&mut document, |_, cx| {
                assert_eq!(cx.analysis.groups[0].statements[0].text, b"x = 2");
                Ok(Changed::No)
            })
            .unwrap();
    }

    #[test]
    fn label_only_trailing_whitespace_is_idempotent() {
        let source = b"program main\nconti end\n10 \n";
        let config = FormatConfig::default();
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice);
    }
}
