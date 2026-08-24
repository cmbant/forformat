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
