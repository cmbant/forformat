use forformat::{analyze_project, format_source_with_context, FormatConfig, FormatMode};
use std::path::Path;

#[test]
fn project_visible_standard_words_are_identifiers_outside_keyword_arguments() {
    let provider =
        b"module Words\nimplicit none\ninteger, public :: File, Size\nend module Words\n";
    let target = b"program p\nuse Words\nimplicit none\ninteger :: x\nx = file + size\nopen(unit=10, FILE='x')\nend program p\n";
    let project = analyze_project([
        (Path::new("words.f90"), provider.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .unwrap();
    let output = String::from_utf8(
        format_source_with_context(
            target,
            &project,
            &FormatConfig {
                mode: FormatMode::NormalizeOnly,
                ..FormatConfig::default()
            },
        )
        .unwrap()
        .bytes,
    )
    .unwrap();

    assert!(output.contains("x = File + Size"), "{output}");
    assert!(output.contains("open(unit=10, file='x')"), "{output}");
}
