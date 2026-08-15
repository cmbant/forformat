use forformat::{
    analyze_project, format_source, format_source_with_context, FormatConfig, FormatMode,
};
use std::path::Path;

#[test]
fn core_fixture_is_idempotent() {
    let source = include_bytes!("fixtures/core.f90");
    let config = FormatConfig::default();
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice);
}

#[test]
fn default_mode_preserves_source_body_bytes_except_trailing_space() {
    let source = b"program p\n  ! caf\xe9\nx=1  +  2 ! keep ! punctuation\nend\n";
    let output = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(output
        .windows(b"caf\xe9".len())
        .any(|window| window == b"caf\xe9"));
    assert!(output
        .windows(b"! keep ! punctuation".len())
        .any(|window| { window == b"! keep ! punctuation" }));

    assert!(!output.windows(3).any(|window| window == b"  \n"));
}

#[test]
fn multiline_array_constructor_uses_valid_rust_delimiters() {
    let source = include_bytes!("fixtures/array_constructor_multiline.f90");
    let expected = include_bytes!("fixtures/array_constructor_multiline.out");
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    assert_eq!(format_source(source, &config).unwrap().bytes, expected);
}

#[test]
fn full_mode_pins_the_reasonable_comment_boundary() {
    let source = include_bytes!("fixtures/comment_behavior.f90");
    let expected = include_bytes!("fixtures/comment_behavior.out");
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    assert_eq!(format_source(source, &config).unwrap().bytes, expected);
}

#[test]
fn declared_case_applies_to_program_unit_locals() {
    let source = b"PROGRAM TESTER\n\
IMPLICIT NONE\n\
INTEGER L\n\
REAL RATIO\n\
l = 2\n\
ratio = 0.1\n\
END PROGRAM TESTER\n";
    let output = format_source(
        source,
        &FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes;
    let output = String::from_utf8(output).unwrap();
    assert!(output.lines().any(|line| line.trim() == "L = 2"));
    assert!(output.lines().any(|line| line.trim() == "RATIO = 0.1"));
}

#[test]
fn procedure_declarations_enter_the_local_case_table() {
    let source = b"SUBROUTINE S(x)\n\
IMPLICIT NONE\n\
PROCEDURE(state_function) :: DTAUDA\n\
x = dtauda(1.0)\n\
END SUBROUTINE S\n";
    let output = format_source(
        source,
        &FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes;
    let output = String::from_utf8(output).unwrap();
    assert!(output.lines().any(|line| line.trim() == "x = DTAUDA(1.0)"));
}

#[test]
fn external_macro_case_is_exact() {
    let source = include_bytes!("fixtures/external_macro_define.f90");
    let expected = include_bytes!("fixtures/external_macro_define.out");
    let config = FormatConfig {
        mode: FormatMode::Full,
        defines: vec![forformat::MacroDefine {
            name: "SIZE".into(),
            value: None,
        }],
        ..FormatConfig::default()
    };
    assert_eq!(format_source(source, &config).unwrap().bytes, expected);
}

#[test]
fn type_bound_procedure_case_requires_resolved_owner() {
    let type_source = include_bytes!("fixtures/type_bound_procedure_owner_type.f90");
    let unresolved = include_bytes!("fixtures/type_bound_procedure_owner_unresolved.f90");
    let unresolved_expected = include_bytes!("fixtures/type_bound_procedure_owner_unresolved.out");
    let resolved = include_bytes!("fixtures/type_bound_procedure_owner_resolved.f90");
    let resolved_expected = include_bytes!("fixtures/type_bound_procedure_owner_resolved.out");
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let mut unresolved_project = analyze_project([
        (Path::new("type.f90"), type_source.as_slice()),
        (Path::new("use.f90"), unresolved.as_slice()),
    ])
    .unwrap();
    unresolved_project.enable_target_local_component_resolution();
    assert_eq!(
        format_source_with_context(unresolved, &unresolved_project, &config)
            .unwrap()
            .bytes,
        unresolved_expected
    );
    assert_eq!(
        format_source(resolved, &config).unwrap().bytes,
        resolved_expected
    );
}
