//! Steps 6, 7, 14: the passes that change the line count.
//!
//! Each of these must report [`Changed::Structure`], because the statement and
//! scope views are derived from line numbers and become stale the moment a line
//! appears or disappears.

use crate::{
    analysis::scope::ScopeKind,
    error::FormatError,
    source::regions::LexState,
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
        vocab,
    },
};
use std::collections::HashSet;

/// Step 6: rejoin `&` splits that cut a token in half.
///
/// A continuation may legally fall inside a name (`some_long_&\n  &name`), and
/// every later pass would see two half-tokens.  Joining them first is what makes
/// the token stream trustworthy.
///
/// Port target: `join_lexical_token_continuations`.
pub fn join_lexical_token_continuations(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = cx;
    let mut changed = false;
    let mut index = 0usize;
    let mut cpp_continuation = false;
    while index < document.lines.len() {
        if cpp_continuation || is_preprocessor_line(&document.lines[index]) {
            cpp_continuation = cpp_line_continues(&document.lines[index]);
            index += 1;
            continue;
        }

        while index + 1 < document.lines.len() {
            let Some(prefix_end) = lexical_prefix_end(&document.lines[index]) else {
                break;
            };
            let Some(suffix_start) = leading_lexical_suffix_start(&document.lines[index + 1])
            else {
                break;
            };
            let mut joined = document.lines[index][..prefix_end].to_vec();
            joined.extend_from_slice(&document.lines[index + 1][suffix_start..]);
            document.lines[index] = joined;
            document.lines.remove(index + 1);
            changed = true;
        }
        index += 1;
    }
    Ok(if changed {
        Changed::Structure
    } else {
        Changed::No
    })
}

#[derive(Clone, Copy)]
struct ParenthesisFrame {
    open: usize,
    protected: bool,
    safe: bool,
    directly_nested: bool,
    has_top_level_question: bool,
}

