//! Reflow for whole-line directives that repeat their sentinel on continuation
//! lines instead of using Fortran statement continuation syntax.

use super::trim;
use crate::{
    format::wrapping::{self, Decline},
    source::{
        syntax::{conditional_compilation_prefix, openmp_directive_prefix},
        LogicalGroup, SourceBuffer,
    },
    transform::document::Document,
};

/// The sentinel prefix a repeated directive line carries, in the spelling this
/// document has already settled on, plus its one separating blank.
fn spelling(line: &[u8], sentinel_end: usize) -> Vec<u8> {
    let start = line
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(0);
    let mut spelling = line[start..sentinel_end].to_vec();
    spelling.push(b' ');
    spelling
}

/// The sentinel a whole-line directive repeats on each continuation.
fn reflow_sentinel(line: &[u8]) -> Option<(usize, Vec<u8>)> {
    openmp_directive_prefix(line)
        .map(|prefix| (prefix.body_start, spelling(line, prefix.sentinel_end)))
}

fn canonical_reflow_sentinel(line: &[u8]) -> Option<(usize, Vec<u8>)> {
    if let Some(prefix) = openmp_directive_prefix(line) {
        return Some((prefix.body_start, spelling(line, prefix.sentinel_end)));
    }
    conditional_compilation_prefix(line)
        .filter(|prefix| {
            prefix.kind == crate::source::syntax::ConditionalPrefixKind::BlankSeparated
        })
        .map(|prefix| (prefix.body_start, b"!$ ".to_vec()))
}

pub(super) fn prepare<B: AsRef<[u8]>>(
    document: &Document,
    buffer: &SourceBuffer<B>,
    group: &LogicalGroup,
) -> Option<Vec<u8>> {
    // A continued OpenMP directive is already a sequence of physical
    // directives. Joining it here would erase the repeated sentinel and one
    // physical line when the wrapper decides the joined text fits.
    let mut indices: Vec<usize> = group.lines.clone().collect();
    if indices.len() > 1 {
        if !is_openmp_line(&document.lines[indices[0]])
            || is_openmp_line(&document.lines[indices[1]])
        {
            return None;
        }
        indices.truncate(1);
    }

    let index = *indices.first()?;
    let line = document.lines.get(index)?;
    let indent_end = line.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let conditional = buffer
        .lines
        .get(index)
        .is_some_and(|line| line.is_conditional_compilation());
    // Conditional-compilation lines are Fortran statements continued with `&`;
    // the statement path models their sentinel and continuation geometry.
    if conditional {
        return None;
    }
    let (body_start, sentinel) = reflow_sentinel(line)?;
    let body = line.get(body_start..)?.trim_ascii_start();
    if crate::source::regions::comment_start(body).is_some() {
        return None;
    }

    let mut joined = line[..indent_end].to_vec();
    joined.extend_from_slice(&sentinel);
    joined.extend_from_slice(body);
    Some(joined)
}

fn is_openmp_line(line: &[u8]) -> bool {
    openmp_directive_prefix(line).is_some()
}

pub(super) fn wrap_line(line: &[u8], line_length: usize) -> Result<Vec<Vec<u8>>, Decline> {
    let indent_end = line
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(0);
    let indent = &line[..indent_end];
    let (body_start, sentinel) = canonical_reflow_sentinel(line).ok_or(Decline::NoSafeBreak)?;
    let mut prefix = indent.to_vec();
    prefix.extend_from_slice(&sentinel);
    if line.len() <= line_length {
        return Ok(vec![line.to_vec()]);
    }
    let mut body = line
        .get(body_start..)
        .ok_or(Decline::NoSafeBreak)?
        .trim_ascii_start()
        .to_vec();
    let mut result = Vec::new();
    while prefix.len() + body.len() > line_length {
        let limit = line_length.saturating_sub(prefix.len() + 2);
        let position = wrapping::wrap_position(&body, limit).ok_or(Decline::NoSafeBreak)?;
        let mut physical = prefix.clone();
        physical.extend_from_slice(trim(&body[..position]));
        physical.extend_from_slice(b" &");
        result.push(physical);
        body = trim(&body[position..]).to_vec();
    }
    let mut last = prefix;
    last.extend_from_slice(&body);
    result.push(last);
    Ok(result)
}

pub(super) fn reindent(line: &[u8], indent: usize) -> Vec<u8> {
    let mut result = vec![b' '; indent];
    result.extend_from_slice(line.trim_ascii_start());
    result
}
