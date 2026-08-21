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
///
/// Directive *casing* is deliberately not here: see
/// [`case_openmp_directives`], which the pipeline runs separately because it is
/// canonicalization rather than whitespace policy.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let changed = normalize_continuations(document, cx)?;
    Ok(changed.or(normalize_openmp_continuation_sentinels(document, cx)?))
}

/// Spell reserved OpenMP directives — the sentinel word and the directive words
/// after it — according to `--openmp-case`, touching nothing else on the line.
///
/// This is separate from the sentinel *shape* normalization in
/// [`normalize_openmp_continuation_sentinels`] because the two answer different
/// questions and are reached by different modes. Repeating a sentinel across a
/// continuation, removing a body-leading `&` and inserting the canonical blank
/// after the sentinel are all presentation: they belong to whitespace and
/// continuation-marker policy, and canonicalize-only does not run them. How a
/// reserved word is *spelled* is not presentation — it is exactly what
/// canonicalization means — so it runs in every normalizing mode, and it does
/// not stop because `--continuation-markers=false` turned the other pass off.
///
/// `!$ ` conditional-compilation lines are ordinary Fortran and are not reached
/// here at all: `openmp_directive_prefix` answers `None` for them, so their
/// keywords stay with `--keyword-case` like any other statement's.
pub fn case_openmp_directives(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let mut changed = Changed::No;
    let mut updated = document.lines.clone();
    for line in &mut updated {
        let Some(prefix) = openmp_directive_prefix(line) else {
            continue;
        };
        let indent_end = line
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(0);
        let mut rebuilt = line[..indent_end].to_vec();
        rebuilt.extend_from_slice(&crate::transform::passes::line_rules::apply_case(
            &line[indent_end..prefix.sentinel_end],
            cx.config.style.openmp_keyword_case(),
        ));
        // From the sentinel's end rather than the body's start, so the blank
        // between them is carried through untouched instead of canonicalized.
        rebuilt.extend_from_slice(&case_openmp_body(&line[prefix.sentinel_end..], cx));
        if rebuilt != *line {
            *line = rebuilt;
            changed = changed.or(Changed::Text);
        }
    }
    if changed != Changed::No {
        document.set_lines(updated);
    }
    Ok(changed)
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
        let stream = cx.analysis.buffer.stream(index);
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
/// two config fields, never one flag. `--openmp-case` is the third: it decides
/// the *spelling* of the sentinel and the directive words, and reaches nothing
/// else on the line.
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
        // The sentinel word keeps whatever spelling `case_openmp_directives`
        // gave it; only the separating blank after it is this pass's to
        // canonicalize.
        rebuilt.extend_from_slice(&current[indent_end..prefix.sentinel_end]);
        rebuilt.push(b' ');
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
        let nearest = match cx.analysis.buffer.stream(index) {
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
        let nearest = match cx.analysis.buffer.stream(index) {
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

/// Does the clause opening at `rest` use the modifier syntax — one or more
/// modifiers separated from the kind by a top-level `:`?
///
/// `schedule([modifier[, modifier]:] kind[, chunk_size])` spends a comma on two
/// jobs, and only the colon tells them apart: in
/// `schedule(monotonic, simd: static, n)` the first comma separates two
/// modifiers and the second hands over to an expression. Answering that needs
/// one look ahead, because the words come out as they are read.
///
/// `rest` is the remainder of a single code region, so a `:` inside a string
/// literal cannot reach here. A clause that runs past the end of the region
/// simply reports no colon, which casts fewer words rather than more.
fn has_modifier_colon(rest: &[u8]) -> bool {
    let mut depth = 0usize;
    for &byte in rest {
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => return false,
            b')' => depth -= 1,
            b':' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// Spell the recognized OpenMP directive words in `body` according to
/// [`StyleConfig::openmp_keyword_case`].
///
/// Directive words get their own switch rather than following `--keyword-case`
/// because uppercase directives over lowercase Fortran is a convention in its
/// own right; `--openmp-case=false` hands them back to `--keyword-case`, which
/// is how `preserve` reaches them.
///
/// What is *not* re-cased is the point of the two tables. A clause's arguments
/// are the user's own program — `PRIVATE(i)` names a variable — so matching a
/// word against one flat keyword table re-spells any identifier that happens to
/// collide with the OpenMP vocabulary, and that vocabulary contains ordinary
/// words: `shared`, `static`, `final`, `order`, `device`, `hint`. Legal Fortran
/// such as `!$omp parallel private(shared)` came out as `PRIVATE(SHARED)`,
/// renaming the user's variable in the source text. Fortran's own
/// case-insensitivity makes that harmless to run, but it is still the formatter
/// rewriting a name it does not own.
///
/// So position decides. At the top level of the directive a word can only be a
/// directive or clause name, and [`DIRECTIVE_WORDS`] applies. Inside a clause's
/// parentheses the default is that the word is the user's, and the exceptions
/// are listed one clause at a time in [`CLAUSE_KINDS`]: the handful of clauses
/// whose argument grammar is a fixed vocabulary rather than a list of names --
/// and within those, only up to the clause's first top-level comma, because
/// every one of them spends its vocabulary on the kind and modifiers and leaves
/// the rest to an expression. `SCHEDULE(STATIC, chunk)` is cased because
/// `schedule` takes a kind; the chunk size beside it is not, even when it is
/// spelled `schedule(dynamic, static)`. Nothing in `MAP(to: a)` is cased
/// either, because `map` takes a list and telling its modifier from the list
/// would take the clause-by-clause grammar of the whole OpenMP specification.
///
/// [`StyleConfig::openmp_keyword_case`]: crate::config::StyleConfig::openmp_keyword_case
fn case_openmp_body(body: &[u8], cx: &PassContext) -> Vec<u8> {
    let mut result = Vec::with_capacity(body.len());
    let mut state = LexState::default();
    let mut regions = Vec::new();
    state.scan(body, |region| regions.push(region));
    // Depth counts only delimiters the code regions report, so a parenthesis
    // inside a string literal or a comment cannot open a clause.
    let mut depth = 0usize;
    // The word immediately before the `(` that took the depth from zero to one,
    // resolved to the reserved argument spellings that clause admits.
    let mut clause_kinds: Option<&[&[u8]]> = None;
    let mut clause_name: Vec<u8> = Vec::new();
    // Is the clause's modifier colon still ahead? While it is, a top-level
    // comma separates two modifiers rather than the kind from its chunk size.
    let mut before_modifier_colon = false;
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
                // Depth two and beyond is inside an expression — `if(f(x))`,
                // `schedule(static, n(1))` — where every word is the user's.
                let reserved = match depth {
                    0 => Some(DIRECTIVE_WORDS),
                    1 => clause_kinds,
                    _ => None,
                };
                // A macro name outranks every case rule (I4).
                if !cx.project.macros.contains(word)
                    && reserved.is_some_and(|reserved| {
                        reserved
                            .iter()
                            .any(|keyword| word.eq_ignore_ascii_case(keyword))
                    })
                {
                    result.extend_from_slice(&crate::transform::passes::line_rules::apply_case(
                        word,
                        cx.config.style.openmp_keyword_case(),
                    ));
                } else {
                    result.extend_from_slice(word);
                }
                if depth == 0 {
                    clause_name = word.to_vec();
                }
                continue;
            }
            match bytes[index] {
                b'(' => {
                    if depth == 0 {
                        clause_kinds = CLAUSE_KINDS
                            .iter()
                            .find(|(clause, _)| clause_name.eq_ignore_ascii_case(clause))
                            .map(|(_, kinds)| *kinds);
                        before_modifier_colon =
                            clause_kinds.is_some() && has_modifier_colon(&bytes[index + 1..]);
                    }
                    depth += 1;
                }
                b':' if depth == 1 => before_modifier_colon = false,
                // The kind and its modifiers are a fixed vocabulary; what
                // follows them is an expression the user wrote, and
                // `schedule(dynamic, static)` names a chunk-size variable that
                // re-casing would rename. Which side of that line a comma falls
                // on is what `before_modifier_colon` answers.
                b',' if depth == 1 && !before_modifier_colon => clause_kinds = None,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        clause_kinds = None;
                        clause_name.clear();
                        before_modifier_colon = false;
                    }
                }
                _ => {}
            }
            result.push(bytes[index]);
            index += 1;
        }
    }
    result
}

