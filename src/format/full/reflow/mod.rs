//! Step-16 statement and directive reflow.
//!
//! This module owns the wrapper's stable inputs and emission decisions. Round
//! convergence lives in [`rounds`], directive-specific handling in [`sentinel`],
//! and all engine measurement goes through `full::layout`.

mod rounds;
mod sentinel;

use super::layout;
use crate::{
    analysis::{analyze_file, scoped_declared_names, ProjectContext, ScopeTree},
    config::FormatConfig,
    error::FormatError,
    format::{
        planner::{GroupPlan, PlanBody, Planner},
        wrapping::{self, ContinuationLayout, Decline},
    },
    source::{
        regions::StreamLexStates,
        syntax::{conditional_compilation_prefix, is_directive_comment, SourceStream},
        LogicalGroup, PhysicalLineKind, SourceBuffer,
    },
    transform::{document::Document, pipeline},
    FormatMeta,
};

type ReflowResult = Vec<(usize, Decline)>;

#[cfg(test)]
pub(super) use rounds::{fixed_point_progress, FixedPointProgress};

/// Run the full wrapping path, including optional `--rewrap` settlement and the
/// final converged layout.
pub(super) fn format_wrapped(
    document: &mut Document,
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<(Document, FormatMeta), FormatError> {
    // Rewrap settlement needs the normalized logical input that produced the
    // first output. Capture it before reflow mutates `document`, and avoid the
    // allocation entirely for ordinary wrapping.
    let settlement_input = config.rewrap.then(|| document.to_bytes());
    let (output, meta) = reflow_and_lay_out(document, project, local, config)?;
    match settlement_input {
        Some(input) => rounds::settle_rewrap(output, meta, input, project, local, config),
        None => Ok((output, meta)),
    }
}

/// Wrap one normalized document and apply the layout/post-layout stages whose
/// output determines both the next rewrap round and the final diagnostics.
fn reflow_and_lay_out(
    document: &mut Document,
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<(Document, FormatMeta), FormatError> {
    let declined = reflow_with_context_inner(document, project, local, config)?;
    // Every long line the wrapper refuses is explainable; the diagnostic
    // separates "unwrappable by design" from a wrapper bug.
    let (output, meta) = layout::lay_out(document, config)?;
    Ok((
        output,
        FormatMeta {
            last_indent: meta.last_indent,
            last_usable: meta.last_usable,
            declines: declined,
        },
    ))
}

/// Step 16: reflow statements that overrun the budget.
///
/// The first-line indent and the continuation column both come from the layout
/// plan, so a user who changes `-k` or turns on `--align-paren` changes where
/// wrapped lines start *and* the width the wrapper had to work with, together.
/// A literal `indent + 4` here would silently disagree with the engine the
/// moment either option moved.
pub fn reflow(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<Vec<(usize, Decline)>, FormatError> {
    let local = analyze_file(&document.to_lf_bytes())?;
    reflow_with_context(document, &ProjectContext::empty(), &local, config)
}

pub fn reflow_with_context(
    document: &mut Document,
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<Vec<(usize, Decline)>, FormatError> {
    reflow_with_context_inner(document, project, local, config)
}

fn reflow_with_context_inner(
    document: &mut Document,
    project: &ProjectContext,
    local: &crate::analysis::FileFacts,
    config: &FormatConfig,
) -> Result<ReflowResult, FormatError> {
    let analysis = document.analyze()?;
    let scopes = ScopeTree::build(&analysis);
    let declared_names = scoped_declared_names(&analysis, &scopes);
    let (lines, declined) = {
        let scope = ReflowScope::build(
            document,
            pipeline::PassContext {
                config,
                project,
                local,
                analysis: &analysis,
                scopes: &scopes,
            },
            &declared_names,
        )?;
        scope.reflow()?
    };
    document.set_lines(lines);
    Ok(declined)
}

/// Everything a group's emission reads that is the same in every round.
///
/// The wrapper settles once which groups are wrappable, which take the sentinel
/// path, and what the authored document measures when laid out. The round loop
/// can then focus only on the measurement that genuinely advances.
struct ReflowScope<'a> {
    document: &'a Document,
    cx: pipeline::PassContext<'a>,
    declared_names: &'a crate::analysis::DeclaredNameIndex,
    plans: Vec<GroupPlan>,
    /// Which groups the round loop is even allowed to flag: the rest reach
    /// emission only to be copied, so flagging them would claim progress that
    /// no round can act on.
    wrappable: Vec<bool>,
    /// The authored document laid out, kept for the sentinel path, which asks
    /// about authored lines rather than the advancing round measurement.
    unwrapped: Option<Document>,
}

impl<'a> ReflowScope<'a> {
    fn build(
        document: &'a Document,
        cx: pipeline::PassContext<'a>,
        declared_names: &'a crate::analysis::DeclaredNameIndex,
    ) -> Result<Self, FormatError> {
        let analysis = cx.analysis;
        let config = cx.config;
        let mut planner = Planner::new(config);
        let plans: Vec<GroupPlan> = analysis
            .groups
            .iter()
            .map(|group| planner.plan(&analysis.buffer, group, config))
            .collect();
        // Which groups take the directive path rather than the statement path.
        // Asked before `unwrapped` is built, because it also determines whether
        // that authored measurement is needed at all.
        let sentinel: Vec<bool> = analysis
            .groups
            .iter()
            .map(|group| sentinel::prepare(document, &analysis.buffer, group).is_some())
            .collect();
        let unwrapped = sentinel
            .iter()
            .any(|sentinel| *sentinel)
            .then(|| layout::measure(&document.to_lf_bytes(), config))
            .transpose()?;
        let wrappable: Vec<bool> = analysis
            .groups
            .iter()
            .zip(&plans)
            .zip(&sentinel)
            .map(|((group, plan), sentinel)| {
                // A group `--refactor-end` rewrites is never wrapped. The engine
                // emits the replacement in place of the first physical line's
                // body, so a wrapper break would be discarded while its
                // continuation survived as invalid source.
                matches!(
                    plan.body,
                    PlanBody::Code {
                        replacement: None,
                        ..
                    }
                ) && !sentinel
                    && eligible(&analysis.buffer, group)
                    && group.statements.len() == 1
            })
            .collect();
        Ok(Self {
            document,
            cx,
            declared_names,
            plans,
            wrappable,
            unwrapped,
        })
    }

    fn analysis(&self) -> &'a crate::transform::document::Analysis {
        self.cx.analysis
    }

    fn config(&self) -> &'a FormatConfig {
        self.cx.config
    }

    /// The width this line had in the authored document laid out.
    ///
    /// Only the sentinel path reads it, and `unwrapped` is `Some` exactly when
    /// that path is reachable.
    fn unwrapped_width(&self, line: usize) -> usize {
        self.unwrapped
            .as_ref()
            .map_or(0, |unwrapped| unwrapped.lines.get(line).map_or(0, Vec::len))
    }

    /// One group's emitted lines, plus the decline to report for it.
    fn emit_group(
        &self,
        group: &LogicalGroup,
        plan: &GroupPlan,
        span: &std::ops::Range<usize>,
        laid_out: &Document,
        needs_reflow: bool,
    ) -> (Vec<Vec<u8>>, Option<(usize, Decline)>) {
        match sentinel::prepare(self.document, &self.analysis().buffer, group) {
            Some(directive) => self.emit_sentinel_group(group, plan, directive),
            None => self.emit_statement_group(group, plan, span, laid_out, needs_reflow),
        }
    }

    /// The directive path: a whole-line OpenMP directive, wrapped by repeating
    /// its sentinel rather than by Fortran's `&` statement continuation.
    fn emit_sentinel_group(
        &self,
        group: &LogicalGroup,
        plan: &GroupPlan,
        directive: Vec<u8>,
    ) -> (Vec<Vec<u8>>, Option<(usize, Decline)>) {
        let mut out: Vec<Vec<u8>> = Vec::new();
        let directive = sentinel::reindent(
            &directive,
            match plan.body {
                PlanBody::Uniform { indent } => indent,
                PlanBody::Code { first_indent, .. } => first_indent,
            },
        );
        let mut decline = None;
        let budget = self.config().wrap.line_length;
        let long = directive.len() > budget
            || group
                .lines
                .clone()
                .any(|index| self.document.lines[index].len() > self.config().wrap.line_length)
            || group
                .lines
                .clone()
                .any(|index| self.unwrapped_width(index) > self.config().wrap.line_length);
        if long {
            match sentinel::wrap_line(&directive, budget) {
                Ok(wrapped) => out.extend(wrapped),
                Err(reason) => {
                    decline = Some((group.lines.start, reason));
                    copy_group(self.document, group, &mut out);
                }
            }
        } else {
            out.push(self.document.lines[group.lines.start].clone());
        }
        if group.lines.len() > 1 {
            for index in group.lines.clone().skip(1) {
                out.push(self.document.lines[index].clone());
            }
        }
        (out, decline)
    }

    /// The statement path: one Fortran statement, joined if the author split it,
    /// then broken at the columns the layout engine is about to place it on.
    fn emit_statement_group(
        &self,
        group: &LogicalGroup,
        plan: &GroupPlan,
        span: &std::ops::Range<usize>,
        laid_out: &Document,
        needs_reflow: bool,
    ) -> (Vec<Vec<u8>>, Option<(usize, Decline)>) {
        let mut out: Vec<Vec<u8>> = Vec::new();
        let Some(geometry) = self.statement_geometry(group, plan) else {
            copy_group(self.document, group, &mut out);
            return (out, None);
        };
        let StatementGeometry {
            index,
            first_indent,
            emitted_target,
            sentinel_width,
            continuation,
            conditional,
            align,
        } = geometry;

        let mut body = self.statement_body(group, index);
        let label = StatementLabel::split(&mut body, sentinel_width, emitted_target, self.config());
        let layout = ContinuationLayout {
            first_indent: label.first_body_column,
            continuation,
        };
        // The gate is asked of the lines the formatter is going to emit. The
        // sticky flag means a group that has been wrapped keeps its wrap instead
        // of measuring its own short lines and unwrapping itself.
        body = body_as_emitted(body, plan.remred, needs_reflow, self.config());
        if !needs_reflow {
            copy_group(self.document, group, &mut out);
            return (out, None);
        }
        // Step 17 may compress an authored declaration alignment run. Measure
        // and break the emitted spelling, not the authored spacing.
        body = with_laid_out_separator(body, laid_out_separator_line(laid_out, span));
        let detached = detach_final_inline_comment(
            self.document,
            &self.analysis().buffer,
            group,
            first_indent,
        );
        // Pay for spaces step 17 may still insert around `::` from the same
        // budget the wrapper uses.
        let budget = self
            .config()
            .wrap
            .line_length
            .saturating_sub(declaration_separator_growth(&body));
        if label.first_body_column + body.len() <= budget {
            self.emit_unbroken_statement(
                group,
                label.prepend(body),
                detached,
                first_indent,
                conditional,
                &mut out,
            );
            return (out, None);
        }
        if detached.is_none() {
            copy_group(self.document, group, &mut out);
            return (out, None);
        }
        let mut decline = None;
        match wrapping::wrap_body_with_alignment(&body, layout, budget, align) {
            Ok(mut wrapped) => {
                if let Some(comment) = detached.flatten() {
                    out.extend(comment);
                }
                if let Some(first) = wrapped.first_mut() {
                    *first = label.prepend(std::mem::take(first));
                }
                out.extend(
                    wrapped
                        .into_iter()
                        .map(|line| restore_conditional_prefix(line, conditional)),
                );
            }
            // A decline means the statement stays exactly as authored. It has
            // to be copied whole, including every continuation line.
            Err(Decline::Fits) => copy_group(self.document, group, &mut out),
            Err(reason) => {
                decline = Some((index, reason));
                copy_group(self.document, group, &mut out);
            }
        }
        (out, decline)
    }

    /// The columns one statement is measured and wrapped against, or `None`
    /// when this group is not a single wrappable statement at all.
    fn statement_geometry(
        &self,
        group: &LogicalGroup,
        plan: &GroupPlan,
    ) -> Option<StatementGeometry> {
        let PlanBody::Code {
            first_indent,
            align,
            ..
        } = plan.body
        else {
            return None;
        };
        if !eligible(&self.analysis().buffer, group) || group.statements.len() != 1 {
            return None;
        }
        let index = group.lines.start;
        let conditional = self
            .analysis()
            .buffer
            .lines
            .get(index)
            .is_some_and(|line| line.is_conditional_compilation() && self.config().openmp);
        // A conditional sentinel is written by the emitter, not by the wrapper,
        // so it is charged to the line but not to the body.
        let sentinel_width = if conditional {
            crate::format::engine::CONDITIONAL_SENTINEL_COLUMNS
        } else {
            0
        };
        let emitted_target = first_indent.saturating_sub(sentinel_width);
        Some(StatementGeometry {
            index,
            first_indent,
            emitted_target,
            sentinel_width,
            continuation: sentinel_width
                + emitted_target.saturating_add(if self.config().indent_continuation {
                    self.config().continuation_indent
                } else {
                    0
                }),
            conditional,
            align,
        })
    }

    /// The statement's text as one line, rejoined and respaced when the author
    /// split it.
    fn statement_body(&self, group: &LogicalGroup, index: usize) -> Vec<u8> {
        let mut body = trim(&group.statements[0].text).to_vec();
        if group.lines.len() > 1 {
            let original_body = body.clone();
            body = crate::transform::passes::line_rules::respace_joined(
                &body,
                &self.cx,
                self.declared_names,
                index,
            );
            body = crate::transform::passes::case_pass::restore_declined_component_spellings(
                &original_body,
                &body,
                index,
                self.declared_names,
                &self.cx,
            );
            body = trim(&body).to_vec();
        }
        body
    }

    /// A statement that fits: it is still emitted as one line when the author
    /// had split it, and otherwise copied exactly as authored.
    fn emit_unbroken_statement(
        &self,
        group: &LogicalGroup,
        body: Vec<u8>,
        detached: Option<Option<Vec<Vec<u8>>>>,
        first_indent: usize,
        conditional: bool,
        out: &mut Vec<Vec<u8>>,
    ) {
        match detached {
            Some(Some(comment)) => {
                out.extend(comment);
                if group.lines.len() > 1 {
                    emit_joined_body(out, &body, first_indent, conditional);
                } else {
                    copy_group_without_final_comment(
                        self.document,
                        &self.analysis().buffer,
                        group,
                        out,
                    );
                }
            }
            Some(None) if group.lines.len() > 1 => {
                emit_joined_body(out, &body, first_indent, conditional);
            }
            _ => copy_group(self.document, group, out),
        }
    }
}

/// The columns one statement is wrapped against.
struct StatementGeometry {
    /// The group's first physical line.
    index: usize,
    /// Indentation the layout engine will give the statement.
    first_indent: usize,
    /// `first_indent` less the sentinel the emitter writes itself.
    emitted_target: usize,
    sentinel_width: usize,
    /// Column a continuation line's body starts on.
    continuation: usize,
    conditional: bool,
    /// Parenthesis alignment is active for this statement.
    align: bool,
}

/// A statement label split off from its body.
struct StatementLabel {
    digits: Option<Vec<u8>>,
    /// The column the statement's own text starts on, label or no label.
    first_body_column: usize,
}

impl StatementLabel {
    fn split(
        body: &mut Vec<u8>,
        sentinel_width: usize,
        emitted_target: usize,
        config: &FormatConfig,
    ) -> Self {
        match crate::format::emitter::split_label(body) {
            Some((label, rest)) => {
                let digits = label.to_vec();
                let first_body_column = sentinel_width
                    + crate::format::emitter::labelled_body_column(
                        emitted_target,
                        digits.len(),
                        config,
                    );
                *body = rest.to_vec();
                Self {
                    digits: Some(digits),
                    first_body_column,
                }
            }
            None => Self {
                digits: None,
                first_body_column: sentinel_width + emitted_target,
            },
        }
    }

    /// Give the label back, with the single space that keeps it a label; the
    /// engine still owns the gap it is finally written with.
    fn prepend(&self, line: Vec<u8>) -> Vec<u8> {
        match &self.digits {
            Some(label) => {
                let mut out = Vec::with_capacity(label.len() + 1 + line.len());
                out.extend_from_slice(label);
                out.push(b' ');
                out.extend_from_slice(&line);
                out
            }
            None => line,
        }
    }
}

/// Find an inline comment in one physical line while keeping conditional and
/// ordinary protected regions independent. The returned offset is in the
/// original physical line, not the sentinel-stripped body.
fn stream_comment_start(
    streams: &mut StreamLexStates,
    line: &[u8],
    stream: SourceStream,
) -> Option<usize> {
    let prefix = stream
        .is_conditional()
        .then(|| conditional_compilation_prefix(line))
        .flatten();
    let body_start = prefix.map_or(0, |prefix| prefix.body_start);
    let body = &line[body_start..];
    let lex = streams.select_mut(stream);
    if !crate::source::regions::resumes_protected_region(lex, body) {
        return None;
    }
    crate::source::regions::line_comment_start(lex, body).map(|start| body_start + start)
}

/// Return the final inline comment, if there is exactly one and it is on the
/// last physical line of the group. `None` means the group is unsafe to detach;
/// `Some(None)` means it has no inline comment.
fn detach_final_inline_comment<B: AsRef<[u8]>>(
    document: &Document,
    buffer: &SourceBuffer<B>,
    group: &LogicalGroup,
    comment_indent: usize,
) -> Option<Option<Vec<Vec<u8>>>> {
    let mut streams = StreamLexStates::default();
    let mut comments = Vec::new();
    for index in group.lines.clone() {
        let line = &document.lines[index];
        if let Some(start) = stream_comment_start(&mut streams, line, buffer.stream(index)) {
            comments.push((index, start));
        }
    }
    if comments.is_empty() {
        return Some(None);
    }
    if comments.len() != 1 || comments[0].0 + 1 != group.lines.end {
        return None;
    }
    let (index, start) = comments[0];
    let line = &document.lines[index];
    let text = line[start..].trim_ascii_start();
    // Sentinel syntax is anchored to the start of a physical line. Lifting a
    // trailing directive-like comment would manufacture or retarget a real
    // directive, so such comments remain inline.
    if is_directive_comment(text) {
        return None;
    }
    let mut comment = vec![b' '; comment_indent];
    comment.extend_from_slice(text);
    Some(Some(vec![comment]))
}

fn emit_joined_body(lines: &mut Vec<Vec<u8>>, body: &[u8], first_indent: usize, conditional: bool) {
    let mut line = vec![b' '; first_indent];
    line.extend_from_slice(body);
    lines.push(restore_conditional_prefix(line, conditional));
}

/// Restore the canonical conditional-compilation sentinel to a line generated
/// by the wrapper. `line` already carries the absolute column where its Fortran
/// body belongs, so the sentinel replaces three leading columns instead of
/// shifting the body to the right.
fn restore_conditional_prefix(line: Vec<u8>, conditional: bool) -> Vec<u8> {
    if !conditional {
        return line;
    }
    let leading = line
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    let mut restored = Vec::with_capacity(line.len() + 3);
    restored.extend_from_slice(b"!$ ");
    restored.extend(std::iter::repeat_n(b' ', leading.saturating_sub(3)));
    restored.extend_from_slice(&line[leading..]);
    restored
}

/// The laid-out physical line carrying this statement's declaration separator.
fn laid_out_separator_line<'a>(
    laid_out: &'a Document,
    span: &std::ops::Range<usize>,
) -> Option<&'a Vec<u8>> {
    span.clone()
        .filter_map(|line| laid_out.lines.get(line))
        .find(|line| {
            crate::transform::passes::layout_post::declaration_separator_info(line).is_some()
        })
}

