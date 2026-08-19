//! Emission.  Every structural decision has already been made by
//! [`super::planner`]; this module turns a group plan into bytes.

use super::{
    continuation::ParenAlignmentState,
    emitter::{emit_line_to_with_quote, EmitStyle, LinePlacement},
    planner::{GroupPlan, PlanBody, Planner},
};
use crate::{
    config::{FormatConfig, FormatMode},
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

pub fn format_buffer<B: AsRef<[u8]>, W: Write>(
    buf: &SourceBuffer<B>,
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    if buf.bytes.as_ref().is_empty() {
        if config.last_usable {
            writeln!(out, "1").map_err(FormatError::Write)?;
        } else if config.last_indent {
            writeln!(out, "0").map_err(FormatError::Write)?;
        }
        return Ok(FormatMeta {
            last_indent: 0,
            last_usable: 1,
            declines: Vec::new(),
        });
    }
    let query_mode = config.last_indent || config.last_usable;
    let mut output = QueryWriter {
        inner: out,
        discard: query_mode,
    };
    let mut planner = Planner::new(config);
    let mut last_indent = 0;
    let mut last_usable = 1;
    LogicalGroup::visit(buf, |group| {
        let plan = planner.plan(buf, &group, config);
        if let Some(value) = plan.last_indent {
            last_indent = value;
        }
        if let Some(value) = plan.last_usable {
            last_usable = value;
        }
        emit_group(buf, &plan, config, &mut output)
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
            declines: Vec::new(),
        });
    }
    if config.last_indent {
        writeln!(out, "{last_indent}").map_err(FormatError::Write)?;
        return Ok(FormatMeta {
            last_indent,
            last_usable,
            declines: Vec::new(),
        });
    }
    Ok(FormatMeta {
        last_indent,
        last_usable,
        declines: Vec::new(),
    })
}

/// Write the physical lines of one planned group.
pub fn emit_group<B: AsRef<[u8]>, W: Write>(
    buf: &SourceBuffer<B>,
    plan: &GroupPlan,
    config: &FormatConfig,
    out: &mut W,
) -> Result<(), FormatError> {
    let style = EmitStyle {
        config,
        apply_indent: plan.apply_indent,
        remred: plan.remred,
    };
    let mut quote = 0u8;
    match &plan.body {
        PlanBody::Uniform { indent } => {
            for i in plan.lines.clone() {
                emit_line_to_with_quote(
                    buf,
                    i,
                    LinePlacement {
                        indent: *indent,
                        first: true,
                        previous_cont: false,
                        alignment: None,
                    },
                    &style,
                    None,
                    &mut quote,
                    out,
                )?;
            }
        }
        PlanBody::Code {
            first_indent,
            directive_indent,
            group_first_cont,
            align,
            replacement,
        } => {
            let mut paren_state = align.then(ParenAlignmentState::default);
            let mut first = true;
            for i in plan.lines.clone() {
                let is_pre = buf.lines[i].kind == PhysicalLineKind::Preprocessor;
                let this_align = if first {
                    None
                } else {
                    paren_state.as_ref().and_then(ParenAlignmentState::current)
                };
                // The alignment scan below has to see the same bytes the
                // emitter is about to write, including whatever `--ws-remred`
                // does to this line's internal whitespace. Snapshot the
                // string-quote state the write path is about to consume so
                // the scan's own reduction pass agrees with it, without
                // letting that scan advance the real one.
                let remred_quote = quote;
                if is_pre {
                    emit_line_to_with_quote(
                        buf,
                        i,
                        LinePlacement {
                            indent: *directive_indent,
                            first: true,
                            previous_cont: false,
                            alignment: None,
                        },
                        &style,
                        None,
                        &mut quote,
                        out,
                    )?;
                } else {
                    emit_line_to_with_quote(
                        buf,
                        i,
                        LinePlacement {
                            indent: *first_indent,
                            first,
                            previous_cont: *group_first_cont,
                            alignment: this_align,
                        },
                        &style,
                        if first { replacement.as_deref() } else { None },
                        &mut quote,
                        out,
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
                        advance_alignment(
                            buf,
                            i,
                            paren_state,
                            *first_indent,
                            first,
                            this_align,
                            *group_first_cont,
                            &style,
                            remred_quote,
                        );
                    }
                }
                first = false;
            }
        }
    }
    Ok(())
}

/// Feed one emitted physical line into the parenthesis-alignment tracker.
///
/// The tracker models columns of the *output*, so the scan has to start at the
/// column the emitter actually used, which is not always the configured
/// indentation: labels and OpenMP sentinels shift the body.
#[allow(clippy::too_many_arguments)]
fn advance_alignment<B: AsRef<[u8]>>(
    buf: &SourceBuffer<B>,
    index: usize,
    paren_state: &mut ParenAlignmentState,
    first_indent: usize,
    first: bool,
    this_align: Option<usize>,
    group_first_cont: bool,
    style: &EmitStyle,
    remred_quote: u8,
) {
    let config = style.config;
    let line = &buf.lines[index];
    let mut scan_target = if first {
        first_indent
    } else if let Some(alignment) = this_align {
        alignment
    } else if config.indent_continuation && group_first_cont {
        first_indent.saturating_add(config.continuation_indent)
    } else {
        first_indent
    };
    if line.omp && config.openmp {
        scan_target = scan_target.saturating_sub(3);
    }
    let scan_line = paren_scan_line(
        buf.code_bytes(line),
        first,
        line.omp && config.openmp,
        config.label_left,
    );
    if first {
        let raw_line = buf.line_bytes(line);
        let code_line = buf.code_bytes(line);
        let label_is_indented = raw_line
            .first()
            .is_some_and(|byte| *byte == b' ' || *byte == b'\t');
        if let Some(label_len) = leading_label_len(code_line) {
            // Scan from the same column as the emitted label or body.  The
            // label emitter uses at least one separator, so the body start is
            // not always the configured indentation column.
            scan_target = if config.label_left {
                label_len + scan_target.saturating_sub(label_len).max(1)
            } else if label_len == 2
                && !label_is_indented
                && scan_line
                    .windows(2)
                    .any(|pair| pair[0].is_ascii_whitespace() && pair[1] == b'(')
            {
                // findent's fixed-width two-digit label path leaves one
                // separator column out of the alignment origin.
                scan_target.saturating_sub(1)
            } else {
                scan_target
            };
        }
    }
    // `paren_state` tracks columns of the bytes the emitter actually writes.
    // `write_body` collapses redundant internal whitespace under
    // `--ws-remred` before those bytes reach `out`, so a padded source line
    // (`pars(      1:    100)=(/&`) would otherwise be scanned at its
    // *authored* column and disagree with the shorter column `write_body`
    // lands on, moving the alignment out from under the continuation it
    // targets. Mirror that same reduction here so the scan and the write
    // agree on where every byte ends up.
    let mut reduced = Vec::new();
    let scan_line: &[u8] = if style.remred {
        let alignment_runs_after = config.mode == FormatMode::Full;
        let mut quote = remred_quote;
        crate::transform::whitespace::reduce_line_into_protected(
            scan_line,
            &mut quote,
            alignment_runs_after && config.align_declarations,
            alignment_runs_after && config.align_comments,
            &mut |byte| reduced.push(byte),
        );
        &reduced
    } else {
        scan_line
    };
    paren_state.scan(scan_line, scan_target);
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