/// Words that name a directive or one of its clauses, recognized only at the
/// top level of a directive line. Kind words that exist solely as a clause
/// argument — `static`, `guided` — are deliberately absent: they live in
/// [`CLAUSE_KINDS`] under the clause that admits them.
const DIRECTIVE_WORDS: &[&[u8]] = &[
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

/// Clauses whose arguments are a fixed vocabulary rather than a list of the
/// user's names, with the spellings each one admits.
///
/// Membership is the promise that no *kind* argument of that clause is ever a
/// program identifier, so casing a word there cannot rename anything. It does
/// not extend past the clause's first top-level comma: `schedule` and
/// `dist_schedule` take a chunk-size expression after theirs, and an expression
/// is the user's, so `schedule(dynamic, static)` keeps its `static`. Clauses
/// that take a list — `private`, `shared`, `map`, `depend`, `reduction`,
/// `linear` — are absent however reserved-looking their modifiers are, because
/// a list is exactly where the user's own names appear.
const CLAUSE_KINDS: &[(&[u8], &[&[u8]])] = &[
    (
        b"default",
        &[b"none", b"shared", b"private", b"firstprivate"],
    ),
    (b"proc_bind", &[b"master", b"primary", b"close", b"spread"]),
    (
        b"schedule",
        &[
            b"static",
            b"dynamic",
            b"guided",
            b"auto",
            b"runtime",
            b"monotonic",
            b"nonmonotonic",
            b"simd",
        ],
    ),
    (b"dist_schedule", &[b"static"]),
    (
        b"order",
        &[b"concurrent", b"reproducible", b"unconstrained"],
    ),
];

fn normalize_openmp_body(body: &[u8], cx: &PassContext) -> Vec<u8> {
    // Casing is [`case_openmp_directives`]'s, and it has already run.
    let spaced = crate::transform::passes::line_rules::normalize_delimiter_spacing(body, cx);
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
    use super::{
        case_openmp_directives, normalize_continuations, normalize_openmp_continuation_sentinels,
        run,
    };
    use crate::{
        analysis::{FileFacts, ProjectContext, ScopeTree},
        config::{FormatConfig, FormatMode, KeywordCase},
        transform::document::Document,
        transform::pipeline::{Changed, PassContext},
    };

    fn cx<'a>(
        document: &Document,
        local: &'a FileFacts,
        project: &'a ProjectContext,
    ) -> PassContext<'a> {
        cx_cased(document, local, project, KeywordCase::Lower, true)
    }

    fn cx_cased<'a>(
        document: &Document,
        local: &'a FileFacts,
        project: &'a ProjectContext,
        keyword_case: KeywordCase,
        openmp_case: bool,
    ) -> PassContext<'a> {
        let analysis = Box::leak(Box::new(document.analyze().unwrap()));
        let scopes = Box::leak(Box::new(ScopeTree::build(analysis)));
        let style = crate::config::StyleConfig {
            keyword_case,
            openmp_case,
            ..crate::config::StyleConfig::default()
        };
        let config = Box::leak(Box::new(FormatConfig {
            mode: FormatMode::Full,
            style,
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
        normalize_openmp_cased(document, local, project, KeywordCase::Lower, true)
    }

    fn normalize_openmp_cased(
        document: &mut Document,
        local: &FileFacts,
        project: &ProjectContext,
        keyword_case: KeywordCase,
        openmp_case: bool,
    ) -> Result<Changed, crate::error::FormatError> {
        let context = cx_cased(document, local, project, keyword_case, openmp_case);
        // The pipeline's order: spelling first, then sentinel shape. These
        // tests assert what the two produce together, which is what a caller
        // in full mode sees.
        let cased = case_openmp_directives(document, &context)?;
        Ok(cased.or(normalize_openmp_continuation_sentinels(document, &context)?))
    }

    /// The two halves are separate passes because separate modes reach them, so
    /// neither may quietly do the other's job.
    #[test]
    fn the_sentinel_shape_pass_does_not_case_and_the_case_pass_does_not_respace() {
        let source = b"!$omp Parallel Do   private(i) &\n!$omp & map(to:X)\n";
        let local = FileFacts::default();
        let project = ProjectContext::empty();

        let mut shape = Document::from_bytes(source);
        let context = cx_cased(&shape, &local, &project, KeywordCase::Lower, true);
        normalize_openmp_continuation_sentinels(&mut shape, &context).unwrap();
        assert_eq!(
            String::from_utf8(shape.to_bytes()).unwrap(),
            "!$omp Parallel Do   private(i) &\n!$omp map(to:X)\n",
        );

        let mut cased = Document::from_bytes(source);
        let context = cx_cased(&cased, &local, &project, KeywordCase::Lower, true);
        case_openmp_directives(&mut cased, &context).unwrap();
        assert_eq!(
            String::from_utf8(cased.to_bytes()).unwrap(),
            "!$OMP PARALLEL DO   PRIVATE(i) &\n!$OMP & MAP(to:X)\n",
        );
    }

    /// A clause's arguments are the user's program, and the OpenMP vocabulary
    /// is full of ordinary words, so a flat keyword table re-spells declared
    /// names: `private(shared)` came out `PRIVATE(SHARED)`. Fortran is
    /// case-insensitive so it still ran, but the formatter had rewritten a name
    /// it does not own, against its own documented contract.
    #[test]
    fn a_clause_argument_spelled_like_a_keyword_keeps_the_authored_case() {
        let mut document = Document::from_bytes(
            b"!$omp parallel private(shared) firstprivate(static) if(final)\n",
        );
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        let context = cx_cased(&document, &local, &project, KeywordCase::Lower, true);
        case_openmp_directives(&mut document, &context).unwrap();
        assert_eq!(
            String::from_utf8(document.to_bytes()).unwrap(),
            "!$OMP PARALLEL PRIVATE(shared) FIRSTPRIVATE(static) IF(final)\n",
        );
    }

    /// The exception, and why it is safe: these clauses take a fixed kind
    /// vocabulary, so no argument of theirs is ever a declared name. The
    /// chunk-size expression beside a kind still is one.
    #[test]
    fn a_kind_clause_cases_its_reserved_argument_but_not_the_expression() {
        let mut document = Document::from_bytes(
            b"!$omp do schedule(dynamic, chunk) default(none) proc_bind(close) order(concurrent)\n",
        );
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        let context = cx_cased(&document, &local, &project, KeywordCase::Lower, true);
        case_openmp_directives(&mut document, &context).unwrap();
        assert_eq!(
            String::from_utf8(document.to_bytes()).unwrap(),
            "!$OMP DO SCHEDULE(DYNAMIC, chunk) DEFAULT(NONE) PROC_BIND(CLOSE) ORDER(CONCURRENT)\n",
        );
    }

    /// A kind clause's vocabulary stops at its first top-level comma. What
    /// follows is a chunk-size expression, so a variable named after a
    /// schedule kind is still the user's name — the same defect as the flat
    /// table, one clause further in.
    #[test]
    fn a_chunk_size_spelled_like_a_schedule_kind_keeps_the_authored_case() {
        let mut document = Document::from_bytes(
            b"!$omp do schedule(dynamic, static) dist_schedule(static, guided)
",
        );
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        let context = cx_cased(&document, &local, &project, KeywordCase::Lower, true);
        case_openmp_directives(&mut document, &context).unwrap();
        assert_eq!(
            String::from_utf8(document.to_bytes()).unwrap(),
            "!$OMP DO SCHEDULE(DYNAMIC, static) DIST_SCHEDULE(STATIC, guided)\n",
        );
    }

    /// The modifier syntax puts a colon, not a comma, between the modifiers and
    /// the kind, so everything up to the colon is still reserved — including
    /// the comma between two modifiers, which is the same character doing a
    /// different job from the one before a chunk size.
    #[test]
    fn schedule_modifiers_before_the_kind_are_still_cased() {
        for (source, expected) in [
            (
                &b"!$omp do schedule(monotonic: static, dynamic)\n"[..],
                "!$OMP DO SCHEDULE(MONOTONIC: STATIC, dynamic)\n",
            ),
            (
                &b"!$omp do schedule(monotonic, simd: static, dynamic)\n"[..],
                "!$OMP DO SCHEDULE(MONOTONIC, SIMD: STATIC, dynamic)\n",
            ),
        ] {
            let mut document = Document::from_bytes(source);
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            let context = cx_cased(&document, &local, &project, KeywordCase::Lower, true);
            case_openmp_directives(&mut document, &context).unwrap();
            assert_eq!(String::from_utf8(document.to_bytes()).unwrap(), expected);
        }
    }

    /// Nesting is where a kind clause stops applying: depth two is an
    /// expression, and `default` there is the intrinsic's argument, not a
    /// clause kind.
    #[test]
    fn a_nested_expression_inside_a_kind_clause_is_left_alone() {
        let mut document =
            Document::from_bytes(b"!$omp do schedule(static, size(shared)) if(f(none))\n");
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        let context = cx_cased(&document, &local, &project, KeywordCase::Lower, true);
        case_openmp_directives(&mut document, &context).unwrap();
        assert_eq!(
            String::from_utf8(document.to_bytes()).unwrap(),
            "!$OMP DO SCHEDULE(STATIC, size(shared)) IF(f(none))\n",
        );
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
            // `private` is a `-D` macro name here, and macro names outrank
            // every case rule (I4) including the OpenMP one.
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

    /// `openmp_case` is the near-universal "uppercase directives over lowercase
    /// Fortran" convention, so it holds the sentinel and the directive words at
    /// upper case whatever `--keyword-case` says.
    #[test]
    fn openmp_directives_are_uppercase_whatever_the_keyword_case_is() {
        for case in [
            KeywordCase::Lower,
            KeywordCase::Upper,
            KeywordCase::Preserve,
        ] {
            let source = b"!$omp Parallel Do private(i)\n!$OMP END parallel do\n";
            let mut document = Document::from_bytes(source);
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            normalize_openmp_cased(&mut document, &local, &project, case, true).unwrap();
            assert_eq!(
                String::from_utf8(document.to_bytes()).unwrap(),
                "!$OMP PARALLEL DO PRIVATE(i)\n!$OMP END PARALLEL DO\n",
                "{case:?}"
            );

            // The spelling this pass settles on has to be a fixed point of it.
            let before = document.lines.clone();
            normalize_openmp_cased(&mut document, &local, &project, case, true).unwrap();
            assert_eq!(document.lines, before, "{case:?}");
        }
    }

    /// Turning `openmp_case` off hands directive words back to `--keyword-case`
    /// like any other keyword. `preserve` in particular then has to leave the
    /// authored spelling of both the sentinel and the directive words alone.
    #[test]
    fn openmp_directive_words_follow_the_keyword_case_setting() {
        for (case, expected) in [
            (
                KeywordCase::Lower,
                "!$omp parallel do private(i)\n!$omp end parallel do\n",
            ),
            (
                KeywordCase::Upper,
                "!$OMP PARALLEL DO PRIVATE(i)\n!$OMP END PARALLEL DO\n",
            ),
            (
                KeywordCase::Preserve,
                "!$omp Parallel Do private(i)\n!$OMP END parallel do\n",
            ),
        ] {
            let source = b"!$omp Parallel Do private(i)\n!$OMP END parallel do\n";
            let mut document = Document::from_bytes(source);
            let local = FileFacts::default();
            let project = ProjectContext::empty();
            normalize_openmp_cased(&mut document, &local, &project, case, false).unwrap();
            assert_eq!(
                String::from_utf8(document.to_bytes()).unwrap(),
                expected,
                "{case:?}"
            );

            // The spelling this pass settles on has to be a fixed point of it.
            let before = document.lines.clone();
            normalize_openmp_cased(&mut document, &local, &project, case, false).unwrap();
            assert_eq!(document.lines, before, "{case:?}");
        }
    }
}
