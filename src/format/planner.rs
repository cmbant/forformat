//! Structural planning, separated from emission.
//!
//! The engine used to compute indentation state and write bytes inside one
//! closure, which meant nothing could look ahead: the first byte of a statement
//! was already on its way to the sink before the statement's own indentation
//! was known.  Wrapping needs the opposite order — the first-line indent and the
//! continuation policy have to be known *before* break points are chosen.
//!
//! So planning answers "where does this group go?" and emission answers "what
//! bytes does it produce?".  The indent stack is touched only here.

use super::{
    preprocessor::{event, PreprocessorState},
    stack::{clamp, IndentStack},
};
use crate::{
    classify::{classify, StatementClass, StatementInfo, StatementKind},
    config::FormatConfig,
    source::{LogicalGroup, PhysicalLineKind, SourceBuffer},
};
use std::ops::Range;

/// How one logical group is laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanBody {
    /// Every physical line is placed at one fixed indent and treated as a first
    /// line: CPP directive groups, blank and comment groups, and OpenMP
    /// directives while OpenMP indentation is off.
    Uniform { indent: usize },
    /// A Fortran statement group, possibly continued over several physical
    /// lines.
    Code {
        /// Indentation of the group's first physical line.
        first_indent: usize,
        /// Indentation used for a CPP directive interleaved in the group, which
        /// follows the structural state *after* the group's transitions.
        directive_indent: usize,
        /// The first physical line ends with `&`.
        group_first_cont: bool,
        /// Parenthesis alignment is active for this group.
        align: bool,
        /// A complete replacement for the first line's body, produced by
        /// `--refactor-end`.
        replacement: Option<Vec<u8>>,
    },
}

/// The layout decision for one logical group, plus the query values it
/// contributes.  A `DocumentLayout` is a `Vec<GroupPlan>`; the streaming engine
/// consumes one at a time, and the wrapper consumes them in bulk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPlan {
    pub lines: Range<usize>,
    pub body: PlanBody,
    /// CPP directive groups keep their source spelling and never receive
    /// structural indentation.
    pub apply_indent: bool,
    /// Redundant-whitespace reduction, off for Hollerith-bearing groups.
    pub remred: bool,
    /// New value for the `-lastindent` query, when this group sets one.
    pub last_indent: Option<usize>,
    /// New value for the `-lastusable` query, when this group sets one.
    pub last_usable: Option<usize>,
}

/// Mutable state carried from group to group while planning a document.
#[derive(Debug)]
pub struct Planner {
    pub stack: IndentStack,
    pub preprocessor: PreprocessorState,
    pub significant_seen: bool,
}

impl Planner {
    pub fn new(config: &FormatConfig) -> Self {
        Self {
            stack: IndentStack::new(config.start_indent),
            preprocessor: PreprocessorState::default(),
            significant_seen: false,
        }
    }

    /// Plan one logical group, advancing the structural state.
    pub fn plan(
        &mut self,
        buf: &SourceBuffer,
        group: &LogicalGroup,
        config: &FormatConfig,
    ) -> GroupPlan {
        let line0 = &buf.lines[group.lines.start];
        if config.auto_start_indent && !self.significant_seen && !group.statements.is_empty() {
            let guessed = guess_start_indent(buf.line_bytes(line0));
            self.stack.set_base(guessed);
        }

        if line0.kind == PhysicalLineKind::Preprocessor {
            let ev = event(buf.line_bytes(line0));
            // The directive's own line is placed at the indentation in force
            // before the event, including `#endif`, which is why the column is
            // captured first and the branch state advanced after.
            let indent = self.stack.current();
            self.preprocessor.apply(ev, &mut self.stack);
            return GroupPlan {
                lines: group.lines.clone(),
                body: PlanBody::Uniform { indent },
                apply_indent: false,
                remred: remred(config),
                last_indent: Some(indent),
                last_usable: None,
            };
        }

        if line0.omp && !config.openmp {
            return GroupPlan {
                lines: group.lines.clone(),
                body: PlanBody::Uniform {
                    indent: self.stack.current(),
                },
                apply_indent: config.apply_indent,
                remred: remred(config),
                last_indent: None,
                last_usable: None,
            };
        }

        let infos: Vec<StatementInfo> = group
            .statements
            .iter()
            .map(|statement| classify(&statement.text))
            .collect();

        if infos.is_empty() {
            // A group with no statement carries only a comment, and a comment
            // belongs at the depth in force around it.  Besides an ordinary
            // comment line, that includes an OpenMP sentinel whose entire
            // payload is a comment (`!$    ! merge splitted arrays`): the
            // buffer classifies it as Code because of the sentinel, so it
            // would otherwise fall to column zero while the code on either
            // side of it inside the `!$` block stays indented.
            let comment_only = line0.kind == PhysicalLineKind::Comment
                || line0.omp && sentinel_payload_is_blank(buf.code_bytes(line0));
            let indent = if comment_only {
                self.stack.current()
            } else {
                0
            };
            return GroupPlan {
                lines: group.lines.clone(),
                body: PlanBody::Uniform { indent },
                apply_indent: config.apply_indent,
                remred: remred(config),
                last_indent: None,
                last_usable: None,
            };
        }

        let hollerith = infos.iter().any(|info| info.contains_hollerith);
        let mut first_indent = self.stack.current();
        let mut replacement = None;
        let mut last_usable = None;
        for (position, info) in infos.iter().enumerate() {
            if info.kind != StatementKind::EndDo {
                if let Some(label) = info.statement_label {
                    self.stack.close_label(label);
                }
            }
            if position == 0 && config.refactor_end && info.class == StatementClass::EndDefinition {
                replacement = self.refactor_end_text(buf, group, info, config);
            }
            let statement_indent = transition(&mut self.stack, info, config);
            if position == 0 {
                // A physical line containing multiple semicolon-separated
                // statements is emitted once, at the indentation of its
                // first statement.  Later statements still update state for
                // the following physical line.
                first_indent = statement_indent;
            }
            if info.class != StatementClass::Neutral || info.kind != StatementKind::Unknown {
                self.significant_seen = true;
                last_usable = Some(group.lines.end);
            }
        }

        GroupPlan {
            lines: group.lines.clone(),
            body: PlanBody::Code {
                first_indent,
                directive_indent: self.stack.current(),
                group_first_cont: super::continuation::trailing_ampersand(buf.code_bytes(line0)),
                align: !hollerith && (config.align_paren || config.align_paren_value != 0),
                replacement,
            },
            apply_indent: config.apply_indent,
            remred: remred(config) && !hollerith,
            last_indent: Some(clamp(first_indent, config.max_indent)),
            last_usable,
        }
    }

