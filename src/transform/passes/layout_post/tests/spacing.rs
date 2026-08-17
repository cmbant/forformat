use super::*;

#[test]
fn trailing_horizontal_whitespace_is_removed_from_every_line() {
    let mut document = Document::from_bytes(b"x = 1   \n\t\n  y = 2\t \n");
    output_whitespace(&mut document, &FormatConfig::default()).unwrap();
    assert_eq!(document.to_bytes(), b"x = 1\n\n  y = 2\n");
}

#[test]
fn final_newlines_match_end_of_file_fixer() {
    for (source, expected) in [
        (b"".as_slice(), b"".as_slice()),
        (b"x = 1".as_slice(), b"x = 1\n".as_slice()),
        (b"x = 1\n\n\n".as_slice(), b"x = 1\n".as_slice()),
        (b"x = 1\r\n\r\n".as_slice(), b"x = 1\r\n".as_slice()),
        (b"\n\n".as_slice(), b"\n".as_slice()),
    ] {
        let mut document = Document::from_bytes(source);
        output_whitespace(&mut document, &FormatConfig::default()).unwrap();
        assert_eq!(document.to_bytes(), expected, "source: {source:?}");
    }
}

#[test]
fn a_blank_line_still_splits_a_block_that_cannot_share_a_column() {
    let source = b"integer :: a\n\ntype(a_very_long_derived_type_name) :: b\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"integer :: a".len())
        .any(|w| w == b"integer :: a"));
    assert!(output
        .windows(b"type(a_very_long_derived_type_name) :: b".len())
        .any(|w| w == b"type(a_very_long_derived_type_name) :: b"));
}

#[test]
fn program_unit_spacing_handles_contains_types_interfaces_and_is_idempotent() {
    let source = b"module m\ntype :: t\ncontains\nprocedure :: p\nend type t\ncontains\nsubroutine s\nend subroutine s\nend module m\ninterface\nsubroutine x\nend subroutine x\nend interface\n";
    let once = apply_all(source);
    assert_eq!(apply_all(&once), once);
    assert!(once.windows(2).filter(|pair| *pair == b"\n\n").count() >= 2);
}

#[test]
fn unit_separator_after_conditional_procedure_end_belongs_before_host_end() {
    for (if_directive, endif_directive) in
        [("#if", "#endif"), ("??if", "??endif"), ("#:if", "#:endif")]
    {
        let source = format!(
            "module m\ncontains\n{if_directive} X\nsubroutine s\nend subroutine s\n{endif_directive}\nend module m\n"
        )
        .into_bytes();
        let expected = format!(
            "module m\n\ncontains\n\n{if_directive} X\nsubroutine s\n\nend subroutine s\n{endif_directive}\n\nend module m\n"
        )
        .into_bytes();
        let once = apply_all(&source);
        assert_eq!(once, expected, "directive: {endif_directive}");
        assert_eq!(apply_all(&once), once);
    }
}

#[test]
fn adjacent_program_units_have_one_blank_line_and_are_idempotent() {
    for (source, expected) in [
        (
            b"function f\nend function f\nsubroutine s\nend subroutine s\n".as_slice(),
            b"function f\n\nend function f\n\nsubroutine s\n\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine f\nend subroutine f\nsubroutine s\nend subroutine s\n".as_slice(),
            b"subroutine f\n\nend subroutine f\n\nsubroutine s\n\nend subroutine s\n"
                .as_slice(),
        ),
        (
            b"module m\nend module m\nprogram p\nend program p\n".as_slice(),
            b"module m\n\nend module m\n\nprogram p\n\nend program p\n".as_slice(),
        ),
    ] {
        let once = apply_all(source);
        assert_eq!(once, expected);
        assert_eq!(apply_all(&once), once);
    }
}

#[test]
fn a_unit_separator_is_a_floor_of_one_and_a_cap_of_two() {
    for (authored, expected) in [(0, 1), (1, 1), (2, 2), (3, 2), (5, 2)] {
        let mut source = b"module m\nend module m\n".to_vec();
        source.extend(std::iter::repeat_n(b'\n', authored));
        source.extend_from_slice(b"program p\nend program p\n");

        let once = apply_all(&source);
        let mut separator = b"end module m\n".to_vec();
        separator.extend(std::iter::repeat_n(b'\n', expected));
        separator.extend_from_slice(b"program p");
        assert!(
            once.windows(separator.len()).any(|w| w == separator),
            "{authored} authored blank lines should settle at {expected}, got:\n{}",
            String::from_utf8_lossy(&once)
        );
        assert_eq!(apply_all(&once), once, "{authored} authored blank lines");
    }
}

#[test]
fn a_contains_separator_stays_exactly_one_line() {
    let source = b"module m\ncontains\n\n\n\nsubroutine s\nend subroutine s\nend module m\n";
    let once = apply_all(source);
    assert!(once
        .windows(b"contains\n\nsubroutine s".len())
        .any(|w| w == b"contains\n\nsubroutine s"));
    assert_eq!(apply_all(&once), once);
}

