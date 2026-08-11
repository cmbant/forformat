//! Steps 12-13: continuation markers and OpenMP sentinels.

use crate::{
    error::FormatError,
    source::{regions::LexState, RegionKind},
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
    let _ = cx;
    let original = document.lines.clone();
    let mut normalized = Vec::with_capacity(original.len());
    let mut continuation = false;
    let mut state = LexState::default();
    for (index, original_line) in original.iter().enumerate() {
        let incoming_literal = state.in_literal();
        let mut line = original_line.clone();
        state.scan(original_line, |_| {});
        let in_literal = incoming_literal || state.in_literal();
        let lexical_prefix =
            index > 0 && is_lexical_token_continuation(&original[index - 1], original_line);
        let lexical_suffix = index + 1 < original.len()
            && is_lexical_token_continuation(original_line, &original[index + 1]);
        if continuation && !in_literal && !lexical_prefix {
            line = remove_leading_continuation(&line);
        }
        if !in_literal && !lexical_suffix {
            line = normalize_continuation_marker(&line);
        }
        normalized.push(line);
        continuation = ends_with_continuation(original_line);
    }
    if normalized == original {
        return Ok(Changed::No);
    }
    document.set_lines(normalized);
    Ok(Changed::Text)
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
    let mut changed = Changed::No;
    let mut continuation = false;
    let mut updated = document.lines.clone();
    for line in &mut updated {
        let mut current = line.clone();
        let Some((sentinel_end, body_start, _omp_style)) = openmp_prefix(&current) else {
            continuation = false;
            continue;
        };
        let body = &current[body_start..];
        let is_continuation = body
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|start| body[start] == b'&');
        let should_repeat = is_continuation || continuation;
        if should_repeat {
            let mut rebuilt = current[..sentinel_end].to_vec();
            rebuilt.push(b' ');
            let mut start = body_start;
            while start < current.len() && current[start].is_ascii_whitespace() {
                start += 1;
            }
            if current.get(start) == Some(&b'&') {
                start += 1;
                while start < current.len() && current[start].is_ascii_whitespace() {
                    start += 1;
                }
            }
            rebuilt.extend_from_slice(&current[start..]);
            if rebuilt != current {
                current = rebuilt;
                changed = changed.or(Changed::Text);
            }
        }
        continuation = openmp_body(&current).is_some_and(ends_with_continuation);
        // `cx` carries the macro table used by the OpenMP vocabulary pass.  It
        // is deliberately applied after sentinel repair so every physical
        // directive line has the same protected prefix.
        if let Some((_, body_start, omp_style)) = openmp_prefix(&current) {
            let body = if omp_style || starts_with_omp(&current[body_start..]) {
                uppercase_openmp_body(&current[body_start..], cx)
            } else {
                current[body_start..].to_vec()
            };
            if body != current[body_start..] {
                let mut rebuilt = current[..body_start].to_vec();
                rebuilt.extend_from_slice(&body);
                current = rebuilt;
                changed = changed.or(Changed::Text);
            }
        }
        *line = current;
    }
    if changed != Changed::No {
        document.set_lines(updated);
    }
    Ok(changed)
}

fn is_lexical_token_continuation(first: &[u8], second: &[u8]) -> bool {
    lexical_prefix_end(first).is_some() && leading_lexical_suffix_start(second).is_some()
}

fn lexical_prefix_end(line: &[u8]) -> Option<usize> {
    if crate::source::regions::comment_start(line).is_some() {
        return None;
    }
    let mut state = LexState::default();
    state.scan(line, |_| {});
    if state.in_literal() {
        return None;
    }
    let mut end = line.len();
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let prefix_end = end.checked_sub(1)?;
    if line.get(prefix_end) != Some(&b'&') || prefix_end == 0 {
        return None;
    }
    (line[prefix_end - 1].is_ascii_alphanumeric() || line[prefix_end - 1] == b'_')
        .then_some(prefix_end)
}

