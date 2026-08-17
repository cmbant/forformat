use super::*;

#[test]
fn an_unlimited_polymorphic_declaration_names_no_type() {
    let facts = facts(b"subroutine s(r)\nclass(*), intent(in) :: r\nend subroutine s\n");
    assert!(!facts.cases.types.contains(b"intent"));
    assert!(!facts.cases.symbols.contains(b"intent"));
    assert!(facts.cases.symbols.contains(b"r"));
}

#[test]
fn a_function_statement_is_not_an_old_style_declaration() {
    let facts = facts(
        b"module m\ncontains\ndouble precision function G()\nG = 1\nend function G\nend module m\n",
    );
    assert!(!facts.cases.symbols.contains(b"function"));
    assert!(facts.cases.symbols.contains(b"G"));
}

#[test]
fn scope_names_land_in_their_own_name_spaces() {
    let facts = facts(
        b"module MyModule\n\
          type :: MyType\n\
          end type MyType\n\
        contains\n\
          subroutine DoThing()\n\
          end subroutine DoThing\n\
        end module MyModule\n",
    );
    assert_eq!(
        facts.cases.modules.get(b"mymodule"),
        Some(b"MyModule".as_slice())
    );
    assert_eq!(facts.cases.types.get(b"mytype"), Some(b"MyType".as_slice()));
    assert_eq!(
        facts.cases.symbols.get(b"dothing"),
        Some(b"DoThing".as_slice())
    );
    assert!(facts.cases.symbols.get(b"mymodule").is_none());
}

#[test]
fn use_statements_do_not_invent_module_declarations() {
    let facts = facts(
        b"program p\n\
        use Precision\n\
        use, intrinsic :: ISO_Fortran_env\n\
        use :: Results, only: x\n\
        end program\n",
    );
    assert!(!facts.cases.modules.contains(b"precision"));
    assert!(!facts.cases.modules.contains(b"iso_fortran_env"));
    assert!(!facts.cases.modules.contains(b"results"));
}

#[test]
fn conflicting_spellings_in_one_file_make_the_name_untouchable() {
    let facts = facts(
        b"module Precision\nend module Precision\n\
        module PRECISION\nend module PRECISION\n",
    );
    assert!(facts.cases.modules.is_ambiguous(b"precision"));
    assert_eq!(facts.cases.modules.get(b"precision"), None);
}

#[test]
fn define_directives_contribute_macro_spellings() {
    let facts = facts(
        b"#define FEATURE_FLAG 1\n#  define Has_Fun(x) (x)\n#undef NOPE\nprogram p\nend\n",
    );
    assert_eq!(
        facts.macros.get(b"feature_flag"),
        Some(b"FEATURE_FLAG".as_slice())
    );
    assert_eq!(facts.macros.get(b"has_fun"), Some(b"Has_Fun".as_slice()));
    assert!(!facts.macros.contains(b"nope"));
}

#[test]
fn declared_entities_are_protected_and_typed() {
    let facts = facts(
        b"module M\n\
          type :: LimberRec\n\
            real(dl), dimension(:), allocatable :: Source\n\
            type(ModelParams) :: Params\n\
          contains\n\
            procedure :: Run\n\
          end type LimberRec\n\
          integer :: Data, Count = 0\n\
        end module M\n",
    );
    assert_eq!(
        facts.cases.components.get(b"limberrec", b"source"),
        Some(b"Source".as_slice())
    );
    assert_eq!(
        facts.cases.type_procedures.get(b"run"),
        Some(b"Run".as_slice())
    );
    assert_eq!(facts.cases.symbols.get(b"data"), Some(b"Data".as_slice()));
    assert_eq!(facts.cases.symbols.get(b"count"), Some(b"Count".as_slice()));
    assert_eq!(
        facts.cases.types.get(b"modelparams"),
        Some(b"ModelParams".as_slice())
    );
    assert_eq!(
        facts
            .types
            .component_types
            .get(&(b"limberrec".to_vec(), b"params".to_vec())),
        Some(&b"modelparams".to_vec())
    );
}

#[test]
fn an_initializer_does_not_contribute_names() {
    let facts = facts(b"program p\ninteger :: n = size(Other), m\nend program p\n");
    assert_eq!(facts.cases.symbols.get(b"n"), Some(b"n".as_slice()));
    assert_eq!(facts.cases.symbols.get(b"m"), Some(b"m".as_slice()));
    assert!(!facts.cases.symbols.contains(b"other"));
    assert!(!facts.cases.symbols.contains(b"size"));
}

#[test]
fn component_chains_resolve_through_the_type_maps() {
    let mut types = TypeMaps::default();
    types
        .variable_types
        .insert(b"state".to_vec(), b"modeldata".to_vec());
    types.component_types.insert(
        (b"modeldata".to_vec(), b"params".to_vec()),
        b"modelparams".to_vec(),
    );
    assert_eq!(
        types.resolve_chain(b"State", &[b"Params"]),
        Some(b"modelparams".to_vec())
    );
    assert_eq!(types.resolve_chain(b"state", &[b"missing"]), None);
    assert_eq!(types.resolve_chain(b"unknown", &[]), None);
}

#[test]
fn disagreeing_types_are_dropped_rather_than_guessed_when_merging() {
    let mut a = TypeMaps::default();
    a.variable_types.insert(b"x".to_vec(), b"t1".to_vec());
    let mut b = TypeMaps::default();
    b.variable_types.insert(b"x".to_vec(), b"t2".to_vec());
    a.merge(&b);
    assert!(a.variable_types.is_empty());

    let mut c = TypeMaps::default();
    c.variable_types.insert(b"x".to_vec(), b"t3".to_vec());
    a.merge(&c);
    assert!(a.variable_types.is_empty());
    assert_eq!(a.resolve_chain(b"x", &[]), None);
}

