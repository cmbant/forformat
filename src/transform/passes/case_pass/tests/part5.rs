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
