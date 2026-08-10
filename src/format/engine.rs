use super::{
    continuation::{trailing_ampersand, ParenAlignmentState},
    emitter::emit_line_to_with_quote,
    preprocessor::{event, PreprocessorEvent, PreprocessorState},
    stack::{clamp, IndentStack},
};
use crate::{
    classify::{classify, StatementClass, StatementInfo, StatementKind},
    config::FormatConfig,
    error::FormatError,
    source::{LogicalGroup, PhysicalLineKind, SourceBuffer},
    FormatMeta, FormatResult,
};
use std::io::Write;

struct QueryWriter<'a, W: Write> {
    inner: &'a mut W,
    discard: bool,
}

impl<W: Write> Write for QueryWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.discard {
            Ok(bytes.len())
        } else {
            self.inner.write(bytes)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.discard {
            Ok(())
        } else {
            self.inner.flush()
        }
    }
}

pub fn format(source: &[u8], config: &FormatConfig) -> Result<FormatResult, FormatError> {
    let mut output = Vec::with_capacity(source.len() + 64);
    let meta = format_to(source, config, &mut output)?;
    Ok(FormatResult {
        bytes: output,
        meta,
    })
}

pub fn format_to<W: Write>(
    source: &[u8],
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    let buf = SourceBuffer::new(source)?;
    format_buffer(&buf, config, out)
}

pub fn format_to_owned<W: Write>(
    source: Vec<u8>,
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    let buf = SourceBuffer::from_vec(source)?;
    format_buffer(&buf, config, out)
}

