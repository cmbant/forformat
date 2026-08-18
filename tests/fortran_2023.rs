use forformat::{format_source, FormatConfig, FormatMode};

fn normalize(source: &[u8]) -> Vec<u8> {
    format_source(
        source,
        &FormatConfig {
            mode: FormatMode::NormalizeOnly,
            apply_indent: false,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes
}

#[test]
fn fortran_2023_words_specifiers_and_intrinsics_normalize() {
    let source = b"program p\n\
real :: x, a(2)\n\
integer :: n\n\
logical :: flag\n\
TYPEOF(x) :: y\n\
CLASSOF(x) :: z\n\
INQUIRE (IOLENGTH = n) x\n\
INQUIRE (UNIT = 10, NAMED = flag)\n\
x = SUM(a) + COUNT(a > 0) + ACOSPI(x)\n\
n = SELECTED_LOGICAL_KIND(8)\n\
x = .NIL.\n\
ENUMERATION TYPE colour\n\
END ENUMERATION TYPE colour\n\
end program p\n";

    let once = normalize(source);
    let twice = normalize(&once);
    assert_eq!(twice, once);

    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("typeof(x) :: y"));
    assert!(output.contains("classof(x) :: z"));
    assert!(output.contains("inquire(iolength=n) x"));
    assert!(output.contains("inquire(unit=10, named=flag)"));
    assert!(output.contains("x = sum(a) + count(a > 0) + acospi(x)"));
    assert!(output.contains("n = selected_logical_kind(8)"));
    assert!(output.contains("x = .nil."));
    assert!(output.contains("enumeration type colour"));
    assert!(output.contains("end enumeration type colour"));
}

#[test]
fn complex_parts_follow_keyword_case_but_declared_components_win() {
    let source = b"module m\n\
type :: Parts\n\
real :: rE\n\
real :: iM\n\
end type Parts\n\
contains\n\
subroutine s(z, item)\n\
complex :: z\n\
type(Parts) :: item\n\
x = z%RE + z%IM\n\
x = item%RE + item%IM\n\
end subroutine s\n\
end module m\n";

    let once = normalize(source);
    let twice = normalize(&once);
    assert_eq!(twice, once);

    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("x = z%re + z%im"));
    assert!(output.contains("x = item%rE + item%iM"));
}

#[test]
fn intrinsic_procedure_names_do_not_reclassify_defined_dotted_operators() {
    let source = b"program p\nx = a .SUM. b\ny = .NIL.\nend program p\n";
    let output = String::from_utf8(normalize(source)).unwrap();
    assert!(output.contains("x = a .SUM. b"));
    assert!(output.contains("y = .nil."));
}
