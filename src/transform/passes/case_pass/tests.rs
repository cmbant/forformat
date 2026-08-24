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

#[test]
fn macro_uses_are_replaced_but_cpp_strings_and_comments_are_protected() {
    let source = b"#define My_Macro 1\nprogram p\nx = MY_MACRO\ns = 'MY_MACRO' ! MY_MACRO\n#if MY_MACRO\nend program p\n";
    let project = analyze_project([(Path::new("macros.f90"), source.as_slice())]).unwrap();
    let output = run_pass(source, &project, |document, context| {
        macros(document, context).unwrap()
    });
    assert_eq!(
        output,
        b"#define My_Macro 1\nprogram p\nx = My_Macro\ns = 'MY_MACRO' ! MY_MACRO\n#if MY_MACRO\nend program p\n"
    );
}

#[test]
fn declared_occurrences_use_their_name_spaces_and_are_idempotent() {
    let source = b"module MiXeD\ntype :: MyType\ninteger :: Source\ncontains\nprocedure :: BuildValue\nend type MyType\ninteger :: Global\ncontains\nsubroutine Work(Local)\ntype(MyType) :: obj\nlocal = GLOBAL\nobj%source = 1\ncall obj%buildvalue()\nend subroutine work\nend module mixed\n";
    let project = analyze_project([(Path::new("names.f90"), source.as_slice())]).unwrap();
    let once = run_pass(source, &project, |document, context| {
        macros(document, context).unwrap();
        declared(document, context).unwrap()
    });
    assert_eq!(
        once,
        b"module MiXeD\ntype :: MyType\ninteger :: Source\ncontains\nprocedure :: BuildValue\nend type MyType\ninteger :: Global\ncontains\nsubroutine Work(Local)\ntype(MyType) :: obj\nLocal = Global\nobj%Source = 1\ncall obj%BuildValue()\nend subroutine Work\nend module MiXeD\n"
    );
    let twice = run_pass(&once, &project, |document, context| {
        macros(document, context).unwrap();
        declared(document, context).unwrap()
    });
    assert_eq!(twice, once);
}

#[test]
fn implicit_function_result_spelling_is_shared_with_calls() {
    let source = b"module m\n\
contains\n\
function BETA3(x)\n\
implicit none\n\
real :: x\n\
real :: BeTa3\n\
BeTa3 = x\n\
end function beta3\n\
subroutine s(x, num)\n\
real :: x, num\n\
num = bEtA3(x)\n\
end subroutine s\n\
end module m\n";
    let project = analyze_project([(Path::new("implicit-result.f90"), source.as_slice())]).unwrap();
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source_with_context(source, &project, &config)
        .unwrap()
        .bytes;
    let twice = format_source_with_context(&once, &project, &config)
        .unwrap()
        .bytes;
    assert_eq!(twice, once);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("function BETA3(x)"));
    assert!(output.contains("real :: BETA3"));
    assert!(output.contains("BETA3 = x"));
    assert!(output.contains("end function BETA3"));
    assert!(output.contains("num = BETA3(x)"));
}

#[test]
fn explicit_function_result_does_not_use_result_spelling_for_calls() {
    let source = b"module m\n\
contains\n\
function BETA3(x) result(ResultValue)\n\
implicit none\n\
real :: x\n\
real :: resultvalue\n\
resultvalue = x\n\
end function beta3\n\
subroutine s(x, num)\n\
real :: x, num\n\
num = bEtA3(x)\n\
end subroutine s\n\
end module m\n";
    let project = analyze_project([(Path::new("explicit-result.f90"), source.as_slice())]).unwrap();
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source_with_context(source, &project, &config)
        .unwrap()
        .bytes;
    let twice = format_source_with_context(&once, &project, &config)
        .unwrap()
        .bytes;
    assert_eq!(twice, once);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("function BETA3(x) result(resultvalue)"));
    assert!(output.contains("resultvalue = x"));
    assert!(output.contains("num = BETA3(x)"));
    assert!(!output.contains("num = ResultValue(x)"));
}

/// A `%` in the first token slot leaves no room for an owner, and the
/// component names `err` and `index` are resolved by the ordinary member
/// path rather than by a name-specific exception.
#[test]
fn leading_percent_and_err_index_components_use_the_ordinary_member_path() {
    let source = b"module m\n\
type :: T\n\
integer :: Err\n\
integer :: Index\n\
end type T\n\
contains\n\
subroutine s(list)\n\
type(T) :: list(2)\n\
list(1)%err = 1\n\
list(2)%index = 2\n\
end subroutine s\n\
end module m\n";
    let project = analyze_project([(Path::new("members.f90"), source.as_slice())]).unwrap();
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source_with_context(source, &project, &config)
        .unwrap()
        .bytes;
    let twice = format_source_with_context(&once, &project, &config)
        .unwrap()
        .bytes;
    assert_eq!(twice, once);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("list(1)%Err = 1"), "{output}");
    assert!(output.contains("list(2)%Index = 2"), "{output}");

    // A statement whose first token is `%` used to index out of bounds.
    for stray in [b"%err\n".as_slice(), b"%index\n".as_slice(), b"% err\n"] {
        let project = analyze_project([(Path::new("stray.f90"), stray)]).unwrap();
        format_source_with_context(stray, &project, &config).unwrap();
    }
}