/// Rewrite whitespace around `body`'s `::` to the runs final alignment already
/// chose for this line, so wrapping measures emitted spelling.
fn with_laid_out_separator(body: Vec<u8>, laid_out: Option<&Vec<u8>>) -> Vec<u8> {
    let Some(laid_out) = laid_out else {
        return body;
    };
    let (Some((at, run, after)), Some((_, laid_out_run, laid_out_after))) = (
        crate::transform::passes::layout_post::declaration_separator_info(&body),
        crate::transform::passes::layout_post::declaration_separator_info(laid_out),
    ) else {
        return body;
    };
    // A missing space on either side is `declaration_separator_growth`'s
    // business, not this function's.
    let before_run = if run == 0 || laid_out_run == 0 {
        run
    } else {
        laid_out_run
    };
    let after_run = if after == 0 || laid_out_after == 0 {
        after
    } else {
        laid_out_after
    };
    if before_run == run && after_run == after {
        return body;
    }
    let mut result = Vec::with_capacity(body.len() + before_run + after_run);
    result.extend_from_slice(&body[..at - run]);
    result.resize(result.len() + before_run, b' ');
    result.extend_from_slice(b"::");
    result.resize(result.len() + after_run, b' ');
    result.extend_from_slice(&body[at + 2 + after..]);
    result
}

