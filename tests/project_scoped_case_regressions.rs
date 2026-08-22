use forformat::{analyze_project, format_source_with_context, FormatConfig, FormatMode};
use std::path::Path;

fn normalize<'a, I>(target: &[u8], sources: I) -> String
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let project = analyze_project(sources).unwrap();
    String::from_utf8(
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
    .unwrap()
}

#[test]
fn use_list_remote_names_resolve_against_the_statement_module() {
    let a = b"module A\ninteger :: RemoteCase\nend module A\n";
    let b = b"module B\ninteger :: REMOTECASE\nend module B\n";
    let target = b"program p\nuse A, only: LocalA => remotecase\nuse B, only: LocalB => remotecase\nprint *, locala, localb\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("a.f90"), a.as_slice()),
            (Path::new("b.f90"), b.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );

    assert!(
        output.contains("use A, only: LocalA => RemoteCase"),
        "{output}"
    );
    assert!(
        output.contains("use B, only: LocalB => REMOTECASE"),
        "{output}"
    );
    assert!(output.contains("print *, LocalA, LocalB"), "{output}");
}

#[test]
fn associate_selector_uses_the_surrounding_scope_not_its_new_alias() {
    let module = b"module M\ninteger :: OuterCase\nend module M\n";
    let target = b"program p\nuse M\nassociate(OUTERCASE => outercase)\nprint *, outercase\nend associate\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("m.f90"), module.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );

    assert!(
        output.contains("associate(OUTERCASE => OuterCase)"),
        "{output}"
    );
    assert!(output.contains("print *, OUTERCASE"), "{output}");
}

#[test]
fn use_double_colon_forms_still_resolve_only_names() {
    let relevant = b"module Relevant\ninteger :: CamelCase\nend module Relevant\n";
    let secondary = b"module Secondary\ninteger :: SecondCase\nend module Secondary\n";
    let unrelated = b"module Unrelated\ninteger :: CAMELCASE\nend module Unrelated\n";
    let target = b"program p\nuse :: Relevant, only: camelcase\nuse, non_intrinsic :: Secondary, only: secondcase\nprint *, camelcase, secondcase\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("relevant.f90"), relevant.as_slice()),
            (Path::new("secondary.f90"), secondary.as_slice()),
            (Path::new("unrelated.f90"), unrelated.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );

    assert!(
        output.contains("use :: Relevant, only: CamelCase"),
        "{output}"
    );
    assert!(
        output.contains("use, non_intrinsic :: Secondary, only: SecondCase"),
        "{output}"
    );
    assert!(
        output.contains("print *, CamelCase, SecondCase"),
        "{output}"
    );
}