#[test]
fn a_block_declaration_does_not_recase_uses_after_its_end() {
    let source = b"module m\n\
integer :: ModuleVar\n\
contains\n\
subroutine s()\n\
block\n\
integer :: MYVAR\n\
myvar = 1\n\
end block\n\
myvar = 2\n\
modulevar = 3\n\
end\n\
end module m\n";
    let project = analyze_project([(Path::new("block.f90"), source.as_slice())]).unwrap();
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source_with_context(source, &project, &config)
        .unwrap()
        .bytes;
    let twice = format_source_with_context(&once, &project, &config)
        .unwrap()
        .bytes;
    assert_eq!(twice, once);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("MYVAR = 1"));
    assert!(output.contains("myvar = 2"));
    assert!(output.contains("ModuleVar = 3"));
}

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

#[test]
fn local_type_components_after_module_contains_do_not_leak() {
    let source = b"module m\ncontains\nsubroutine s\ntype :: Local\ninteger :: WeirdCase\nend type Local\nend subroutine s\nend module m\nprogram p\nx = weirdcase\nend program p\n";
    let project = analyze_project([(Path::new("local_type.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}

#[test]
fn ambiguous_local_and_project_cases_are_silent() {
    let local_source = b"module Foo\nmodule fOO\nuse foo\n";
    let local_project =
        analyze_project([(Path::new("local.f90"), local_source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(local_source, &local_project, |document, context| {
            declared(document, context).unwrap()
        }),
        local_source
    );

    let project = analyze_project([
        (Path::new("a.f90"), b"module Foo\n".as_slice()),
        (Path::new("b.f90"), b"module FOO\n".as_slice()),
    ])
    .unwrap();
    let use_source = b"program p\nuse foo\nend program p\n";
    assert_eq!(
        run_pass(use_source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        use_source
    );
}

#[test]
fn old_style_procedure_headers_supply_local_case_spellings() {
    let source = b"module m\ncontains\nfunction f(this, maxfun)\nclass(*) this\ninteger maxfun\nthis = maxfun\nend function f\nend module m\n";
    let analysis = Document::from_bytes(source).analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
    assert_eq!(
        names.local_at(5).and_then(|map| map.get(b"this")),
        Some(b"this".as_slice())
    );
    assert_eq!(
        names.local_at(5).and_then(|map| map.get(b"maxfun")),
        Some(b"maxfun".as_slice())
    );
}

#[test]
fn old_style_declaration_protects_each_comma_separated_entity() {
    let source = b"program p\nreal(dl) kh, Mixed\nend program p\n";
    let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}

#[test]
fn explicit_local_declarations_override_continued_header_spelling() {
    let source = b"module m\ncontains\nfunction f(this, maxfun)\nclass(*) THIS\ninteger MAXFUN\nthis = maxfun\nend function f\nend module m\n";
    let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module m\ncontains\nfunction f(THIS, MAXFUN)\nclass(*) THIS\ninteger MAXFUN\nTHIS = MAXFUN\nend function f\nend module m\n"
    );
}

#[test]
fn associate_aliases_use_an_agreed_project_symbol_case() {
    let source = b"module m\ninteger :: W\ncontains\nsubroutine p\nassociate(w => W)\nx = w\nend associate\nend subroutine p\nend module m\n";
    let project = analyze_project([(Path::new("names.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module m\ninteger :: W\ncontains\nsubroutine p\nassociate(W => W)\nx = W\nend associate\nend subroutine p\nend module m\n"
    );
}

#[test]
fn numeric_kind_suffixes_follow_declared_case_including_exponents() {
    let source = b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_dl\nreal(DL), parameter :: Y = 1.0e8_dl\nend module Constants\n";
    let project = analyze_project([(Path::new("constants.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_DL\nreal(DL), parameter :: Y = 1.0e8_DL\nend module Constants\n"
    );
}

#[test]
fn kind_suffixes_use_project_declarations_and_ignore_digit_kinds() {
    let source = b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\nmodule Values\nuse Kinds\nreal(MyReal) :: x\nx = 1.0_myreal + 2.0_8\nend module Values\n";
    let project = analyze_project([(Path::new("kinds.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\nmodule Values\nuse Kinds\nreal(MyReal) :: x\nx = 1.0_MyReal + 2.0_8\nend module Values\n"
    );
}

#[test]
fn undeclared_kind_suffixes_are_inert() {
    let source = b"program p\nx = 1.0_unknown + 2.0_8\nend program p\n";
    let project = analyze_project([(Path::new("unknown.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}

#[test]
fn unresolved_same_named_components_are_silent() {
    let source = b"module m\ntype :: first\ninteger :: Source\nend type first\ntype :: second\ninteger :: source\nend type second\nunknown_first%source = 1\nunknown_second%Source = 2\nend module m\n";
    let project = analyze_project([(Path::new("component.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        source
    );
}

#[test]
fn inherited_components_shadow_and_preserve_nearest_ambiguity() {
    let source = b"module m\n\
type :: Parent\n\
real :: INTEGRATE_TOL\n\
procedure :: ParentRun\n\
real :: Value\n\
end type Parent\n\
type, extends(Parent) :: Child\n\
real :: VALUE\n\
real :: Ambig\n\
real :: AMBIG\n\
end type Child\n\
contains\n\
subroutine work(this)\n\
class(Child) :: this\n\
type(Unknown) :: unknown\n\
this%integrate_tol = 1\n\
this%value = 2\n\
this%ambig = 3\n\
call this%parentrun()\n\
unknown%INTEGRATE_TOL = 4\n\
end subroutine work\n\
end module m\n";
    let project = analyze_project([(Path::new("inheritance.f90"), source.as_slice())]).unwrap();
    assert!(!project
        .cases
        .components
        .contains(b"child", b"integrate_tol"));
    assert!(project.cases.components.contains(b"child", b"value"));
    assert!(project.cases.components.contains(b"child", b"ambig"));
    assert!(project.cases.components.get(b"child", b"ambig").is_none());
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module m\n\
type :: Parent\n\
real :: INTEGRATE_TOL\n\
procedure :: ParentRun\n\
real :: Value\n\
end type Parent\n\
type, extends(Parent) :: Child\n\
real :: VALUE\n\
real :: Ambig\n\
real :: AMBIG\n\
end type Child\n\
contains\n\
subroutine work(this)\n\
class(Child) :: this\n\
type(Unknown) :: unknown\n\
this%INTEGRATE_TOL = 1\n\
this%VALUE = 2\n\
this%ambig = 3\n\
call this%ParentRun()\n\
unknown%INTEGRATE_TOL = 4\n\
end subroutine work\n\
end module m\n"
    );
}

#[test]
fn powell_bobyqb_has_a_local_case_map() {
    let source = b"module m\ninteger :: MAXFUN\ncontains\nfunction f(THIS, &\n maxfun)\nclass(*) :: this\ninteger :: Maxfun\nthis = maxfun\nend function f\nend module m\n";
    let analysis = Document::from_bytes(source).analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
    let line = source
        .split(|byte| *byte == b'\n')
        .position(|line| line.starts_with(b"function f"))
        .unwrap();
    assert_eq!(
        names.local_at(line).and_then(|map| map.get(b"this")),
        Some(b"this".as_slice())
    );
    assert_eq!(
        names.local_at(line).and_then(|map| map.get(b"maxfun")),
        Some(b"Maxfun".as_slice())
    );
    let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module m\ninteger :: MAXFUN\ncontains\nfunction f(this, &\n Maxfun)\nclass(*) :: this\ninteger :: Maxfun\nthis = Maxfun\nend function f\nend module m\n"
    );
}

#[test]
fn nested_declaration_bounds_use_the_active_procedure_local_case() {
    let source = b"module m\ncontains\nsubroutine first(EV)\ntype(EvolutionVars) EV, EVout\nreal(dl), intent(out) :: yout(EVOut%nvar)\nend subroutine first\nsubroutine second(EV)\ntype(EvolutionVars) EV, EVOut\nreal(dl), intent(out) :: yout(EVout%nvar)\nend subroutine second\nend module m\n";
    let analysis = Document::from_bytes(source).analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
    assert_eq!(
        names.local_at(4).and_then(|map| map.get(b"evout")),
        Some(b"EVout".as_slice())
    );
    assert_eq!(
        names.local_at(8).and_then(|map| map.get(b"evout")),
        Some(b"EVOut".as_slice())
    );
    let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
    assert_eq!(
        run_pass(source, &project, |document, context| {
            declared(document, context).unwrap()
        }),
        b"module m\ncontains\nsubroutine first(EV)\ntype(EvolutionVars) EV, EVout\nreal(dl), intent(out) :: yout(EVout%nvar)\nend subroutine first\nsubroutine second(EV)\ntype(EvolutionVars) EV, EVOut\nreal(dl), intent(out) :: yout(EVOut%nvar)\nend subroutine second\nend module m\n"
    );
}
