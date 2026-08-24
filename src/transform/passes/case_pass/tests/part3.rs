#[test]
fn implicit_function_syntax_is_guarded_but_call_syntax_is_not() {
    let declarations = b"module globals\ninteger :: xfun\ncontains\nsubroutine xproc(n)\ninteger :: n\nend subroutine xproc\nend module globals\n";
    let source = b"subroutine s(out)\nout = XFUN(3)\ncall XPROC(3)\nend subroutine s\n";
    let project = analyze_project([
        (Path::new("globals.f90"), declarations.as_slice()),
        (Path::new("target.f90"), source.as_slice()),
    ])
    .unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"subroutine s(out)\nout = XFUN(3)\ncall xproc(3)\nend subroutine s\n"
    );
}

#[test]
fn explicit_host_locals_and_use_names_still_canonicalize() {
    let declarations = b"module globals\ninteger :: ProjectName\nend module globals\n";
    let source = b"subroutine host\ninteger :: HostName\ncontains\nsubroutine child\nhostname = 1\nend subroutine child\nend subroutine host\nsubroutine imports\nuse globals, only: projectname\nend subroutine imports\n";
    let project = analyze_project([
        (Path::new("globals.f90"), declarations.as_slice()),
        (Path::new("target.f90"), source.as_slice()),
    ])
    .unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"subroutine host\ninteger :: HostName\ncontains\nsubroutine child\nHostName = 1\nend subroutine child\nend subroutine host\nsubroutine imports\nuse globals, only: ProjectName\nend subroutine imports\n"
    );
}

#[test]
fn unresolved_members_do_not_borrow_other_name_spaces() {
    let sources = [
        (
            Path::new("global.f90"),
            b"type :: ComponentCase\nend type ComponentCase\n".as_slice(),
        ),
        (
            Path::new("components.f90"),
            b"subroutine Work\nreal :: WINDOW\nWINDOW = RedWin%componentcase%Window_f_a(a, winamp)\nend subroutine work\n".as_slice(),
        ),
    ];
    let project = analyze_project(sources).unwrap();
    let source = b"subroutine Work\nreal :: WINDOW\nWINDOW = RedWin%componentcase%Window_f_a(a, winamp)\nend subroutine work\n";
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"subroutine Work\nreal :: WINDOW\nWINDOW = RedWin%componentcase%Window_f_a(a, winamp)\nend subroutine Work\n"
    );
}

#[test]
fn missing_owner_members_do_not_borrow_unrelated_bindings_or_symbols() {
    let declarations = b"module declarations\n\
type :: Other\n\
contains\n\
procedure :: RunCase\n\
end type Other\n\
integer :: ValueCase\n\
end module declarations\n";
    let source = b"program p\n\
type(Unknown) :: item\n\
call item%runcase()\n\
item%valuecase = 1\n\
end program p\n";
    let project = analyze_project([
        (Path::new("declarations.f90"), declarations.as_slice()),
        (Path::new("use.f90"), source.as_slice()),
    ])
    .unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}

#[test]
fn associate_aliases_propagate_indexed_selector_types_with_lexical_shadowing() {
    let source = b"module SourceWindows\n\
type :: TSourceWindow\n\
contains\n\
procedure :: Window_f_a\n\
end type TSourceWindow\n\
type :: TRedWin\n\
class(TSourceWindow), pointer :: Window\n\
end type TRedWin\n\
type :: ModelData\n\
type(TRedWin), allocatable :: Redshift_W(:)\n\
end type ModelData\n\
type :: Other\n\
integer :: WrongCase\n\
end type Other\n\
contains\n\
subroutine Work(State, OtherState)\n\
class(ModelData) :: State\n\
type(Other) :: OtherState\n\
AssocBlock: associate(UnTyped => UnknownCall(1, 2), RedWin => State%Redshift_W(1))\n\
call RedWin%window%window_F_A()\n\
associate(RedWin => OtherState)\n\
RedWin%wrongcase = 1\n\
end associate\n\
call RedWin%WINDOW%WINDOW_F_A()\n\
end associate AssocBlock\n\
call RedWin%WINDOW%WINDOW_F_A()\n\
end subroutine Work\n\
end module SourceWindows\n";
    let project = analyze_project([(Path::new("associate.f90"), source.as_slice())]).unwrap();
    let output = run_pass(source, &project, |document, context| {
        declared(document, context).unwrap()
    });
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("RedWin%Window%Window_f_a()").count(), 2);
    assert!(output.contains("RedWin%WrongCase = 1"));
    assert!(output.contains("call RedWin%WINDOW%WINDOW_F_A()\nend subroutine Work"));
}

#[test]
fn module_variables_are_case_matched_without_leaking_local_shadowing() {
    let config = b"module config\ninteger :: FeedbackLevel\ntype :: State\nreal :: transfer_times\nreal :: H0\nend type State\nend module config\n";
    let source = b"module Uses\nuse config\ncontains\nsubroutine Work(Feedbacklevel, H0)\ninteger :: Feedbacklevel\nreal :: H0\ntype(State) :: obj\nprint *, feedbacklevel\nprint *, obj%transfer_times\nend subroutine work\nend module Uses\n";
    let project = analyze_project([
        (Path::new("config.f90"), config.as_slice()),
        (Path::new("uses.f90"), source.as_slice()),
    ])
    .unwrap();
    let output = format_source_with_context(
        source,
        &project,
        &FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes;
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("print *, Feedbacklevel"));
    assert!(!output.contains("print *, FeedbackLevel"));
    assert!(output.contains("obj%transfer_times"));
}

#[test]
fn declaration_entities_are_not_replaced_by_global_symbol_case() {
    let source = b"module M\ninteger :: ERROR\ntype :: T\ncontains\nprocedure :: Error\nend type T\nend module M\n";
    let project = analyze_project([(Path::new("declaration.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}
