use super::*;

#[test]
fn scoped_name_indexes_use_zero_based_physical_line_indices() {
    let names = scoped(
        b"module m\ninteger :: Status\ncontains\nsubroutine s(Size)\ninteger :: Local\nx = Size\nend subroutine s\nend module m\n",
    );
    assert!(names.file_declared_contains(0, b"status"));
    assert!(names.file_declared_contains(5, b"status"));
    assert!(names.local_contains(3, b"size"));
    assert!(names.local_contains(5, b"local"));
    assert!(!names.local_contains(7, b"size"));
    assert!(!names.file_declared_contains(0, b"size"));
}

#[test]
fn block_declarations_do_not_outlive_their_construct() {
    let names = scoped(
        b"module m\ninteger :: ModuleVar\ncontains\nsubroutine s()\nblock\ninteger :: MYVAR\nmyvar = 1\nend block\nmyvar = 2\nend\nend module m\n",
    );
    assert!(names.local_contains(6, b"myvar"));
    assert_eq!(
        names.governing_local_case(6, b"myvar"),
        DeclaredSpelling::Spelling(b"MYVAR")
    );
    assert!(!names.local_contains(8, b"myvar"));
    assert_eq!(
        names.governing_local_case(8, b"myvar"),
        DeclaredSpelling::Absent
    );
    assert!(names.file_declared_contains(6, b"modulevar"));
}

#[test]
fn a_block_shadows_a_host_name_without_making_it_ambiguous() {
    let names = scoped(
        b"subroutine s()\ninteger :: Value\nblock\ninteger :: VALUE\nVALUE = 1\nend block\nValue = 2\nend subroutine s\n",
    );
    assert_eq!(
        names.governing_local_case(4, b"value"),
        DeclaredSpelling::Spelling(b"VALUE")
    );
    assert_eq!(
        names.governing_local_case(6, b"value"),
        DeclaredSpelling::Spelling(b"Value")
    );
}

#[test]
fn procedure_header_names_survive_a_logical_continuation() {
    let names = scoped(
        b"subroutine s(first, second, &\nthird)\ncall f(FIRST, SECOND, THIRD)\nend subroutine s\n",
    );
    for name in [b"first".as_slice(), b"second", b"third"] {
        assert!(names.local_contains(0, name));
        assert!(names.local_contains(1, name));
    }
}

#[test]
fn program_units_use_the_procedure_local_case_scope() {
    let names = scoped(
        b"program tester\nimplicit none\ninteger L\nreal RATIO\nl = 2\nratio = 0.1\nend program tester\n",
    );
    assert_eq!(
        names.local_at(4).and_then(|locals| locals.get(b"l")),
        Some(b"L".as_slice())
    );
    assert_eq!(
        names.local_at(5).and_then(|locals| locals.get(b"ratio")),
        Some(b"RATIO".as_slice())
    );
}

#[test]
fn implicit_typing_policies_follow_scope_inheritance_and_resets() {
    let default = scoped(b"subroutine s\nx = I\nend subroutine s\n");
    assert!(default.implicit_allows(1, b"I"));

    let none = scoped(b"subroutine s\nimplicit none\nx = I\nend subroutine s\n");
    assert!(!none.implicit_allows(2, b"I"));

    let none_type = scoped(b"subroutine s\nimplicit none(type)\nx = I\nend subroutine s\n");
    assert!(!none_type.implicit_allows(2, b"I"));

    let none_external = scoped(b"subroutine s\nimplicit none(external)\nx = I\nend subroutine s\n");
    assert!(none_external.implicit_allows(2, b"I"));

    let contained = scoped(
        b"subroutine host\nimplicit none\ncontains\nsubroutine child\nx = I\nend subroutine child\nend subroutine host\n",
    );
    assert!(!contained.implicit_allows(4, b"I"));

    let ranged = scoped(
        b"subroutine host\nimplicit none(type)\ncontains\nsubroutine child\nimplicit integer(i-n)\nx = I + A\nend subroutine child\nend subroutine host\n",
    );
    assert!(ranged.implicit_allows(5, b"I"));
    assert!(!ranged.implicit_allows(5, b"A"));

    let interface = scoped(
        b"module m\nimplicit none\ninterface\nsubroutine signature\nx = I\nend subroutine signature\nend interface\nend module m\n",
    );
    assert!(interface.implicit_allows(4, b"I"));

    let malformed =
        scoped(b"subroutine s\nimplicit none(type)\nimplicit real(a-)\nx = I\nend subroutine s\n");
    assert!(malformed.implicit_allows(3, b"I"));

    let malformed_before_none =
        scoped(b"subroutine s\nimplicit real(a-)\nimplicit none\nx = I\nend subroutine s\n");
    assert!(malformed_before_none.implicit_allows(3, b"I"));

    let inherited_malformed = scoped(
        b"subroutine host\nimplicit real(a-)\ncontains\nsubroutine child\nimplicit none\nx = I\nend subroutine child\nend subroutine host\n",
    );
    assert!(inherited_malformed.implicit_allows(5, b"I"));
}

