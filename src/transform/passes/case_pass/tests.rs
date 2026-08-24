use super::{declared, macros};
use crate::{
    analysis::{analyze_file, analyze_project, ScopeTree},
    config::{FormatConfig, FormatMode},
    format_source_with_context,
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
    },
};
use std::path::Path;

fn run_pass(
    source: &[u8],
    project: &crate::analysis::ProjectContext,
    pass: impl FnOnce(&mut Document, &PassContext<'_>) -> Changed,
) -> Vec<u8> {
    let mut document = Document::from_bytes(source);
    let local = analyze_file(source).unwrap();
    let analysis = document.analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    let config = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };
    let context = PassContext {
        config: &config,
        project,
        local: &local,
        analysis: &analysis,
        scopes: &scopes,
    };
    let _ = pass(&mut document, &context);
    document.to_bytes()
}

include!("tests/part1.rs");
include!("tests/part2.rs");
include!("tests/part3.rs");
include!("tests/part4.rs");
include!("tests/part5.rs");