fn format_buffer<W: Write>(
    buf: &SourceBuffer,
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    if buf.bytes.is_empty() {
        if config.last_usable {
            writeln!(out, "1").map_err(FormatError::Write)?;
        } else if config.last_indent {
            writeln!(out, "0").map_err(FormatError::Write)?;
        }
        return Ok(FormatMeta {
            last_indent: 0,
            last_usable: 1,
        });
    }
    let query_mode = config.last_indent || config.last_usable;
    let mut output = QueryWriter {
        inner: out,
        discard: query_mode,
    };
    let mut state = IndentStack::new(config.start_indent);
    let mut pre = PreprocessorState::default();
    let mut last_indent = 0;
    let mut last_usable = 1;
    let mut significant_seen = false;
    LogicalGroup::visit(buf, |group| {
        let first_line = group.lines.start;
        let line0 = &buf.lines[first_line];
        if config.auto_start_indent && !significant_seen && !group.statements.is_empty() {
            let guessed = guess_start_indent(buf.line_bytes(line0));
            state.set_base(guessed);
        }
        if line0.kind == PhysicalLineKind::Preprocessor {
            let ev = event(buf.line_bytes(line0));
            let indent = state.current();
            let mut pre_config = config.clone();
            pre_config.apply_indent = false;
            if ev != PreprocessorEvent::EndIf {
                pre.apply(ev, &mut state);
            }
            let mut quote = 0u8;
            for i in group.lines.clone() {
                emit_line_to_with_quote(
                    buf,
                    i,
                    indent,
                    &pre_config,
                    true,
                    false,
                    None,
                    None,
                    &mut quote,
                    &mut output,
                )?;
            }
            if ev == PreprocessorEvent::EndIf {
                pre.apply(ev, &mut state);
            }
            last_indent = indent;
            return Ok(());
        }
        if line0.omp && !config.openmp {
            let mut quote = 0u8;
            for i in group.lines.clone() {
                emit_line_to_with_quote(
                    buf,
                    i,
                    state.current(),
                    config,
                    true,
                    false,
                    None,
                    None,
                    &mut quote,
                    &mut output,
                )?;
            }
            return Ok(());
        }
        let mut infos = Vec::new();
        for st in &group.statements {
            infos.push(classify(&st.text));
        }
        if infos.is_empty() {
            let mut quote = 0u8;
            for i in group.lines.clone() {
                let indent = if line0.kind == PhysicalLineKind::Comment {
                    state.current()
                } else {
                    0
                };
                emit_line_to_with_quote(
                    buf,
                    i,
                    indent,
                    config,
                    true,
                    false,
                    None,
                    None,
                    &mut quote,
                    &mut output,
                )?;
            }
            return Ok(());
        }
        let emit_config = if infos.iter().any(|x| x.contains_hollerith) {
            let mut adjusted = config.clone();
            adjusted.ws_remred = false;
            adjusted.ws_remred_value = 0;
            adjusted
        } else {
            config.clone()
        };
        let mut first_indent = state.current();
        let mut replacement_storage: Option<Vec<u8>> = None;
        for (position, info) in infos.iter().enumerate() {
            if info.kind != StatementKind::EndDo {
                if let Some(label) = info.statement_label {
                    state.close_label(label);
                }
            }
            if position == 0 && config.refactor_end && info.class == StatementClass::EndDefinition {
                if let Some(frame) = state
                    .frames
                    .last()
                    .filter(|frame| frame.kind != StatementKind::AbstractInterface)
                {
                    let mut replacement = crate::transform::refactor_end::end_text(
                        frame.kind,
                        frame.name.as_deref(),
                        config.uppercase_end,
                    );
                    if let Some(paren) = info.payload.iter().position(|byte| *byte == b'(') {
                        replacement.extend_from_slice(&info.payload[paren..]);
                    }
                    if let Some(comment) = &line0.comment_span {
                        replacement.push(b' ');
                        replacement.extend_from_slice(
                            &buf.bytes[comment.start as usize..comment.end as usize],
                        );
                    }
                    replacement_storage = Some(replacement);
                }
            }
            let statement_indent = transition(&mut state, info, config);
            if position == 0 {
                // A physical line containing multiple semicolon-separated
                // statements is emitted once, at the indentation of its
                // first statement.  Later statements still update state for
                // the following physical line.
                first_indent = statement_indent;
            }
            if info.class != StatementClass::Neutral || info.kind != StatementKind::Unknown {
                significant_seen = true;
                last_usable = group.lines.end;
            }
        }
        let mut paren_state = if infos.iter().any(|x| x.contains_hollerith) {
            None
        } else if config.align_paren || config.align_paren_value != 0 {
            Some(ParenAlignmentState::default())
        } else {
            None
        };
        let group_first_cont = trailing_ampersand(buf.code_bytes(line0));
        let mut first = true;
        let mut quote = 0u8;
        for i in group.lines.clone() {
            let is_pre = buf.lines[i].kind == PhysicalLineKind::Preprocessor;
            let this_align = if first {
                None
            } else {
                paren_state.as_ref().and_then(ParenAlignmentState::current)
            };
            if is_pre {
                emit_line_to_with_quote(
                    buf,
                    i,
                    state.current(),
                    &emit_config,
                    true,
                    false,
                    None,
                    None,
                    &mut quote,
                    &mut output,
                )?;
            } else {
                emit_line_to_with_quote(
                    buf,
                    i,
                    first_indent,
                    &emit_config,
                    first,
                    group_first_cont,
                    this_align,
                    if first {
                        replacement_storage.as_deref()
                    } else {
                        None
                    },
                    &mut quote,
                    &mut output,
                )?;
            }
            if let Some(paren_state) = paren_state.as_mut() {
                if !is_pre
                    && !matches!(
                        buf.lines[i].kind,
                        PhysicalLineKind::Blank
                            | PhysicalLineKind::Comment
                            | PhysicalLineKind::FindentFix
                    )
                {
                    let mut scan_target = if first {
                        first_indent
                    } else if let Some(alignment) = this_align {
                        alignment
                    } else if config.indent_continuation && group_first_cont {
                        first_indent.saturating_add(config.continuation_indent)
                    } else {
                        first_indent
                    };
                    if buf.lines[i].omp && config.openmp {
                        scan_target = scan_target.saturating_sub(3);
                    }
                    let scan_line = paren_scan_line(
                        buf.code_bytes(&buf.lines[i]),
                        first,
                        buf.lines[i].omp && config.openmp,
                        config.label_left,
                    );
                    if first {
                        let raw_line = buf.line_bytes(&buf.lines[i]);
                        let code_line = buf.code_bytes(&buf.lines[i]);
                        let label_is_indented = raw_line
                            .first()
                            .is_some_and(|byte| *byte == b' ' || *byte == b'\t');
                        if let Some(label_len) = leading_label_len(code_line) {
                            // Scan from the same column as the emitted label
                            // or body.  The label emitter uses at least one
                            // separator, so the body start is not always the
                            // configured indentation column.
                            scan_target = if config.label_left {
                                label_len + scan_target.saturating_sub(label_len).max(1)
                            } else if label_len == 2
                                && !label_is_indented
                                && scan_line
                                    .windows(2)
                                    .any(|pair| pair[0].is_ascii_whitespace() && pair[1] == b'(')
                            {
                                // findent's fixed-width two-digit label path
                                // leaves one separator column out of the
                                // alignment origin.
                                scan_target.saturating_sub(1)
                            } else {
                                scan_target
                            };
                        }
                    }
                    paren_state.scan(scan_line, scan_target);
                }
            }
            first = false;
            last_indent = clamp(first_indent, config.max_indent);
        }
        // An unrecognized statement must not disturb state; transition handles that.
        Ok(())
    })?;
    // Release the mutable sink borrow before the query result is written to
    // the caller's original writer. QueryWriter deliberately has no Drop
    // behavior; this explicit lifetime boundary is what matters here.
    #[allow(clippy::drop_non_drop)]
    drop(output);
    if config.last_usable {
        writeln!(out, "{last_usable}").map_err(FormatError::Write)?;
        return Ok(FormatMeta {
            last_indent,
            last_usable,
        });
    }
    if config.last_indent {
        writeln!(out, "{last_indent}").map_err(FormatError::Write)?;
        return Ok(FormatMeta {
            last_indent,
            last_usable,
        });
    }
    let _ = significant_seen;
    Ok(FormatMeta {
        last_indent,
        last_usable,
    })
}

fn paren_scan_line(line: &[u8], first: bool, omp: bool, label_left: bool) -> &[u8] {
    let mut s = line;
    while s
        .first()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        s = &s[1..];
    }
    if omp && s.starts_with(b"!$ ") {
        s = &s[3..];
    }
    if first && label_left {
        let mut digits = 0;
        while digits < s.len() && s[digits].is_ascii_digit() {
            digits += 1;
        }
        if digits > 0
            && s.get(digits)
                .is_some_and(|byte| *byte == b' ' || *byte == b'\t' || *byte == b'&')
        {
            s = &s[digits..];
            while s
                .first()
                .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
            {
                s = &s[1..];
            }
        }
    }
    s
}

fn leading_label_len(line: &[u8]) -> Option<usize> {
    let mut s = line;
    while s
        .first()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        s = &s[1..];
    }
    let mut digits = 0;
    while digits < s.len() && s[digits].is_ascii_digit() {
        digits += 1;
    }
    (digits > 0
        && s.get(digits)
            .is_some_and(|byte| *byte == b' ' || *byte == b'\t' || *byte == b'&'))
    .then_some(digits)
}

fn transition(state: &mut IndentStack, info: &StatementInfo, cfg: &FormatConfig) -> usize {
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

fn guess_start_indent(line: &[u8]) -> usize {
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