#[test]
fn governing_local_case_includes_host_association() {
    let names = scoped(
        b"subroutine host\ninteger :: IndexValue\ncontains\nsubroutine child\nx = indexvalue\nend subroutine child\nend subroutine host\n",
    );
    assert_eq!(
        names.governing_local_case(4, b"indexvalue"),
        DeclaredSpelling::Spelling(b"IndexValue".as_slice())
    );
}

#[test]
fn procedure_pointer_declarations_are_procedure_locals() {
    let names = scoped(
        b"subroutine s(x)\nimplicit none\nprocedure(state_function) :: DTAUDA\nx = dtauda(1.0)\nend subroutine s\n",
    );
    assert_eq!(
        names.local_at(3).and_then(|locals| locals.get(b"dtauda")),
        Some(b"DTAUDA".as_slice())
    );
}

#[test]
fn scoped_declared_names_exclude_components_and_interface_bodies() {
    let names = scoped(
        b"module m\ninterface\nsubroutine signature(Status)\ninteger :: Status\nend subroutine signature\nend interface\ntype :: t\ninteger :: Component\nend type t\ninteger :: Visible\nend module m\n",
    );
    for line in 0..11 {
        assert!(!names.file_declared_contains(line, b"component"));
        assert!(!names.file_declared_contains(line, b"signature"));
    }
    assert!(names.file_declared_contains(9, b"visible"));
}

#[test]
fn continued_parameter_declarations_are_procedure_locals() {
    let source = b"module m\ncontains\nfunction f(t)\ninteger, parameter :: n_table = 27\ninteger, dimension(n_table), parameter :: Temps = &\n [1, 2]\nreal, dimension(n_table), parameter :: rates = &\n [1.0, 2.0]\nx = RATES + TEMPS(1)\nend function f\nend module m\n";
    let document = Document::from_bytes(source);
    let analysis = document.analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    let names = scoped_declared_names(&analysis, &scopes);
    assert!(names.local_contains(8, b"rates"));
    assert!(names.local_contains(8, b"temps"));
}

#[test]
fn numeric_function_names_do_not_hide_later_locals() {
    let source = b"module m\ncontains\nfunction kappa_HH_21cm(T, deriv)\ninteger, parameter :: n_table = 27\ninteger, dimension(n_table), parameter :: Temps = &\n [1, 2]\nreal, dimension(n_table), parameter :: rates = &\n [1.0, 2.0]\nx = RATES + TEMPS(1)\nend function kappa_HH_21cm\nend module m\n";
    let document = Document::from_bytes(source);
    let analysis = document.analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    let names = scoped_declared_names(&analysis, &scopes);
    assert!(names.local_contains(8, b"rates"));
    assert!(names.local_contains(8, b"temps"));
}

#[test]
fn procedure_headers_results_and_interface_dummies_are_local() {
    let names = scoped(
        b"module m\ninterface\nfunction sig(arg) result(answer)\nreal :: arg, answer\nend function sig\nend interface\ncontains\nfunction real_name(value) result(ResultValue)\nreal :: value, ResultValue\nend function real_name\nend module m\n",
    );
    assert!(names.local_contains(2, b"arg"));
    assert!(names.local_contains(2, b"answer"));
    assert!(names.local_contains(7, b"value"));
    assert!(names.local_contains(7, b"resultvalue"));
    assert!(!names.file_declared_contains(2, b"arg"));
}