#[test]
fn submodules_and_separate_module_procedures_share_unit_spacing() {
    let source = b"submodule (p) c\ncontains\nmodule procedure binding\nx = 1\nend procedure binding\nend submodule c\nsubmodule (p:gp) c2\ncontains\nmodule procedure binding\nx = 1\nend procedure binding\nend submodule c2\nmodule equivalent\ncontains\nsubroutine binding\nx = 1\nend subroutine binding\nend module equivalent\n";
    let once = apply_all(source);
    assert_eq!(apply_all(&once), once);
    for (header, end) in [
        (
            b"submodule (p) c\n\ncontains\n\nmodule procedure binding".as_slice(),
            b"end procedure binding\n\nend submodule c".as_slice(),
        ),
        (
            b"submodule (p:gp) c2\n\ncontains\n\nmodule procedure binding".as_slice(),
            b"end procedure binding\n\nend submodule c2".as_slice(),
        ),
        (
            b"module equivalent\n\ncontains\n\nsubroutine binding".as_slice(),
            b"end subroutine binding\n\nend module equivalent".as_slice(),
        ),
    ] {
        assert!(once.windows(header.len()).any(|window| window == header));
        assert!(once.windows(end.len()).any(|window| window == end));
    }
}

#[test]
fn procedure_ends_in_interfaces_and_types_do_not_consume_the_host_unit() {
    let source = b"module interface_host\ninterface\nmodule procedure p\nend procedure p\nend interface\ncontains\nsubroutine s\nend subroutine s\nend module interface_host\nmodule type_host\ntype :: t\ncontains\nprocedure :: p\nend procedure p\nend type t\ncontains\nsubroutine s\nend subroutine s\nend module type_host\nblock data b\ncommon /b/ x\nend block data b\n";
    let output = apply_all(source);
    assert_eq!(apply_all(&output), output);
    for expected in [
        b"end procedure p\nend interface\n\ncontains".as_slice(),
        b"end procedure p\nend type t\n\ncontains".as_slice(),
        b"common /b/ x\n\nend block data b".as_slice(),
    ] {
        assert!(output
            .windows(expected.len())
            .any(|window| window == expected));
    }
}

#[test]
fn module_interfaces_are_limited_to_one_blank_line() {
    let source = b"module demo\n\n\ninterface\n\n\nend interface\n\n\ncontains\nsubroutine work\nend subroutine work\nend module demo\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"module demo\n\ninterface\n\nend interface\n\ncontains".len())
        .any(|w| w == b"module demo\n\ninterface\n\nend interface\n\ncontains"));
}

#[test]
fn contains_boundaries_keep_exactly_one_blank_line() {
    let source = b"module demo\ninteger :: value\n\n\n\ncontains\n\n\n\nsubroutine work\nend subroutine work\nend module demo\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"integer :: value\n\ncontains\n\nsubroutine work".len())
        .any(|w| w == b"integer :: value\n\ncontains\n\nsubroutine work"));
}

#[test]
fn contains_after_select_type_keeps_the_following_blank_line() {
    let source = b"function format_value(value) result(text)\nclass(*) :: value\nselect type (value)\ntype is (integer)\ntext = 'integer'\nend select\ncontains\nsubroutine error\nend subroutine error\nend function format_value\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"end select\n\ncontains\n\nsubroutine error".len())
        .any(|w| w == b"end select\n\ncontains\n\nsubroutine error"));
}

#[test]
fn bare_program_unit_ends_have_the_same_separator_as_named_ends() {
    let source = b"subroutine first\ninteger :: value\nvalue = 1\nend\nsubroutine second\ninteger :: value\nvalue = 2\nend\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"value = 1\n\nend\n\nsubroutine second".len())
        .any(|w| w == b"value = 1\n\nend\n\nsubroutine second"));
}

#[test]
fn named_program_unit_end_reduces_the_following_blank_run() {
    let source = b"subroutine a\nend subroutine a\n\n\n\nx=1\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"end subroutine a\n\n\nx=1".len())
        .any(|w| w == b"end subroutine a\n\n\nx=1"));
    assert!(!output
        .windows(b"end subroutine a\n\n\n\n".len())
        .any(|w| w == b"end subroutine a\n\n\n\n"));
}

#[test]
fn blank_runs_are_capped_without_crossing_cpp_continuations() {
    let source = b"#define A \\\n+\n\n\n\nvalue\n\n\n\nnext\n";
    let mut document = Document::from_bytes(source);
    limit_blank_lines(&mut document, &FormatConfig::default()).unwrap();
    assert_eq!(
        document.to_bytes(),
        b"#define A \\\n+\n\n\nvalue\n\n\nnext\n"
    );
    let once = document.to_bytes();
    limit_blank_lines(&mut document, &FormatConfig::default()).unwrap();
    assert_eq!(document.to_bytes(), once);
}

#[test]
fn classifier_drives_prefixed_headers_and_abstract_interfaces() {
    let source = b"module m\nabstract interface\npure subroutine signature(x)\nreal :: x\nend subroutine signature\nend interface\ncontains\npure elemental function f(x) result(y)\nreal :: x, y\ny = x\nend function f\nend module m\n";
    let once = apply_all(source);
    assert_eq!(apply_all(&once), once);
    assert!(once
        .windows(b"end interface\n\ncontains\n\npure elemental function f".len())
        .any(|window| window == b"end interface\n\ncontains\n\npure elemental function f"));
}
