//! Early canonicalization of contextual compact conditional continuations.
//!
//! `SourceBuffer` is the semantic authority: `!$&...` belongs to the
//! conditional-compilation stream only when that stream already has an open
//! continuation. This pass merely rewrites those proven compact spellings to
//! the stable `!$ &...` form before the normalization pipeline's first analysis.
//! The ordinary continuation pass still decides whether the body-leading `&` is
//! redundant or lexically required.

use crate::{
    error::FormatError,
    source::{
        syntax::{conditional_compilation_prefix, ConditionalPrefixKind},
        SourceBuffer,
    },
    transform::{document::Document, pipeline::Changed},
};

pub fn run(document: &mut Document) -> Result<Changed, FormatError> {
    let original = document.lines.clone();
    let source = document.to_lf_bytes();
    let buffer = SourceBuffer::new(&source)?;
    let mut normalized = original.clone();
    let mut changed = false;

    for ((line, physical), output) in original.iter().zip(&buffer.lines).zip(&mut normalized) {
        if !physical.is_conditional_compilation() {
            continue;
        }
        let Some(prefix) = conditional_compilation_prefix(line) else {
            continue;
        };
        if prefix.kind == ConditionalPrefixKind::CompactContinuation {
            output.insert(prefix.body_start, b' ');
            changed = true;
        }
    }

    if changed {
        document.set_lines(normalized);
        Ok(Changed::Text)
    } else {
        Ok(Changed::No)
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::transform::{document::Document, pipeline::Changed};

    #[test]
    fn compact_prefix_is_canonicalized_only_for_an_open_conditional_continuation() {
        let mut document =
            Document::from_bytes(b"!$& standalone\n!$ x = a &\n!$& b\n!$ x = 1\n!$& closed\n");
        assert_eq!(run(&mut document).unwrap(), Changed::Text);
        assert_eq!(
            document.lines,
            vec![
                b"!$& standalone".to_vec(),
                b"!$ x = a &".to_vec(),
                b"!$ & b".to_vec(),
                b"!$ x = 1".to_vec(),
                b"!$& closed".to_vec(),
            ]
        );
    }

    #[test]
    fn shared_prefix_rules_cover_tab_separated_initial_lines() {
        let mut document = Document::from_bytes(b"!$\tx = a &\n!$& b\n");
        assert_eq!(run(&mut document).unwrap(), Changed::Text);
        assert_eq!(document.lines[1], b"!$ & b".to_vec());
    }

    #[test]
    fn compact_prefix_keeps_required_leading_marker_for_the_continuation_pass() {
        let mut document =
            Document::from_bytes(b"!$ sub&\n!$&routine sub\n!$ text = 'ab &\n!$&cd'\n");
        assert_eq!(run(&mut document).unwrap(), Changed::Text);
        assert_eq!(document.lines[1], b"!$ &routine sub".to_vec());
        assert_eq!(document.lines[3], b"!$ &cd'".to_vec());
    }

    #[test]
    fn conditional_state_steps_over_other_streams_and_comment_lines() {
        let mut document = Document::from_bytes(b"!$ x = a &\ny = 2\n! ordinary comment\n!$& b\n");
        assert_eq!(run(&mut document).unwrap(), Changed::Text);
        assert_eq!(document.lines[3], b"!$ & b".to_vec());
    }

    #[test]
    fn comment_ampersands_do_not_open_compact_continuations() {
        let mut document =
            Document::from_bytes(b"!$ x = 1 ! not a continuation &\n!$& untouched\n");
        let original = document.lines.clone();
        assert_eq!(run(&mut document).unwrap(), Changed::No);
        assert_eq!(document.lines, original);
    }

    #[test]
    fn hollerith_payload_ampersand_does_not_open_compact_continuation() {
        let mut document = Document::from_bytes(
            b"!$ x = 1H&
!$& standalone
",
        );
        let original = document.lines.clone();
        assert_eq!(run(&mut document).unwrap(), Changed::No);
        assert_eq!(document.lines, original);
    }

    #[test]
    fn malformed_literal_continuation_does_not_consume_protected_state() {
        let mut document =
            Document::from_bytes(b"!$ text = 'ab &\n!$ malformed without leading marker\n!$&cd'\n");
        assert_eq!(run(&mut document).unwrap(), Changed::Text);
        assert_eq!(document.lines[2], b"!$ &cd'".to_vec());
    }
}