/// Step 7: remove redundant nested parentheses.
///
/// Eligible: a right-hand side, an `IF` condition, a `DO WHILE` condition.
/// Protected: procedure arguments and `ASSOCIATE` targets, where an extra pair
/// can change meaning or intent. A pair containing a top-level `?` is also
/// protected because Fortran 2023 conditional expressions require those
/// parentheses syntactically.
///
/// Port target: `remove_redundant_nested_parentheses`.
pub fn remove_redundant_nested_parentheses(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = cx;
    let source = document
        .lines
        .iter()
        .enumerate()
        .flat_map(|(index, line)| {
            let mut bytes = line.clone();
            if index + 1 < document.lines.len() {
                bytes.push(b'\n');
            }
            bytes
        })
        .collect::<Vec<_>>();
    if source.is_empty() {
        return Ok(Changed::No);
    }

    let protected = protected_offsets(&document.lines);
    let glue = continuation_glue(&source, &protected);
    let mut removals = HashSet::new();
    let mut stack: Vec<ParenthesisFrame> = Vec::new();
    let mut line_start = 0usize;
    let mut line_has_assignment = false;
    let mut last_non_whitespace = None;

    for index in 0..source.len() {
        let byte = source[index];
        if protected[index] {
            if byte == b'\n' {
                line_start = index + 1;
                line_has_assignment = false;
                last_non_whitespace = None;
            }
            continue;
        }
        // A continuation marker — and the newline it holds open — is punctuation
        // of the source layout, not of the expression.  The parentheses of a
        // statement nest across it, so the scan looks straight through: neither
        // `directly_nested` nor the search for the matching `)` may be defeated
        // by where the author chose to break the line.
        if glue[index] {
            continue;
        }
        if byte == b'=' {
            let previous = index.checked_sub(1).and_then(|i| source.get(i));
            let following = source.get(index + 1).copied();
            if !matches!(previous, Some(b'<' | b'>' | b'=' | b'/'))
                && !matches!(following, Some(b'=' | b'>'))
            {
                line_has_assignment = true;
            }
        }
        match byte {
            b'(' => {
                let preceding = preceding_identifier(&source, index, 0);
                let condition = preceding_identifier(&source, index, line_start);
                let is_argument_list = preceding.is_some_and(|name| {
                    !vocab::contains(vocab::FORTRAN_KEYWORDS, name)
                        && !vocab::contains(vocab::FORTRAN_SPECIFIERS, name)
                });
                let is_condition = condition.is_some_and(|name| {
                    name.eq_ignore_ascii_case(b"if")
                        || name.eq_ignore_ascii_case(b"elseif")
                        || name.eq_ignore_ascii_case(b"while")
                });
                let (parent_protected, parent_safe, directly_nested) = stack
                    .last()
                    .map(|parent| {
                        (
                            parent.protected,
                            parent.safe,
                            last_non_whitespace == Some(parent.open),
                        )
                    })
                    .unwrap_or((false, false, false));
                stack.push(ParenthesisFrame {
                    open: index,
                    protected: is_argument_list || parent_protected,
                    safe: line_has_assignment || is_condition || parent_safe,
                    directly_nested,
                    has_top_level_question: false,
                });
            }
            b'?' => {
                // The current frame is exactly the parenthesis depth at which
                // the marker occurs. Nested calls/subscripts have their own
                // frames, so this records a top-level conditional marker for
                // this pair without needing to parse the rest of the F2023
                // conditional-expression grammar yet.
                if let Some(frame) = stack.last_mut() {
                    frame.has_top_level_question = true;
                }
            }
            b')' => {
                if let Some(inner) = stack.pop() {
                    if let Some(parent) = stack.last() {
                        if parent.safe
                            && !parent.protected
                            && inner.directly_nested
                            && !inner.has_top_level_question
                        {
                            let mut following = index + 1;
                            while source.get(following).is_some_and(u8::is_ascii_whitespace)
                                || glue.get(following) == Some(&true)
                            {
                                following += 1;
                            }
                            if source.get(following) == Some(&b')') {
                                removals.insert(inner.open);
                                removals.insert(index);
                            }
                        }
                    }
                }
            }
            b'\n' => {
                line_start = index + 1;
                line_has_assignment = false;
            }
            _ => {}
        }
        if !byte.is_ascii_whitespace() {
            last_non_whitespace = Some(index);
        }
    }
    if removals.is_empty() {
        return Ok(Changed::No);
    }

    let mut rebuilt = Vec::with_capacity(source.len() - removals.len());
    for (index, byte) in source.into_iter().enumerate() {
        if !removals.contains(&index) {
            rebuilt.push(byte);
        }
    }
    let mut lines = rebuilt
        .split(|byte| *byte == b'\n')
        .map(Vec::from)
        .collect::<Vec<_>>();
    if lines.len() > document.lines.len() {
        lines.pop();
    }
    document.set_lines(lines);
    Ok(Changed::Text)
}

/// Step 14: drop a bare terminal `RETURN`.
///
/// Only when it is the final single-line statement before a procedure's `END`,
/// and never when it carries an inline comment.
///
/// Port target: `remove_terminal_procedure_returns`.
pub fn remove_terminal_procedure_returns(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let mut remove = HashSet::new();
    for scope in &cx.scopes.scopes {
        if scope.kind != ScopeKind::Procedure || scope.lines.end <= scope.lines.start + 1 {
            continue;
        }
        let end_line = scope.lines.end - 1;
        let Some(end_group) = cx
            .analysis
            .groups
            .iter()
            .find(|group| group.lines.end == scope.lines.end)
        else {
            continue;
        };
        if !end_group.statements.iter().any(|statement| {
            matches!(
                crate::classify::classify(&statement.text).class,
                crate::classify::StatementClass::EndDefinition
            )
        }) {
            continue;
        }
        let mut previous = None;
        for group in &cx.analysis.groups {
            if group.lines.end > end_line {
                break;
            }
            for statement in &group.statements {
                if !statement.text.iter().all(u8::is_ascii_whitespace) {
                    previous = Some((group, statement));
                }
            }
        }
        let Some((group, statement)) = previous else {
            continue;
        };
        if statement.text.eq_ignore_ascii_case(b"return")
            && group.lines.end == group.lines.start + 1
            && group.lines.start > scope.lines.start
            && group.lines.end <= end_line
            && document.lines.get(group.lines.start).is_some_and(|line| {
                let code = crate::source::regions::comment_start(line).unwrap_or(line.len());
                line[..code].trim_ascii().eq_ignore_ascii_case(b"return")
                    && crate::source::regions::comment_start(line).is_none()
            })
        {
            remove.insert(group.lines.start);
        }
    }
    if remove.is_empty() {
        return Ok(Changed::No);
    }
    document.set_lines(
        document
            .lines
            .iter()
            .enumerate()
            .filter(|(index, _)| !remove.contains(index))
            .map(|(_, line)| line.clone())
            .collect(),
    );
    Ok(Changed::Structure)
}