/// How many bytes step 17 will insert around this statement's `::` spellings.
fn declaration_separator_growth(body: &[u8]) -> usize {
    let mut quote = 0u8;
    let mut index = 0;
    let mut growth = 0;
    while index < body.len() {
        let byte = body[index];
        if quote != 0 {
            if byte == quote {
                if body.get(index + 1) == Some(&quote) {
                    index += 2;
                    continue;
                }
                quote = 0;
            }
            index += 1;
        } else if byte == b'\'' || byte == b'"' {
            quote = byte;
            index += 1;
        } else if byte == b'!' {
            break;
        } else if body.get(index..index + 2) == Some(b"::") {
            let before = usize::from(index == 0 || !matches!(body[index - 1], b' ' | b'\t'));
            let after = usize::from(!matches!(body.get(index + 2), Some(b' ' | b'\t')));
            growth += before + after;
            index += 2;
        } else {
            index += 1;
        }
    }
    growth
}

/// Project the statement text onto the whitespace the layout emitter will write
/// before the wrapper measures it.
fn body_as_emitted(
    mut body: Vec<u8>,
    remred: bool,
    project: bool,
    config: &FormatConfig,
) -> Vec<u8> {
    if project && remred {
        let mut reduced = Vec::with_capacity(body.len());
        let mut lex = crate::source::LexState::default();
        crate::transform::whitespace::reduce_line_into_protected(
            &body,
            &mut lex,
            config.mode.aligns_after_layout() && config.align_declarations,
            config.mode.aligns_after_layout() && config.align_comments,
            &mut |byte| reduced.push(byte),
        );
        body = reduced;
    }
    body
}

