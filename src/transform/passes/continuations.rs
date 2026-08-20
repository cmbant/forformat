//! Steps 12-13: continuation markers and OpenMP sentinels.

use crate::{
    error::FormatError,
    source::{regions, regions::LexState, RegionKind},
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
/// The normalizer strips a *leading* `&` from continuation lines. Rust keeps
/// that rule for pre-existing markers and never emits one, which is what makes
/// findent's `-K` (`--indent_ampersand`) inert on already-formatted source
/// rather than contradictory: `-K` governs where an existing leading `&` sits,
/// and the wrapper simply never creates one (§7.1 of the port plan).
///
/// The `&` that splits a *lexical token* is the exception, and it is not
/// cosmetic: `sub&` / `&routine` is one `subroutine` token only while the `&`
/// immediately follows the token's characters. Writing the marker out as ` &`
/// there leaves `sub routine`, which is a different program — and one gfortran
/// rejects. Both neighbours are found by skipping physical lines the logical
/// statement also steps over: comments, blanks and preprocessor directives.
/// A conditional `!$ ` line is different: it is Fortran code with a sentinel
/// prefix, so its body participates in continuation and lexical-token state.
///
/// Port target: `normalize_continuations`.
pub fn normalize_continuations(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = cx;
    let original = document.lines.clone();
    let (previous_statement_line, next_statement_line) = statement_neighbours(&original);
    let mut normalized = Vec::with_capacity(original.len());
    let mut continuation = false;
    let mut state = LexState::default();
    let mut open_stream: Option<bool> = None;
    for (index, original_line) in original.iter().enumerate() {
        let conditional = conditional_stream(original_line);
        let passed_over = regions::stepped_over_by_continuation(original_line)
            || open_stream.is_some_and(|open| open != conditional);
        let incoming_protected = state.in_literal() || state.in_hollerith();
        let mut line = original_line.clone();
        let code = fortran_code(original_line);
        let mut comment = None;
        if !passed_over {
            state.scan(code, |region| {
                if comment.is_none() && region.kind == RegionKind::Comment {
                    comment = Some(region.range.start);
                }
            });
        }
        let protected = incoming_protected || state.in_literal() || state.in_hollerith();
        let lexical_prefix = previous_statement_line[index]
            .is_some_and(|at| is_lexical_token_continuation(&original[at], original_line));
        let lexical_suffix = next_statement_line[index]
            .is_some_and(|at| is_lexical_token_continuation(original_line, &original[at]));
        if continuation && !protected && !lexical_prefix {
            line = remove_leading_continuation(&line);
        }
        if !protected && !lexical_suffix {
            line = normalize_continuation_marker(&line);
        }
        normalized.push(line);
        if !passed_over {
            continuation = ends_with_continuation_before(code, comment);
            if code.trim_ascii_end().last() != Some(&b'&') {
                state = LexState::default();
            }
            let still_open = continuation || state.in_literal() || state.in_hollerith();
            open_stream = still_open.then_some(conditional);
        }
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
/// Note that `--openmp=0` disables OpenMP *indentation* while directive *text*
/// normalization stays on: two concerns, two config fields, never one flag.
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
        let Some((_, body_start, omp_style)) = openmp_prefix(&current) else {
            continuation = false;
            continue;
        };
        // `!$ ` conditional-compilation lines are ordinary Fortran code with
        // a sentinel prefix. Step 12 owns their continuation markers, including
        // the lexical-token exception; this pass only normalizes `!$OMP`.
        if !omp_style {
            continuation = false;
            continue;
        }
        let body = &current[body_start..];
        let is_continuation = body
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|start| body[start] == b'&');
        let should_repeat = is_continuation || continuation;
        let mut start = body_start;
        if should_repeat && current.get(start) == Some(&b'&') {
            start += 1;
            while start < current.len() && current[start].is_ascii_whitespace() {
                start += 1;
            }
        }
        let normalized_body = normalize_openmp_body(&current[start..], cx);
        let indent_end = current
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(0);
        let mut rebuilt = current[..indent_end].to_vec();
        rebuilt.extend_from_slice(b"!$OMP ");
        rebuilt.extend_from_slice(&normalized_body);
        if rebuilt != current {
            current = rebuilt;
            changed = changed.or(Changed::Text);
        }
        continuation = openmp_body(&current).is_some_and(ends_with_continuation);
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

/// Slice away the conditional-compilation sentinel from a physical line.
///
/// `SourceBuffer` classifies exactly `!$ ` (after indentation) as Fortran code
/// and scans the bytes after that three-byte sentinel. Continuation syntax must
/// use the same view or `!` is mistaken for a comment marker.
fn fortran_code(line: &[u8]) -> &[u8] {
    &line[fortran_code_start(line)..]
}

fn fortran_code_start(line: &[u8]) -> usize {
    let Some(start) = line.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return 0;
    };
    if line
        .get(start..)
        .is_some_and(|rest| rest.starts_with(b"!$ "))
    {
        start + 3
    } else {
        0
    }
}

/// For every line, the nearest line above and below it that carries part of the
/// same statement.
///
/// Precomputed in two linear passes rather than searched per line: a file that
/// opens with a long comment header would otherwise make each of those lines
/// rescan the whole header, which is quadratic and cost 2s on a 40k-line file.
fn statement_neighbours(lines: &[Vec<u8>]) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let carries_statement: Vec<bool> = lines
        .iter()
        .map(|line| !regions::stepped_over_by_continuation(line))
        .collect();
    // One cursor per stream, so a neighbour is always of the line's own class.
    let stream: Vec<usize> = lines
        .iter()
        .map(|line| usize::from(conditional_stream(line)))
        .collect();
    let mut previous = vec![None; lines.len()];
    let mut nearest = [None, None];
    for index in 0..lines.len() {
        previous[index] = nearest[stream[index]];
        if carries_statement[index] {
            nearest[stream[index]] = Some(index);
        }
    }
    let mut next = vec![None; lines.len()];
    let mut nearest = [None, None];
    for index in (0..lines.len()).rev() {
        next[index] = nearest[stream[index]];
        if carries_statement[index] {
            nearest[stream[index]] = Some(index);
        }
    }
    (previous, next)
}

