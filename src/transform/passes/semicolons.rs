//! Canonicalize redundant free-form statement separators.
//!
//! A semicolon is meaningful only when it separates two non-empty statements.
//! Leading/trailing semicolons and extra semicolons in a separator run are
//! therefore redundant. This pass uses the logical-group provenance rather than
//! physical-line heuristics, so a separator that happens to sit next to a
//! continuation marker is kept when it separates statements across that
//! continuation. Semicolons inside protected literals and Hollerith payloads
//! remain part of their statement spans and are never considered separators.

use crate::{
    error::FormatError,
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
    },
};

fn has_statement_content(statement: &[u8]) -> bool {
    statement
        .iter()
        .any(|byte| !byte.is_ascii_whitespace() && *byte != b'&')
}

/// Remove semicolons that delimit no statement, keeping exactly one separator
/// between every adjacent pair of non-empty logical statements.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let source: &[u8] = cx.analysis.buffer.bytes.as_ref();
    let mut removals = vec![Vec::<usize>::new(); document.lines.len()];

    for group in &cx.analysis.groups {
        if group.pieces.is_empty() || group.statements.iter().any(|statement| statement.is_fix) {
            continue;
        }

        // A bare continuation marker is syntax, not a statement. It can appear
        // as its own range when the other source stream physically interrupts a
        // continued statement. Step 12 removes that marker later, so counting
        // it here would make this pass see a different separator on the next run.
        let statement_ranges = group
            .statements
            .iter()
            .filter(|statement| has_statement_content(&statement.text))
            .map(|statement| statement.offset..statement.offset + statement.text.len())
            .collect::<Vec<_>>();
        let mut separators = Vec::new();

        for piece in &group.pieces {
            let bytes = &source[piece.bytes.start as usize..piece.bytes.end as usize];
            for (piece_offset, byte) in bytes.iter().enumerate() {
                if *byte != b';' {
                    continue;
                }

                let joined_offset = piece.text.start + piece_offset;
                if statement_ranges
                    .iter()
                    .any(|range| range.contains(&joined_offset))
                {
                    continue;
                }

                let absolute = piece.bytes.start as usize + piece_offset;
                let line_start = cx.analysis.buffer.lines[piece.line].span.start as usize;
                separators.push((joined_offset, piece.line, absolute - line_start));
            }
        }

        let mut kept_internal = vec![false; statement_ranges.len().saturating_sub(1)];
        for (joined_offset, line, line_offset) in separators {
            let internal_gap = statement_ranges
                .windows(2)
                .position(|pair| pair[0].end <= joined_offset && joined_offset < pair[1].start);
            match internal_gap {
                Some(gap) if !kept_internal[gap] => {
                    kept_internal[gap] = true;
                    continue;
                }
                _ => removals[line].push(line_offset),
            }
        }
    }

    let mut changed = false;
    for (line, positions) in document.lines.iter_mut().zip(&mut removals) {
        if positions.is_empty() {
            continue;
        }

        positions.sort_unstable();
        positions.dedup();
        let capacity = line.len().saturating_sub(positions.len());
        let mut remove_iter = positions.iter().copied().peekable();
        let mut rebuilt = Vec::with_capacity(capacity);
        for (index, byte) in line.iter().enumerate() {
            if remove_iter.peek().copied() == Some(index) {
                remove_iter.next();
            } else {
                rebuilt.push(*byte);
            }
        }
        *line = rebuilt;
        changed = true;
    }

    if changed && document.canonicalize_empty_unterminated_tail() {
        return Ok(Changed::Structure);
    }
    Ok(if changed { Changed::Text } else { Changed::No })
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::{
        analysis::{FileFacts, ProjectContext, ScopeTree},
        config::FormatConfig,
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
    };

    fn normalize(source: &[u8]) -> Vec<u8> {
        let mut document = Document::from_bytes(source);
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        let config = FormatConfig::default();
        let cx = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert!(matches!(run(&mut document, &cx).unwrap(), Changed::Text));
        document.to_bytes()
    }

    #[test]
    fn drops_empty_leading_trailing_and_repeated_separators() {
        assert_eq!(
            normalize(b";;call a();;; call b();; ! tail\n"),
            b"call a(); call b() ! tail\n"
        );
    }

    #[test]
    fn preserves_protected_semicolons_and_one_real_separator() {
        assert_eq!(
            normalize(b"x = 'a;b';; y = 3H;!x;; ! tail\n"),
            b"x = 'a;b'; y = 3H;!x ! tail\n"
        );
    }

    #[test]
    fn keeps_a_separator_that_crosses_a_continuation() {
        assert_eq!(
            normalize(b"call a() &\n& ; call b();\n"),
            b"call a() &\n& ; call b()\n"
        );
    }

    #[test]
    fn final_unterminated_separator_keeps_document_and_analysis_in_sync() {
        let mut document = Document::from_bytes(b"\n;");
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let local = FileFacts::default();
        let project = ProjectContext::empty();
        let config = FormatConfig::default();
        let cx = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };

        assert_eq!(run(&mut document, &cx).unwrap(), Changed::Structure);
        assert_eq!(document.to_bytes(), b"\n");
        assert_eq!(
            document.lines.len(),
            document.analyze().unwrap().buffer.lines.len()
        );
    }

    #[test]
    fn leaves_preprocessor_and_findentfix_text_untouched() {
        assert_eq!(
            normalize(b"#define S \";;;\"\n! findentfix: call a();;\ncall b();\n"),
            b"#define S \";;;\"\n! findentfix: call a();;\ncall b()\n"
        );
    }
}
