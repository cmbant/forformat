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