fn copy_group(document: &Document, group: &LogicalGroup, lines: &mut Vec<Vec<u8>>) {
    for index in group.lines.clone() {
        if let Some(line) = document.lines.get(index) {
            lines.push(line.clone());
        }
    }
}

pub(super) fn copy_group_without_final_comment<B: AsRef<[u8]>>(
    document: &Document,
    buffer: &SourceBuffer<B>,
    group: &LogicalGroup,
    lines: &mut Vec<Vec<u8>>,
) {
    let final_line = group.lines.end.saturating_sub(1);
    let mut streams = StreamLexStates::default();
    for index in group.lines.clone() {
        let Some(line) = document.lines.get(index) else {
            continue;
        };
        let comment = stream_comment_start(&mut streams, line, buffer.stream(index));
        if index == final_line {
            if let Some(comment) = comment {
                lines.push(line[..comment].trim_ascii_end().to_vec());
                continue;
            }
        }
        lines.push(line.clone());
    }
}

/// Reflow is declined when the group interleaves anything that cannot sit
/// between a continuation marker and the text it continues (I5).
fn eligible(buffer: &SourceBuffer, group: &LogicalGroup) -> bool {
    group.lines.clone().all(|index| {
        buffer.lines.get(index).is_some_and(|line| {
            matches!(
                line.kind,
                PhysicalLineKind::Code | PhysicalLineKind::FindentFix
            )
        })
    })
}

fn trim(line: &[u8]) -> &[u8] {
    let mut s = line;
    while s.first().is_some_and(u8::is_ascii_whitespace) {
        s = &s[1..];
    }
    while s.last().is_some_and(u8::is_ascii_whitespace) {
        s = &s[..s.len() - 1];
    }
    s
}