pub(crate) fn is_preprocessor_line(line: &[u8]) -> bool {
    line.iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .is_some_and(|index| line[index] == b'#' || line[index..].starts_with(b"??"))
}

pub(crate) fn cpp_line_continues(line: &[u8]) -> bool {
    let mut end = line.len();
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    line.get(..end).is_some_and(|line| line.ends_with(b"\\"))
}

fn lexical_prefix_end(line: &[u8]) -> Option<usize> {
    if is_preprocessor_line(line) || crate::source::regions::comment_start(line).is_some() {
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
    if line.get(..end).is_some_and(|line| line.ends_with(b"&")) {
        let prefix_end = end - 1;
        (prefix_end > 0
            && (line[prefix_end - 1].is_ascii_alphanumeric() || line[prefix_end - 1] == b'_'))
            .then_some(prefix_end)
    } else {
        None
    }
}

fn leading_lexical_suffix_start(line: &[u8]) -> Option<usize> {
    if is_preprocessor_line(line) {
        return None;
    }
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

fn preceding_identifier(source: &[u8], index: usize, start: usize) -> Option<&[u8]> {
    let mut end = index;
    while end > start && source[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut begin = end;
    while begin > start && (source[begin - 1].is_ascii_alphanumeric() || source[begin - 1] == b'_')
    {
        begin -= 1;
    }
    (begin != end).then_some(&source[begin..end])
}

/// Mark the bytes that only hold a statement across a line break.
///
/// Those are the trailing `&` that continues a line, the optional `&` that
/// reopens it on the next, and the newline between them.  A blank or
/// comment-only line inside a continued statement keeps the statement open
/// without ending it, so it neither starts nor stops the run.  Marks inside a
/// string or a comment are not markers at all and are left alone.
fn continuation_glue(source: &[u8], protected: &[bool]) -> Vec<bool> {
    let mut glue = vec![false; source.len()];
    let mut continued = false;
    let mut start = 0usize;
    while start <= source.len() {
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |offset| start + offset);
        let mut code = (start..end)
            .filter(|index| !protected[*index] && !source[*index].is_ascii_whitespace());
        let first = code.next();
        let last = code.next_back().or(first);
        if let (true, Some(first)) = (continued, first) {
            if source[first] == b'&' {
                glue[first] = true;
            }
        }
        if let Some(last) = last {
            if source[last] == b'&' {
                glue[last] = true;
                if end < source.len() {
                    glue[end] = true;
                }
                continued = true;
            } else {
                continued = false;
            }
        }
        start = end + 1;
    }
    glue
}

fn protected_offsets(lines: &[Vec<u8>]) -> Vec<bool> {
    let total = lines.iter().map(Vec::len).sum::<usize>() + lines.len().saturating_sub(1);
    let mut protected = vec![false; total];
    let mut offset = 0;
    let mut state = LexState::default();
    for (index, line) in lines.iter().enumerate() {
        let mut regions = Vec::new();
        state.scan(line, |region| regions.push(region));
        for region in regions {
            if !matches!(region.kind, crate::source::RegionKind::Code) {
                for byte in region.range {
                    protected[offset + byte] = true;
                }
            }
        }
        offset += line.len();
        if index + 1 < lines.len() {
            offset += 1;
        }
    }
    protected
}

#[cfg(test)]
mod tests {
    use super::{
        join_lexical_token_continuations, remove_redundant_nested_parentheses,
        remove_terminal_procedure_returns,
    };
    use crate::{
        analysis::ProjectContext,
        config::FormatConfig,
        transform::{document::Document, pipeline},
    };

    fn context(
        document: &Document,
    ) -> (
        crate::analysis::FileFacts,
        crate::analysis::ScopeTree,
        crate::transform::document::Analysis,
    ) {
        let analysis = document.analyze().unwrap();
        let scopes = crate::analysis::ScopeTree::build(&analysis);
        (crate::analysis::FileFacts::default(), scopes, analysis)
    }

    #[test]
    fn lexical_join_is_structural_and_idempotent() {
        let mut document = Document::from_bytes(b"program p\nlong_na&\n  &me = 1\nend program\n");
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            join_lexical_token_continuations(&mut document, &cx).unwrap(),
            pipeline::Changed::Structure
        );
        assert_eq!(
            document.lines,
            vec![
                b"program p".to_vec(),
                b"long_name = 1".to_vec(),
                b"end program".to_vec()
            ]
        );
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            join_lexical_token_continuations(&mut document, &cx).unwrap(),
            pipeline::Changed::No
        );
    }

    #[test]
    fn nested_parentheses_obey_expression_and_protection_rules() {
        let mut document = Document::from_bytes(
            b"x = ((a + b))\nif ((x)) then\ncall f(((x)))\nassociate (a => ((x)))\n",
        );
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            remove_redundant_nested_parentheses(&mut document, &cx).unwrap(),
            pipeline::Changed::Text
        );
        assert_eq!(document.lines[0], b"x = (a + b)".to_vec());
        assert_eq!(document.lines[1], b"if (x) then".to_vec());
        assert_eq!(document.lines[2], b"call f(((x)))".to_vec());
        assert_eq!(document.lines[3], b"associate (a => ((x)))".to_vec());
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            remove_redundant_nested_parentheses(&mut document, &cx).unwrap(),
            pipeline::Changed::No
        );
    }

    #[test]
    fn conditional_expression_parentheses_are_not_removed() {
        let mut document = Document::from_bytes(
            b"if ((flag ? a : b)) then\nif (((flag ? a : b))) then\nif ((flag .and. '?' == '?')) then\n",
        );
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            remove_redundant_nested_parentheses(&mut document, &cx).unwrap(),
            pipeline::Changed::Text
        );
        assert_eq!(document.lines[0], b"if ((flag ? a : b)) then".to_vec());
        assert_eq!(document.lines[1], b"if ((flag ? a : b)) then".to_vec());
        assert_eq!(document.lines[2], b"if (flag .and. '?' == '?') then".to_vec());

        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            remove_redundant_nested_parentheses(&mut document, &cx).unwrap(),
            pipeline::Changed::No
        );
    }

    #[test]
    fn terminal_return_requires_a_bare_final_line_and_is_idempotent() {
        let mut document = Document::from_bytes(b"subroutine s\nreturn\nend subroutine s\nsubroutine t\nreturn ! keep\nend subroutine t\n");
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            remove_terminal_procedure_returns(&mut document, &cx).unwrap(),
            pipeline::Changed::Structure
        );
        assert!(!document.lines.iter().any(|line| line == b"return"));
        let (local, scopes, analysis) = context(&document);
        let cx = pipeline::PassContext {
            config: &FormatConfig::default(),
            project: &ProjectContext::empty(),
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_eq!(
            remove_terminal_procedure_returns(&mut document, &cx).unwrap(),
            pipeline::Changed::No
        );
        assert!(document
            .lines
            .iter()
            .any(|line| line.starts_with(b"return !")));
    }
}
