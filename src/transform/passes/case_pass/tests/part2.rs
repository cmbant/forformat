#[test]
fn program_locals_resolve_type_bound_procedure_owners() {
    let declarations = b"module settings\n\
type :: TSettingIni\n\
contains\n\
procedure :: ReadFilename\n\
end type TSettingIni\n\
end module settings\n";
    let source = b"program ExampleApp\n\
use settings\n\
type(TSettingIni) :: Ini\n\
x = Ini%ReadFileName('file_root')\n\
end program ExampleApp\n";
    let project = analyze_project([
        (Path::new("settings.f90"), declarations.as_slice()),
        (Path::new("driver.f90"), source.as_slice()),
    ])
    .unwrap();
    let output = run_pass(source, &project, |document, context| {
        declared(document, context).unwrap()
    });
    assert!(output
        .windows(b"Ini%ReadFilename".len())
        .any(|window| window == b"Ini%ReadFilename"));
    assert!(!output
        .windows(b"Ini%ReadFileName".len())
        .any(|window| window == b"Ini%ReadFileName"));
}

#[test]
fn use_associated_module_variables_resolve_component_owners() {
    let results = b"module results\n\
type :: ModelData\n\
integer :: MODEL_PK\n\
end type ModelData\n\
end module results\n";
    let gauge = b"module GaugeInterface\n\
use results\n\
class(ModelData), pointer :: State\n\
end module GaugeInterface\n";
    let unrelated = b"module unrelated\n\
type :: Other\n\
integer :: MODEL_Pk\n\
end type Other\n\
type(Other) :: State\n\
end module unrelated\n";
    let source = b"module ExampleMain\n\
use GaugeInterface\n\
use GaugeInterface, only: Active => State\n\
contains\n\
subroutine OtherWork(State)\n\
type(Other) :: State\n\
end subroutine OtherWork\n\
subroutine MakeNonlinearSources\n\
x = State%MODEL_Pk\n\
x = Active%MODEL_Pk\n\
end subroutine MakeNonlinearSources\n\
end module ExampleMain\n";
    let project = analyze_project([
        (Path::new("results.f90"), results.as_slice()),
        (Path::new("equations.f90"), gauge.as_slice()),
        (Path::new("unrelated.f90"), unrelated.as_slice()),
        (Path::new("examplemain.f90"), source.as_slice()),
    ])
    .unwrap();
    assert_eq!(project.types.resolve_chain(b"State", &[]), None);
    let output = run_pass(source, &project, |document, context| {
        declared(document, context).unwrap()
    });
    assert!(output
        .windows(b"State%MODEL_PK".len())
        .any(|window| window == b"State%MODEL_PK"));
    assert!(!output
        .windows(b"State%MODEL_Pk".len())
        .any(|window| window == b"State%MODEL_Pk"));
    assert!(output
        .windows(b"Active%MODEL_PK".len())
        .any(|window| window == b"Active%MODEL_PK"));
}

#[test]
fn declared_names_do_not_leak_from_type_components() {
    let source = b"module C\ntype Foo\ninteger :: SIZE\nend type Foo\ncontains\nsubroutine report(x)\nreal, intent(in) :: x(:)\nprint *, SIZE(x)\nend subroutine report\nend module C\n";
    let output = crate::format_source(
        source,
        &FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes;
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("print *, size(x)"));
    assert!(!output.contains("print *, SIZE(x)"));
}

#[test]
fn interface_dummies_are_not_module_variables() {
    let source = b"module M\ninterface\nsubroutine ext(ArgCase)\ninteger :: ArgCase\nend subroutine ext\nend interface\ncontains\nsubroutine s\nprint *, argcase\nend subroutine s\nend module M\n";
    let project = analyze_project([(Path::new("interface.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}

#[test]
fn implicit_identifiers_do_not_borrow_unrelated_project_case() {
    let declarations = b"module globals\ninteger :: i\nend module globals\n";
    let cases = [
        (
            b"subroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine s(A)\nimplicit none\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\nimplicit none\ndo i = 1, 3\nA(i) = i\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine s(A)\nimplicit none(type)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\nimplicit none(type)\ndo i = 1, 3\nA(i) = i\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine s(A)\nimplicit none(external)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\nimplicit none(external)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine host\nimplicit none\ncontains\nsubroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\nend subroutine host\n".as_slice(),
            b"subroutine host\nimplicit none\ncontains\nsubroutine s(A)\ndo i = 1, 3\nA(i) = i\nend subroutine s\nend subroutine host\n".as_slice(),
        ),
        (
            b"module target\nimplicit none\ninterface\nsubroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\nend interface\nend module target\n".as_slice(),
            b"module target\nimplicit none\ninterface\nsubroutine s(A)\ndo I = 1, 3\nA(I) = I\nend subroutine s\nend interface\nend module target\n".as_slice(),
        ),
        (
            b"subroutine s(A)\nimplicit none(type)\nimplicit integer(i-n)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\nimplicit none(type)\nimplicit integer(i-n)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine s(A)\nimplicit none(type)\nimplicit real(a-)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\nimplicit none(type)\nimplicit real(A-)\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
        ),
        (
            b"subroutine s(A)\nimplicit real(a-)\nimplicit none\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
            b"subroutine s(A)\nimplicit real(A-)\nimplicit none\ndo I = 1, 3\nA(I) = I\nend subroutine s\n".as_slice(),
        ),
    ];

    for (index, (source, expected)) in cases.into_iter().enumerate() {
        let name = format!("case-{index}.f90");
        let project = analyze_project([
            (Path::new("globals.f90"), declarations.as_slice()),
            (Path::new(&name), source),
        ])
        .unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            expected,
            "implicit policy case {index}"
        );
    }
}
