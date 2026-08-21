//! Steps 12-13: continuation markers and OpenMP sentinels.

use crate::{
    error::FormatError,
    source::{
        regions::LexState,
        syntax::{
            conditional_compilation_body_start, conditional_compilation_prefix,
            openmp_directive_prefix, ConditionalPrefixKind, SourceStream,
        },
        PhysicalLineKind, RegionKind,
    },
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
/// A conditional `!$` line followed by a horizontal blank is different: it is
/// Fortran code with a sentinel prefix, so its body participates in continuation
/// and lexical-token state.
///
/// Port target: `normalize_continuations`.
pub fn normalize_continuations(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let original = document.lines.clone();
    debug_assert_eq!(
        original.len(),
        cx.analysis.buffer.lines.len(),
        "continuation pass requires analysis of the current document",
    );
    let (previous_statement_line, next_statement_line) = statement_neighbours(cx);
    let mut normalized = Vec::with_capacity(original.len());
    let mut continuation = false;
    let mut state = LexState::default();
    let mut open_stream: Option<SourceStream> = None;
    for (index, original_line) in original.iter().enumerate() {
        let stream = source_stream(cx, index);
        let passed_over =
            !carries_statement(cx, index) || open_stream.is_some_and(|open| open != stream);
        let incoming_protected = state.in_literal() || state.in_hollerith();
        let mut line = original_line.clone();
        let code = fortran_code(original_line);
        let mut line_continued = false;
        if !passed_over {
            line_continued = state.scan_line(code, |_| {}).continued;
        }
        let protected = incoming_protected || state.in_literal() || state.in_hollerith();
        let lexical_prefix = previous_statement_line[index]
            .is_some_and(|at| is_lexical_token_continuation(&original[at], original_line));
        let lexical_suffix = next_statement_line[index]
            .is_some_and(|at| is_lexical_token_continuation(original_line, &original[at]));
        if !passed_over {
            if continuation && !protected && !lexical_prefix {
                line = remove_leading_continuation(&line);
            }
            if line_continued && !protected && !lexical_suffix {
                line = normalize_continuation_marker(&line);
            }
        }
        normalized.push(line);
        if !passed_over {
            continuation = line_continued;
            if !continuation {
                state = LexState::default();
            }
            let still_open = continuation || state.in_literal() || state.in_hollerith();
            open_stream = still_open.then_some(stream);
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
/// A continued directive needs a repeated reserved sentinel (`!$OMP` or
/// `!$OMPX`) on each physical line with valid `&` markers, and the available
/// width has to account for the sentinel. `--openmp=0` disables OpenMP
/// *indentation* while directive *text* normalization stays on: two concerns,
/// two config fields, never one flag.
///
/// Port target: `normalize_openmp_continuation_sentinels`,
/// `prepare_sentinel_reflow`, `wrap_sentinel_line`.
pub fn normalize_openmp_continuation_sentinels(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let mut changed = Changed::No;
    let mut continuation = false;
    let mut updated = document.lines.clone();
    for line in &mut updated {
        let mut current = line.clone();
        let Some(prefix) = openmp_directive_prefix(&current) else {
            continuation = false;
            continue;
        };
        let body = &current[prefix.body_start..];
        let is_continuation = body
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|start| body[start] == b'&');
        let should_repeat = is_continuation || continuation;
        let mut start = prefix.body_start;
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
        rebuilt.extend_from_slice(prefix.sentinel.canonical());
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
fn fortran_code(line: &[u8]) -> &[u8] {
    &line[fortran_code_start(line)..]
}

fn fortran_code_start(line: &[u8]) -> usize {
    conditional_compilation_body_start(line).unwrap_or(0)
}

/// For every line, the nearest line above and below it that carries part of the
/// same statement.
///
/// Precomputed in two linear passes rather than searched per line: a file that
/// opens with a long comment header would otherwise make each of those lines
/// rescan the whole header, which is quadratic and cost 2s on a 40k-line file.
fn statement_neighbours(cx: &PassContext) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let lines = &cx.analysis.buffer.lines;
    let mut previous = vec![None; lines.len()];
    let mut ordinary = None;
    let mut conditional = None;
    for (index, slot) in previous.iter_mut().enumerate() {
        let nearest = match source_stream(cx, index) {
            SourceStream::Ordinary => &mut ordinary,
            SourceStream::Conditional => &mut conditional,
        };
        *slot = *nearest;
        if carries_statement(cx, index) {
            *nearest = Some(index);
        }
    }
    let mut next = vec![None; lines.len()];
    let mut ordinary = None;
    let mut conditional = None;
    for (index, slot) in next.iter_mut().enumerate().rev() {
        let nearest = match source_stream(cx, index) {
            SourceStream::Ordinary => &mut ordinary,
            SourceStream::Conditional => &mut conditional,
        };
        *slot = *nearest;
        if carries_statement(cx, index) {
            *nearest = Some(index);
        }
    }
    (previous, next)
}

fn carries_statement(cx: &PassContext, index: usize) -> bool {
    cx.analysis
        .buffer
        .lines
        .get(index)
        .is_some_and(|line| line.kind == PhysicalLineKind::Code)
}

/// Which continuation stream a physical line belongs to.
fn source_stream(cx: &PassContext, index: usize) -> SourceStream {
    if cx
        .analysis
        .buffer
        .lines
        .get(index)
        .is_some_and(|line| line.is_conditional_compilation())
    {
        SourceStream::Conditional
    } else {
        SourceStream::Ordinary
    }
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
    let mut state = LexState::default();
    state.scan_line(line, |_| {}).continued
}

fn remove_leading_continuation(line: &[u8]) -> Vec<u8> {
    let prefix = conditional_compilation_prefix(line);
    let code_start = prefix.map_or(0, |prefix| prefix.body_start);
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
    if prefix.is_some_and(|prefix| prefix.kind == ConditionalPrefixKind::CompactContinuation) {
        // Removing the `&` from `!$& foo` must leave a valid conditional
        // sentinel. Without this separator the result would be the joined
        // near-miss `!$foo`, which is an ordinary comment rather than code.
        result.extend_from_slice(b" ");
    }
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

fn openmp_body(line: &[u8]) -> Option<&[u8]> {
    openmp_directive_prefix(line).map(|prefix| &line[prefix.body_start..])
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
/// remain authored.  This runs only on reserved OpenMP directive bodies, never
/// on conditional-compilation `!$` lines.
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

    fn cx<'a>(
        document: &Document,
        local: &'a FileFacts,
        project: &'a ProjectContext,
    ) -> PassContext<'a> {
        let analysis = Box::leak(Box::new(document.analyze().unwrap()));
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

    fn normalize(
        document: &mut Document,
        local: &FileFacts,
        project: &ProjectContext,
    ) -> Result<Changed, crate::error::FormatError> {
        let context = cx(document, local, project);
        normalize_continuations(document, &context)
    }

    fn run_pass(
        document: &mut Document,
        local: &FileFacts,
        project: &ProjectContext,
    ) -> Result<Changed, crate::error::FormatError> {
        let context = cx(document, local, project);
        run(document, &context)
    }

    fn normalize_openmp(
        document: &mut Document,
        local: &FileFacts,
        project: &ProjectContext,
    ) -> Result<Changed, crate::error::FormatError> {
        let context = cx(document, local, project);
        normalize_openmp_continuation_sentinels(document, &context)
    }

    #[test]
    fn continuation_markers_are_normalized_without_touching_literals() {
        let mut document = Document::from_bytes(b"x = a &\n  & b\ny = 'a &\n &b'\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        assert_eq!(
            normalize(&mut document, &local, &project).unwrap(),
            Changed::Text
        );
        assert_eq!(document.lines[0], b"x = a &".to_vec());
        assert_eq!(document.lines[1], b"  b".to_vec());
        assert_eq!(document.lines[2], b"y = 'a &".to_vec());
        assert_eq!(document.lines[3], b" &b'".to_vec());
    }

    #[test]
    fn compact_conditional_marker_removal_keeps_a_valid_sentinel() {
        let mut document = Document::from_bytes(b"!$ call f( &\n!$& arg = 1)\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        assert_eq!(
            normalize(&mut document, &local, &project).unwrap(),
            Changed::Text
        );
        assert_eq!(document.lines[0], b"!$ call f( &".to_vec());
        assert_eq!(document.lines[1], b"!$ arg = 1)".to_vec());
    }

    #[test]
    fn compact_conditional_lexical_marker_is_not_removed() {
        let mut document = Document::from_bytes(b"!$ sub&\n!$&routine sub\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        normalize(&mut document, &local, &project).unwrap();
        assert_eq!(document.lines[0], b"!$ sub&".to_vec());
        assert_eq!(document.lines[1], b"!$&routine sub".to_vec());
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
            normalize(&mut document, &local, &project).unwrap();
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
            normalize(&mut document, &local, &project).unwrap();
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
                normalize(&mut document, &local, &project).unwrap(),
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
        normalize(&mut document, &local, &project).unwrap();
        assert_eq!(document.lines[2], b"  b".to_vec());
    }

    #[test]
    fn conditional_compilation_lines_participate_in_continuation_state() {
        for source in [
            b"!$ sub&\n!$ &routine sub\n!$ x = a &\n!$   & b\n".as_slice(),
            b"!$\tsub&\n!$\t&routine sub\n!$\tx = a &\n!$\t  & b\n",
        ] {
            let mut document = Document::from_bytes(source);
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            run_pass(&mut document, &local, &project).unwrap();

            assert!(document.lines[0].ends_with(b"sub&"));
            assert!(document.lines[1].ends_with(b"&routine sub"));
            assert!(document.lines[2].ends_with(b"x = a &"));
            assert!(document.lines[3].ends_with(b"  b"));
        }
    }

    #[test]
    fn a_split_token_is_glued_across_the_other_sentinel_stream() {
        let mut document = Document::from_bytes(b"sub&\n!$ x = 1\n&routine sub\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        normalize(&mut document, &local, &project).unwrap();

        assert_eq!(document.lines[0], b"sub&".to_vec());
        assert_eq!(document.lines[2], b"&routine sub".to_vec());
    }

    #[test]
    fn the_conditional_stream_keeps_its_own_neighbours() {
        let mut document = Document::from_bytes(b"!$ sub&\nx = 1\n!$ &routine sub\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        normalize(&mut document, &local, &project).unwrap();

        assert_eq!(document.lines[0], b"!$ sub&".to_vec());
        assert_eq!(document.lines[2], b"!$ &routine sub".to_vec());
    }

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
                normalize(&mut document, &local, &project).unwrap(),
                Changed::No,
                "literal state was lost across {sep:?}"
            );
            assert_eq!(document.lines, original);
        }
    }

    #[test]
    fn openmp_sentinels_repeat_and_macros_keep_their_case() {
        for sentinel in ["!$omp", "!$ompx"] {
            let source = format!("{sentinel} parallel do private=foo &\n{sentinel} & map(to:X)\n");
            let mut document = Document::from_bytes(source.as_bytes());
            let local = FileFacts::default();
            let mut project = ProjectContext::empty();
            project.define(&[crate::config::MacroDefine {
                name: "private".into(),
                value: None,
            }]);
            assert_ne!(
                normalize_openmp(&mut document, &local, &project).unwrap(),
                Changed::No
            );
            let canonical = if sentinel.ends_with('x') {
                b"!$OMPX ".as_slice()
            } else {
                b"!$OMP ".as_slice()
            };
            assert!(document.lines[0].starts_with(canonical));
            assert!(document.lines[1].starts_with(canonical));
            assert!(!document.lines[0]
                .windows(b"PRIVATE".len())
                .any(|w| w == b"PRIVATE"));
            let before = document.lines.clone();
            assert_eq!(
                normalize_openmp(&mut document, &local, &project).unwrap(),
                Changed::No
            );
            assert_eq!(document.lines, before);
        }
    }
}