fn leading_lexical_suffix_start(line: &[u8]) -> Option<usize> {
    let mut start = 0;
    while start < line.len() && line[start].is_ascii_whitespace() {
        start += 1;
    }
    if line.get(start) != Some(&b'&') {
        return None;
    }
    let suffix = start + 1;
    (line
        .get(suffix)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_'))
    .then_some(suffix)
}

fn ends_with_continuation(line: &[u8]) -> bool {
    let end = crate::source::regions::comment_start(line).unwrap_or(line.len());
    let mut end = end;
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end > 0 && line.get(end - 1) == Some(&b'&')
}

fn remove_leading_continuation(line: &[u8]) -> Vec<u8> {
    let mut start = 0;
    while start < line.len() && line[start].is_ascii_whitespace() {
        start += 1;
    }
    if line.get(start) != Some(&b'&') {
        return line.to_vec();
    }
    let mut next = start + 1;
    while next < line.len() && line[next].is_ascii_whitespace() {
        next += 1;
    }
    let mut result = line[..start].to_vec();
    result.extend_from_slice(&line[next..]);
    result
}

fn normalize_continuation_marker(line: &[u8]) -> Vec<u8> {
    let comment = crate::source::regions::comment_start(line).unwrap_or(line.len());
    let mut end = comment;
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 || line.get(end - 1) != Some(&b'&') {
        return line.to_vec();
    }
    let mut code_end = end - 1;
    while code_end > 0 && line[code_end - 1].is_ascii_whitespace() {
        code_end -= 1;
    }
    let mut result = line[..code_end].to_vec();
    result.extend_from_slice(b" &");
    result.extend_from_slice(&line[end..comment]);
    result.extend_from_slice(&line[comment..]);
    result
}

fn openmp_prefix(line: &[u8]) -> Option<(usize, usize, bool)> {
    let start = line.iter().position(|byte| !byte.is_ascii_whitespace())?;
    if !line[start..].starts_with(b"!$") {
        return None;
    }
    let mut sentinel_end = start + 2;
    let omp_style = line
        .get(sentinel_end..sentinel_end + 3)
        .is_some_and(|bytes| bytes.eq_ignore_ascii_case(b"omp"));
    if omp_style {
        sentinel_end += 3;
    }
    let mut body_start = sentinel_end;
    while body_start < line.len() && line[body_start].is_ascii_whitespace() {
        body_start += 1;
    }
    Some((sentinel_end, body_start, omp_style))
}

fn openmp_body(line: &[u8]) -> Option<&[u8]> {
    openmp_prefix(line).map(|(_, start, _)| &line[start..])
}

fn starts_with_omp(body: &[u8]) -> bool {
    let body = body.trim_ascii_start();
    body.get(..3)
        .is_some_and(|word| word.eq_ignore_ascii_case(b"omp"))
        && body
            .get(3)
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
}

fn uppercase_openmp_body(body: &[u8], cx: &PassContext) -> Vec<u8> {
    const KEYWORDS: &[&[u8]] = &[
        b"omp",
        b"do",
        b"atomic",
        b"barrier",
        b"cancel",
        b"cancellation",
        b"critical",
        b"declare",
        b"distribute",
        b"end",
        b"flush",
        b"loop",
        b"master",
        b"masked",
        b"ordered",
        b"parallel",
        b"sections",
        b"section",
        b"simd",
        b"single",
        b"target",
        b"task",
        b"taskgroup",
        b"taskloop",
        b"taskwait",
        b"taskyield",
        b"teams",
        b"threadprivate",
        b"workshare",
        b"allocate",
        b"collapse",
        b"copyin",
        b"copyprivate",
        b"default",
        b"firstprivate",
        b"if",
        b"lastprivate",
        b"linear",
        b"map",
        b"nowait",
        b"num_threads",
        b"private",
        b"reduction",
        b"schedule",
        b"static",
        b"dynamic",
        b"guided",
        b"runtime",
        b"shared",
        b"simdlen",
        b"proc_bind",
        b"defaultmap",
        b"depend",
        b"device",
        b"dist_schedule",
        b"final",
        b"grainsize",
        b"hint",
        b"in_reduction",
        b"is_device_ptr",
        b"mergeable",
        b"nogroup",
        b"num_tasks",
        b"order",
        b"priority",
        b"safelen",
        b"thread_limit",
        b"to",
        b"from",
        b"use_device_addr",
        b"use_device_ptr",
    ];
    let mut result = Vec::with_capacity(body.len());
    let mut state = LexState::default();
    let mut regions = Vec::new();
    state.scan(body, |region| regions.push(region));
    for region in regions {
        let bytes = &body[region.range.clone()];
        if region.kind != RegionKind::Code {
            result.extend_from_slice(bytes);
            continue;
        }
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                let word = &bytes[start..index];
                if !cx.project.macros.contains(word)
                    && KEYWORDS
                        .iter()
                        .any(|keyword| word.eq_ignore_ascii_case(keyword))
                {
                    result.extend(word.iter().map(u8::to_ascii_uppercase));
                } else {
                    result.extend_from_slice(word);
                }
            } else {
                result.push(bytes[index]);
                index += 1;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{normalize_continuations, normalize_openmp_continuation_sentinels};
    use crate::{
        analysis::{FileFacts, ProjectContext, ScopeTree},
        config::{FormatConfig, FormatMode},
        transform::document::Document,
        transform::pipeline::{Changed, PassContext},
    };

    fn cx<'a>(local: &'a FileFacts, project: &'a ProjectContext) -> PassContext<'a> {
        let analysis = Box::leak(Box::new(Document::from_bytes(b"").analyze().unwrap()));
        let scopes = Box::leak(Box::new(ScopeTree::build(analysis)));
        let config = Box::leak(Box::new(FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        }));
        PassContext {
            config,
            project,
            local,
            analysis,
            scopes,
        }
    }

    #[test]
    fn continuation_markers_are_normalized_without_touching_literals() {
        let mut document = Document::from_bytes(b"x = a &\n  & b\ny = 'a &\n &b'\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        assert_eq!(
            normalize_continuations(&mut document, &cx(&local, &project)).unwrap(),
            Changed::Text
        );
        assert_eq!(document.lines[0], b"x = a &".to_vec());
        assert_eq!(document.lines[1], b"  b".to_vec());
        assert_eq!(document.lines[2], b"y = 'a &".to_vec());
        assert_eq!(document.lines[3], b" &b'".to_vec());
    }

    #[test]
    fn openmp_sentinels_repeat_and_macros_keep_their_case() {
        let mut document = Document::from_bytes(b"!$ parallel do private=foo\n!$ & map(to:X)\n");
        let local = FileFacts::default();
        let mut project = ProjectContext::empty();
        project.define(&[crate::config::MacroDefine {
            name: "private".into(),
            value: None,
        }]);
        assert_ne!(
            normalize_openmp_continuation_sentinels(&mut document, &cx(&local, &project)).unwrap(),
            Changed::No
        );
        assert!(document.lines[1].starts_with(b"!$"));
        assert!(!document.lines[0]
            .windows(b"PRIVATE".len())
            .any(|w| w == b"PRIVATE"));
        let before = document.lines.clone();
        assert_eq!(
            normalize_openmp_continuation_sentinels(&mut document, &cx(&local, &project)).unwrap(),
            Changed::No
        );
        assert_eq!(document.lines, before);
    }
}