    fn refactor_end_text(
        &self,
        buf: &SourceBuffer,
        group: &LogicalGroup,
        info: &StatementInfo,
        config: &FormatConfig,
    ) -> Option<Vec<u8>> {
        let frame = self
            .stack
            .frames
            .last()
            .filter(|frame| frame.kind != StatementKind::AbstractInterface)?;
        let mut replacement = crate::transform::refactor_end::end_text(
            frame.kind,
            frame.name.as_deref(),
            config.uppercase_end,
        );
        if let Some(paren) = info.payload.iter().position(|byte| *byte == b'(') {
            replacement.extend_from_slice(&info.payload[paren..]);
        }
        if let Some(comment) = &buf.lines[group.lines.start].comment_span {
            replacement.push(b' ');
            replacement.extend_from_slice(&buf.bytes[comment.start as usize..comment.end as usize]);
        }
        Some(replacement)
    }
}

fn remred(config: &FormatConfig) -> bool {
    config.ws_remred || config.ws_remred_value != 0
}

/// Advance the indent stack for one statement and return the column its line
/// starts at.  This is findent's structural core and is shared by every mode.
pub fn transition(state: &mut IndentStack, info: &StatementInfo, cfg: &FormatConfig) -> usize {
    let line_indent;
    let k = info.kind;
    let ci = &cfg.construct_indents;
    match k {
        StatementKind::Program
        | StatementKind::Module
        | StatementKind::Submodule
        | StatementKind::Subroutine
        | StatementKind::Function
        | StatementKind::BlockData
        | StatementKind::Interface
        | StatementKind::AbstractInterface
        | StatementKind::Type
        | StatementKind::Structure
        | StatementKind::Union
        | StatementKind::Map
        | StatementKind::Procedure => {
            line_indent = state.raw_current();
            // A MODULE PROCEDURE inside an INTERFACE is an interface member,
            // not a nested procedure body.  In a SUBMODULE/CONTAINS region it
            // is a real procedure definition and receives a frame.
            if k == StatementKind::Procedure
                && state.frames.last().is_some_and(|frame| {
                    matches!(
                        frame.kind,
                        StatementKind::Interface
                            | StatementKind::AbstractInterface
                            | StatementKind::Type
                    )
                })
            {
                return line_indent;
            }
            let amount = match k {
                StatementKind::Program
                | StatementKind::Subroutine
                | StatementKind::Function
                | StatementKind::BlockData => ci.procedure,
                StatementKind::Module | StatementKind::Submodule => ci.module,
                StatementKind::Interface | StatementKind::AbstractInterface => ci.interface,
                StatementKind::Type => ci.r#type,
                StatementKind::Procedure => ci.procedure,
                StatementKind::Structure | StatementKind::Union | StatementKind::Map => ci.r#type,
                _ => ci.procedure,
            };
            state.push(k, amount, info.entity_name.clone(), cfg.max_indent);
        }
        StatementKind::If => {
            line_indent = state.raw_current();
            state.push(k, ci.if_, None, cfg.max_indent);
        }
        StatementKind::Do => {
            line_indent = state.raw_current();
            state.push(k, ci.do_, None, cfg.max_indent);
            if let Some(label) = info.referenced_labels.first() {
                state.label_do(*label);
            }
        }
        StatementKind::Select => {
            line_indent = state.raw_current();
            state.push(k, ci.select, None, cfg.max_indent);
        }
        StatementKind::Where => {
            line_indent = state.raw_current();
            state.push(k, ci.where_, None, cfg.max_indent);
        }
        StatementKind::Forall => {
            line_indent = state.raw_current();
            state.push(k, ci.forall, None, cfg.max_indent);
        }
        StatementKind::Associate => {
            line_indent = state.raw_current();
            state.push(k, ci.associate, None, cfg.max_indent);
        }
        StatementKind::Block => {
            line_indent = state.raw_current();
            state.push(k, ci.block, None, cfg.max_indent);
        }
        StatementKind::Critical => {
            line_indent = state.raw_current();
            state.push(k, ci.critical, None, cfg.max_indent);
        }
        StatementKind::ChangeTeam => {
            line_indent = state.raw_current();
            state.push(k, ci.changeteam, None, cfg.max_indent);
        }
        StatementKind::Enum => {
            line_indent = state.raw_current();
            state.push(k, ci.r#enum, None, cfg.max_indent);
        }
        StatementKind::Else | StatementKind::ElseIf => {
            line_indent = state.raw_current().saturating_sub(ci.if_);
        }
        StatementKind::ElseWhere => {
            line_indent = state.raw_current().saturating_sub(ci.where_);
        }
        StatementKind::Case => {
            line_indent = state.raw_current().saturating_sub(cfg.case_indent);
        }
        StatementKind::Entry => {
            line_indent = state.raw_current().saturating_sub(cfg.entry_indent);
        }
        StatementKind::Contains => {
            if cfg.contains_restart {
                line_indent = state.base;
                state.restart_at_base(cfg.max_indent);
            } else {
                // `max_indent` limits emitted columns, not structural depth.
                // Keep CONTAINS at the same logical level even when the
                // procedure/module body was clamped before this point.
                line_indent = state.raw_current().saturating_sub(cfg.contains_indent);
            }
        }
        StatementKind::EndIf => {
            line_indent = state
                .pop_kind(StatementKind::If)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndDo => {
            line_indent = state
                .pop_kind(StatementKind::Do)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndSelect => {
            line_indent = state
                .pop_kind(StatementKind::Select)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndWhere => {
            line_indent = state
                .pop_kind(StatementKind::Where)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndForall => {
            line_indent = state
                .pop_kind(StatementKind::Forall)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndAssociate => {
            line_indent = state
                .pop_kind(StatementKind::Associate)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndBlock => {
            line_indent = state
                .pop_kind(StatementKind::Block)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndCritical => {
            line_indent = state
                .pop_kind(StatementKind::Critical)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndTeam => {
            line_indent = state
                .pop_kind(StatementKind::ChangeTeam)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndEnum => {
            line_indent = state
                .pop_kind(StatementKind::Enum)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndProcedure => {
            line_indent = state
                .pop_kind(StatementKind::Procedure)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndStructure => {
            line_indent = state
                .pop_kind(StatementKind::Structure)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndUnion => {
            line_indent = state
                .pop_kind(StatementKind::Union)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::EndMap => {
            line_indent = state
                .pop_kind(StatementKind::Map)
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::Unknown if info.class == StatementClass::EndDefinition => {
            line_indent = state
                .pop_definition()
                .or_else(|| {
                    info.end_kind.and_then(|kind| {
                        state.pop_orphan_procedure(kind, cfg.indent, cfg.max_indent)
                    })
                })
                .or_else(|| state.recover_definition_end(cfg.indent, cfg.max_indent))
                .or_else(|| {
                    // findent's shallow END recognizer still backs out one
                    // default indentation level for an explicit END whose
                    // target is not the active definition (for example
                    // `END SUBROUTINE` inside an IF).  Preserve the frame so
                    // its real END can close it later, but keep the malformed
                    // line and following siblings at the recovered branch.
                    (!state.frames.is_empty()).then(|| {
                        let raw = state.raw_current().saturating_sub(cfg.indent);
                        state.branch(cfg.indent);
                        raw
                    })
                })
                .unwrap_or_else(|| state.raw_current());
        }
        StatementKind::Include => {
            line_indent = if cfg.include_left {
                state.base
            } else {
                state.raw_current()
            };
        }
        StatementKind::LabelContinue => {
            if let Some(l) = info.statement_label {
                state.close_label(l)
            }
            line_indent = state.raw_current();
        }
        _ => {
            line_indent = state.raw_current();
            if let Some(kind) = info.unframed_procedure {
                state.mark_orphan_procedure(kind);
            }
        }
    }
    line_indent
}

/// True when an OpenMP sentinel line's code bytes hold nothing after the `!$ `
/// marker, which is what a sentinel line carrying only a comment looks like.
fn sentinel_payload_is_blank(code: &[u8]) -> bool {
    let code = code.trim_ascii_start();
    let payload = code.strip_prefix(b"!$ ".as_slice()).unwrap_or(code);
    payload.iter().all(u8::is_ascii_whitespace)
}

pub fn guess_start_indent(line: &[u8]) -> usize {
    let mut columns = 0;
    for c in line {
        match c {
            b' ' => columns += 1,
            b'\t' => columns = (columns / 8 + 1) * 8,
            _ => break,
        }
    }
    columns
}
