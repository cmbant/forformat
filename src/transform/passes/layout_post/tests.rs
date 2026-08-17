use super::{
    declaration_separator_alignment, limit_blank_lines, output_whitespace, program_unit_spacing,
    trailing_comment_alignment,
};
use crate::{config::FormatConfig, transform::document::Document};

fn apply_all(source: &[u8]) -> Vec<u8> {
    apply_all_with(source, &FormatConfig::default())
}

fn apply_all_with_comment_alignment(source: &[u8]) -> Vec<u8> {
    let config = FormatConfig {
        align_comments: true,
        ..FormatConfig::default()
    };
    apply_all_with(source, &config)
}

fn apply_all_with(source: &[u8], config: &FormatConfig) -> Vec<u8> {
    let mut document = Document::from_bytes(source);
    declaration_separator_alignment(&mut document, config).unwrap();
    trailing_comment_alignment(&mut document, config).unwrap();
    program_unit_spacing(&mut document, config).unwrap();
    limit_blank_lines(&mut document, config).unwrap();
    output_whitespace(&mut document, config).unwrap();
    document.to_bytes()
}

fn assert_retained_lines_do_not_grow(before: &[Vec<u8>], after: &[Vec<u8>]) {
    let old = before
        .iter()
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()));
    let new = after
        .iter()
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()));
    assert!(old.zip(new).all(|(old, new)| new.len() <= old.len()));
}

mod alignment;
mod spacing;