/// Which continuation stream a physical line belongs to.
///
/// `!$ ` conditional-compilation lines form their own stream, and a statement
/// only ever continues within one stream. `!$ x&` splices with `!$ &y` under
/// both readings of the sentinel, so those two are neighbours. An ordinary
/// `code&` splices with `&more` across an intervening `!$ ` line only when
/// OpenMP is *off* — with OpenMP on, that source does not compile at all — so
/// the `!$ ` line is stepped over rather than allowed to break the token.
/// findent agrees in both directions, and un-gluing `call my&` there turns a
/// program that compiles and runs into `call my sub(...)`, which does not.
fn conditional_stream(line: &[u8]) -> bool {
    line.trim_ascii_start().starts_with(b"!$ ")
}

fn lexical_prefix_end(line: &[u8]) -> Option<usize> {
    let line = fortran_code(line);
    if crate::source::regions::comment_start(line).is_some() {
        return None;
    }
    let mut state = LexState::default();
    state.scan(line, |_| {});
    if state.in_literal() || state.in_hollerith() {
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
    let line = fortran_code(line);
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
    let line = fortran_code(line);
    ends_with_continuation_before(line, crate::source::regions::comment_start(line))
}

fn ends_with_continuation_before(line: &[u8], comment: Option<usize>) -> bool {
    let mut end = comment.unwrap_or(line.len());
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end > 0 && line.get(end - 1) == Some(&b'&')
}

fn remove_leading_continuation(line: &[u8]) -> Vec<u8> {
    let code_start = fortran_code_start(line);
    let code = &line[code_start..];
    let mut start = 0;
    while start < code.len() && code[start].is_ascii_whitespace() {
        start += 1;
    }
    if code.get(start) != Some(&b'&') {
        return line.to_vec();
    }
    let mut next = start + 1;
    while next < code.len() && code[next].is_ascii_whitespace() {
        next += 1;
    }
    let mut result = line[..code_start + start].to_vec();
    result.extend_from_slice(&code[next..]);
    result
}

fn normalize_continuation_marker(line: &[u8]) -> Vec<u8> {
    let code_start = fortran_code_start(line);
    let code = &line[code_start..];
    let comment = crate::source::regions::comment_start(code).unwrap_or(code.len());
    let mut end = comment;
    while end > 0 && code[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 || code.get(end - 1) != Some(&b'&') {
        return line.to_vec();
    }
    let mut code_end = end - 1;
    while code_end > 0 && code[code_end - 1].is_ascii_whitespace() {
        code_end -= 1;
    }
    let mut result = line[..code_start + code_end].to_vec();
    result.extend_from_slice(b" &");
    result.extend_from_slice(&code[end..comment]);
    result.extend_from_slice(&code[comment..]);
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

fn normalize_openmp_body(body: &[u8], cx: &PassContext) -> Vec<u8> {
    let upper = uppercase_openmp_body(body, cx);
    let spaced = crate::transform::passes::line_rules::normalize_delimiter_spacing(&upper, cx);
    normalize_openmp_clause_separators(&spaced)
}

/// Match the narrow OpenMP clause rule: `DEFAULT(X) PRIVATE(Y)`
/// becomes `DEFAULT(X), PRIVATE(Y)`, while adjacent tokens without whitespace
/// remain authored.  This runs only on `!$OMP` bodies, never on `!$` lines.
fn normalize_openmp_clause_separators(body: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(body.len() + 8);
    let mut index = 0;
    while index < body.len() {
        if body[index] == b'!' {
            output.extend_from_slice(&body[index..]);
            break;
        }
        if body[index] == b')' {
            let whitespace_start = index + 1;
            let mut whitespace_end = whitespace_start;
            while whitespace_end < body.len() && body[whitespace_end].is_ascii_whitespace() {
                whitespace_end += 1;
            }
            let mut token_end = whitespace_end;
            if token_end < body.len()
                && (body[token_end].is_ascii_alphabetic() || body[token_end] == b'_')
            {
                token_end += 1;
                while token_end < body.len()
                    && (body[token_end].is_ascii_alphanumeric() || body[token_end] == b'_')
                {
                    token_end += 1;
                }
                let mut opening = token_end;
                while opening < body.len() && body[opening].is_ascii_whitespace() {
                    opening += 1;
                }
                if opening < body.len()
                    && body[opening] == b'('
                    && whitespace_end > whitespace_start
                {
                    output.extend_from_slice(b"), ");
                    index = whitespace_end;
                    continue;
                }
            }
        }
        output.push(body[index]);
        index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{normalize_continuations, normalize_openmp_continuation_sentinels, run};
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

    /// A token split across a continuation keeps its `&` glued to the token
    /// even when a comment, a blank or a preprocessor directive sits between
    /// the halves.
    /// Normalizing the marker to ` &` there un-splits the token: `sub&` /
    /// `&routine` stops being `subroutine` and becomes `sub routine`, which is
    /// not the program that was authored and does not compile.
    #[test]
    fn a_split_token_survives_a_separator_between_its_halves() {
        for separator in [
            "!comment",
            "",
            "   ",
            "#ifdef X",
            "?? if X",
            "! don't stop here",
        ] {
            let source = format!("sub&\n{separator}\n&routine sub\nx = 1\nend subroutine sub\n");
            let mut document = Document::from_bytes(source.as_bytes());
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            normalize_continuations(&mut document, &cx(&local, &project)).unwrap();
            assert_eq!(
                document.lines[0],
                b"sub&".to_vec(),
                "marker un-split the token across {separator:?}"
            );
            assert_eq!(
                document.lines[2],
                b"&routine sub".to_vec(),
                "leading marker lost across {separator:?}"
            );
        }
    }

    #[test]
    fn ordinary_continuation_state_survives_passed_over_lines() {
        for separator in ["!comment", "", "   ", "#ifdef X", "?? if X"] {
            let source = format!("x = a &\n{separator}\n  & b\n");
            let mut document = Document::from_bytes(source.as_bytes());
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            normalize_continuations(&mut document, &cx(&local, &project)).unwrap();
            assert_eq!(
                document.lines[2],
                b"  b".to_vec(),
                "continuation state stopped at {separator:?}"
            );
        }
    }

    #[test]
    fn continued_literal_state_survives_passed_over_lines() {
        for separator in ["! don't close this", "", "#ifdef X", "?? if X"] {
            let source = format!("x = 'ab &\n{separator}\n&cd'\n");
            let mut document = Document::from_bytes(source.as_bytes());
            let original = document.lines.clone();
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            assert_eq!(
                normalize_continuations(&mut document, &cx(&local, &project)).unwrap(),
                Changed::No,
                "literal state was lost at {separator:?}"
            );
            assert_eq!(document.lines, original);
        }
    }

    #[test]
    fn unterminated_literal_without_continuation_does_not_leak_state() {
        let mut document = Document::from_bytes(b"x = 'unterminated\ny = a &\n  & b\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        normalize_continuations(&mut document, &cx(&local, &project)).unwrap();
        assert_eq!(document.lines[2], b"  b".to_vec());
    }

    #[test]
    fn conditional_compilation_lines_participate_in_continuation_state() {
        let mut document =
            Document::from_bytes(b"!$ sub&\n!$ &routine sub\n!$ x = a &\n!$   & b\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        run(&mut document, &cx(&local, &project)).unwrap();

        assert_eq!(document.lines[0], b"!$ sub&".to_vec());
        assert_eq!(document.lines[1], b"!$ &routine sub".to_vec());
        assert_eq!(document.lines[2], b"!$ x = a &".to_vec());
        assert_eq!(document.lines[3], b"!$   b".to_vec());
    }

    /// A statement continues only within its own sentinel stream, so an
    /// ordinary split token is glued across an intervening `!$ ` line rather
    /// than broken by it.
    ///
    /// Un-gluing it is not a cosmetic choice. `call my&` / `!$ y = 2` /
    /// `&sub(y)` compiles and runs without OpenMP, printing 42; rewriting the
    /// marker to `call my &` makes it `call my sub(y)`, which does not compile.
    /// With OpenMP on, that source does not compile either way, so breaking the
    /// token protects nothing. findent keeps `call my&` here too.
    #[test]
    fn a_split_token_is_glued_across_the_other_sentinel_stream() {
        let mut document = Document::from_bytes(b"sub&\n!$ x = 1\n&routine sub\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        normalize_continuations(&mut document, &cx(&local, &project)).unwrap();

        assert_eq!(document.lines[0], b"sub&".to_vec());
        assert_eq!(document.lines[2], b"&routine sub".to_vec());
    }

    /// The converse: an ordinary line between two `!$ ` halves is likewise not
    /// a neighbour of either, so the conditional split token stays glued.
    #[test]
    fn the_conditional_stream_keeps_its_own_neighbours() {
        let mut document = Document::from_bytes(b"!$ sub&\nx = 1\n!$ &routine sub\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        normalize_continuations(&mut document, &cx(&local, &project)).unwrap();

        assert_eq!(document.lines[0], b"!$ sub&".to_vec());
        assert_eq!(document.lines[2], b"!$ &routine sub".to_vec());
    }

    /// A literal opened on an ordinary line is not closed by a `!$ ` line
    /// sitting inside it: without OpenMP that line is a comment, and with
    /// OpenMP the source does not compile.
    #[test]
    fn a_literal_survives_a_line_from_the_other_stream() {
        for (open, sep) in [("x = 'ab &", "!$ y = 2"), ("!$ x = 'ab &", "y = 2")] {
            let source = format!(
                "{open}\n{sep}\n{}&cd'\n",
                if open.starts_with("!$") { "!$ " } else { "" }
            );
            let mut document = Document::from_bytes(source.as_bytes());
            let original = document.lines.clone();
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            assert_eq!(
                normalize_continuations(&mut document, &cx(&local, &project)).unwrap(),
                Changed::No,
                "literal state was lost across {sep:?}"
            );
            assert_eq!(document.lines, original);
        }
    }

    #[test]
    fn openmp_sentinels_repeat_and_macros_keep_their_case() {
        let mut document =
            Document::from_bytes(b"!$omp parallel do private=foo &\n!$omp & map(to:X)\n");
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