#[test]
fn old_style_class_locals_and_components_feed_the_same_chain() {
    let facts = facts(
        b"type :: T\nreal :: X\nend type T\n\
          subroutine s(this)\nclass(T) this\nthis%x = 1\nend subroutine s\n",
    );
    assert_eq!(
        facts
            .types
            .procedure_local_types
            .get(b"s".as_slice())
            .and_then(|types| types.get(b"this".as_slice())),
        Some(&b"t".to_vec())
    );
    assert_eq!(
        facts.cases.components.get(b"t", b"x"),
        Some(b"X".as_slice())
    );
}

#[test]
fn old_style_declarations_register_entities_but_not_type_statement_words() {
    let facts = facts(b"program p\nreal x, y(3)\ninteger*4 n\ntype is (t)\nend program p\n");
    for name in [b"x".as_slice(), b"y", b"n"] {
        assert!(facts.cases.symbols.contains(name));
    }
    assert!(!facts.cases.symbols.contains(b"is"));
    assert!(!facts.cases.symbols.contains(b"function"));
}

#[test]
fn use_only_renames_and_auxiliary_name_lists_feed_symbols() {
    let facts = facts(
        b"program p\nuse M, only: Local => Remote, Plain\nexternal :: Ext1, Ext2\nintrinsic sin, cos\ncommon /Block/ A, B\nnamelist /Group/ C, D\nentry Enter(X)\nend program p\n",
    );
    for name in [
        b"local".as_slice(),
        b"remote",
        b"plain",
        b"ext1",
        b"ext2",
        b"sin",
        b"cos",
        b"block",
        b"a",
        b"b",
        b"group",
        b"c",
        b"d",
        b"enter",
    ] {
        assert!(facts.cases.symbols.contains(name), "missing {name:?}");
    }
    assert!(!facts.cases.symbols.contains(b"x"));
}

#[test]
fn type_bound_binding_targets_are_not_registered_as_bindings() {
    let facts = facts(
        b"type :: T\nprocedure(iface), pass :: Run => Run_impl\ngeneric :: Op(+) => Add\nfinal :: Cleanup\nend type T\n",
    );
    for name in [b"run".as_slice(), b"op", b"cleanup"] {
        assert!(facts.cases.type_procedures.contains(name));
    }
    for name in [b"run_impl".as_slice(), b"add"] {
        assert!(!facts.cases.type_procedures.contains(name));
        assert!(!facts.cases.symbols.contains(name));
    }
}

#[test]
fn select_type_alias_uses_the_selector_type_for_chains() {
    let facts = facts(
        b"module m\ntype :: T\ninteger :: Value\nend type T\ncontains\nsubroutine s(obj)\nclass(T) :: obj\nselect type (Alias => obj)\nAlias%VALUE = 1\nend select\nend subroutine s\nend module m\n",
    );
    assert_eq!(
        facts
            .types
            .procedure_local_types
            .get(b"s".as_slice())
            .and_then(|types| types.get(b"alias".as_slice())),
        Some(&b"t".to_vec())
    );
    assert_eq!(
        facts.cases.components.get(b"t", b"value"),
        Some(b"Value".as_slice())
    );
}

#[test]
fn component_case_keys_keep_same_names_in_different_types_separate() {
    let facts = facts(
        b"module m\ntype :: First\ninteger :: Tcmb\nend type First\n\
          type :: Second\ninteger :: tcMB\nend type Second\n",
    );
    assert_eq!(
        facts.cases.components.get(b"first", b"tcmb"),
        Some(b"Tcmb".as_slice())
    );
    assert_eq!(
        facts.cases.components.get(b"second", b"tcmb"),
        Some(b"tcMB".as_slice())
    );
}

#[test]
fn extends_records_parentage_without_registering_inherited_components() {
    let facts = facts(
        b"module m\n\
          type :: Leaf\n\
            real :: Value\n\
          end type Leaf\n\
          type :: Parent\n\
            real :: INTEGRATE_TOL\n\
            type(Leaf) :: Nested\n\
          end type Parent\n\
          type, extends(Parent) :: Child\n\
            real :: VALUE\n\
          end type Child\n",
    );
    assert_eq!(
        facts.types.parent_types.get(b"child".as_slice()),
        Some(&b"parent".to_vec())
    );
    assert_eq!(
        facts.types.component_type(b"child", b"nested"),
        Some(b"leaf".to_vec())
    );
    assert!(!facts.cases.components.contains(b"child", b"integrate_tol"));
    assert!(!facts.cases.components.contains(b"child", b"nested"));
}

#[test]
fn inherited_component_type_cycles_and_unknown_parents_are_unresolved() {
    let mut types = TypeMaps::default();
    types.insert_parent(b"A", b"B");
    types.insert_parent(b"B", b"A");
    types.insert_component(b"A", b"nested", b"Leaf");
    assert_eq!(types.component_type(b"A", b"missing"), None);
    assert_eq!(
        types.component_type(b"A", b"nested"),
        Some(b"leaf".to_vec())
    );
    types.insert_parent(b"Ambig", b"A");
    types.insert_parent(b"Ambig", b"B");
    types.insert_component(b"A", b"value", b"Leaf");
    assert_eq!(types.component_type(b"Ambig", b"value"), None);

    let mut unknown = TypeMaps::default();
    unknown.insert_parent(b"Child", b"External");
    assert_eq!(unknown.component_type(b"Child", b"value"), None);
}
