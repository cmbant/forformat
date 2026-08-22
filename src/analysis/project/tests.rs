use super::{
    analyze_file, analyze_file_at, analyze_project, includes::normalize_path, ProjectContext,
};
use crate::{
    analysis::names::NameSpace,
    config::{FormatConfig, FormatMode, MacroDefine},
    format_source_with_context,
};
use std::path::Path;

const MODULE: &[u8] = br#"module Precision
integer, parameter :: dl = 8
end module Precision
"#;
const USER: &[u8] = br#"program p
use Precision
end program p
"#;
const SHOUTER: &[u8] = br#"module PRECISION
end module PRECISION
"#;

#[test]
fn lexical_path_normalization_preserves_unmatched_parents() {
    assert_eq!(
        normalize_path(Path::new("../foo.f90")),
        Path::new("../foo.f90")
    );
    assert_ne!(
        normalize_path(Path::new("../foo.f90")),
        normalize_path(Path::new("foo.f90"))
    );
    assert_eq!(
        normalize_path(Path::new("src/sub/../../../defs.inc")),
        Path::new("../defs.inc")
    );
}

#[test]
fn include_paths_above_relative_source_directory_do_not_alias_siblings() {
    let target = br#"program p
include '../../../defs.inc'
print *, valuecase
end program p
"#;
    let project = analyze_project([
        (Path::new("src/sub/target.f90"), target.as_slice()),
        (
            Path::new("../defs.inc"),
            br#"integer :: ValueCase
"#.as_slice(),
        ),
        (
            Path::new("defs.inc"),
            br#"integer :: VALUECASE
"#.as_slice(),
        ),
    ])
    .unwrap();
    let local = analyze_file_at(Path::new("src/sub/target.f90"), target).unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 2, b"valuecase"),
        Some(b"ValueCase".to_vec())
    );
}

#[test]
fn a_project_wide_agreement_applies_to_a_file_that_does_not_declare_the_name() {
    let project = analyze_project([
        (Path::new("precision.f90"), MODULE),
        (Path::new("user.f90"), USER),
    ])
    .unwrap();
    let local = analyze_file(br#"program r
end program r
"#)
    .unwrap();
    let resolver = project.resolver(&local);
    assert_eq!(
        resolver.spelling(NameSpace::Module, b"precision"),
        Some(b"Precision".as_slice())
    );
    assert_eq!(
        project.sources,
        vec![Path::new("precision.f90"), Path::new("user.f90")]
    );
}

#[test]
fn project_wide_disagreement_leaves_the_name_alone() {
    let project =
        analyze_project([(Path::new("a.f90"), MODULE), (Path::new("b.f90"), SHOUTER)]).unwrap();
    let local = analyze_file(br#"program r
end program r
"#)
    .unwrap();
    assert_eq!(
        project
            .resolver(&local)
            .spelling(NameSpace::Module, b"precision"),
        None
    );
}

#[test]
fn a_local_spelling_still_wins_over_the_project() {
    let project = analyze_project([(Path::new("a.f90"), MODULE)]).unwrap();
    let local = analyze_file(br#"module PRECISION
end module PRECISION
"#)
    .unwrap();
    assert_eq!(
        project
            .resolver(&local)
            .spelling(NameSpace::Module, b"precision"),
        Some(b"PRECISION".as_slice())
    );
}

#[test]
fn merging_is_order_independent_for_the_case_tables() {
    let forward = analyze_project([(Path::new("a"), MODULE), (Path::new("b"), USER)]).unwrap();
    let backward = analyze_project([(Path::new("b"), USER), (Path::new("a"), MODULE)]).unwrap();
    assert_eq!(forward.cases, backward.cases);
}

#[test]
fn command_line_defines_join_the_macro_table() {
    let mut project = ProjectContext::empty();
    project.define(&[MacroDefine {
        name: "MPI_Enabled".to_string(),
        value: None,
    }]);
    let local = analyze_file(br#"program p
end
"#)
    .unwrap();
    assert_eq!(
        project
            .resolver(&local)
            .spelling(NameSpace::Symbol, b"mpi_enabled"),
        Some(b"MPI_Enabled".as_slice())
    );
}

#[test]
fn synthetic_project_cases_cover_local_and_project_precedence() {
    let declared = analyze_project([(
        Path::new("declared.f90"),
        br#"module SharedName
end module SharedName
"#.as_slice(),
    )])
    .unwrap();
    let no_local = analyze_file(br#"program p
end program p
"#)
    .unwrap();
    assert_eq!(
        declared
            .resolver(&no_local)
            .spelling(NameSpace::Module, b"sharedname"),
        Some(b"SharedName".as_slice())
    );

    let split = analyze_project([
        (
            Path::new("a.f90"),
            br#"module SplitName
end module
"#.as_slice(),
        ),
        (
            Path::new("b.f90"),
            br#"module SPLITNAME
end module
"#.as_slice(),
        ),
    ])
    .unwrap();
    assert_eq!(
        split
            .resolver(&no_local)
            .spelling(NameSpace::Module, b"splitname"),
        None
    );

    let project = analyze_project([(
        Path::new("global.f90"),
        br#"module M
integer :: Colliding
end module M
"#.as_slice(),
    )])
    .unwrap();
    let local = analyze_file(br#"module Local
integer :: COLLIDING
end module Local
"#)
    .unwrap();
    assert_eq!(
        project
            .resolver(&local)
            .spelling(NameSpace::Symbol, b"colliding"),
        Some(b"COLLIDING".as_slice())
    );

    let component_project = analyze_project([(
        Path::new("component.f90"),
        br#"module C
type :: T
integer :: Component
end type T
end module C
"#.as_slice(),
    )])
    .unwrap();
    let component_local = analyze_file(br#"module L
type :: T
integer :: COMPONENT
end type T
integer :: Component
end module L
"#)
    .unwrap();
    let resolver = component_project.resolver(&component_local);
    assert_eq!(
        resolver.component_spelling(b"t", b"component"),
        Some(b"COMPONENT".as_slice())
    );
}

#[test]
fn program_top_level_spelling_still_wins_over_a_module() {
    let program = br#"program validation
integer, parameter :: BJL_RECURRENCE_MAX_L = 25
contains
subroutine check
integer :: value
value = bjl_recurrence_max_l
end subroutine check
end program validation
"#;
    let module = br#"module bessel
integer, parameter :: BJL_recurrence_MAX_L = 25
contains
subroutine check
integer :: value
value = bjl_recurrence_max_l
end subroutine check
end module bessel
"#;
    let project = analyze_project([
        (Path::new("program.f90"), program.as_slice()),
        (Path::new("module.f90"), module.as_slice()),
    ])
    .unwrap();
    let config = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };
    let program_output = format_source_with_context(program, &project, &config)
        .unwrap()
        .bytes;
    let module_output = format_source_with_context(module, &project, &config)
        .unwrap()
        .bytes;
    let program_use = program_output
        .split(|byte| *byte == b'\n')
        .find(|line| line.starts_with(b"value ="))
        .unwrap();
    let module_use = module_output
        .split(|byte| *byte == b'\n')
        .find(|line| line.starts_with(b"value ="))
        .unwrap();
    assert_eq!(program_use, b"value = BJL_RECURRENCE_MAX_L");
    assert_eq!(module_use, b"value = BJL_recurrence_MAX_L");
}

#[test]
fn private_and_unrelated_modules_do_not_enter_visible_symbol_scope() {
    let api = br#"module api
integer :: PublicName
end module api
"#;
    let hidden = br#"module hidden
private
integer :: PUBLICNAME
end module hidden
"#;
    let unrelated = br#"module unrelated
integer :: publicNAME
end module unrelated
"#;
    let target = br#"program p
use api
implicit none
print *, publicname
end program p
"#;
    let project = analyze_project([
        (Path::new("api.f90"), api.as_slice()),
        (Path::new("hidden.f90"), hidden.as_slice()),
        (Path::new("unrelated.f90"), unrelated.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .unwrap();
    let local = analyze_file(target).unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 3, b"publicname"),
        Some(b"PublicName".to_vec())
    );
}

#[test]
fn module_exports_follow_transitive_use_and_intermediate_private() {
    let base = br#"module base
integer :: ExportedName
end module base
"#;
    let middle = br#"module middle
use base
end module middle
"#;
    let top = br#"module top
use middle
end module top
"#;
    let target = br#"program p
use top
print *, exportedname
end program p
"#;
    let project = analyze_project([
        (Path::new("base.f90"), base.as_slice()),
        (Path::new("middle.f90"), middle.as_slice()),
        (Path::new("top.f90"), top.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .unwrap();
    let local = analyze_file(target).unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 2, b"exportedname"),
        Some(b"ExportedName".to_vec())
    );

    let middle_private = br#"module middle
use base
private :: ExportedName
end module middle
"#;
    let project = analyze_project([
        (Path::new("base.f90"), base.as_slice()),
        (Path::new("middle.f90"), middle_private.as_slice()),
        (Path::new("top.f90"), top.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 2, b"exportedname"),
        None
    );
}

#[test]
fn transitive_renames_keep_the_local_export_spelling() {
    let base = br#"module base
integer :: OriginalName
end module base
"#;
    let middle = br#"module middle
use base, only: MiddleName => OriginalName
end module middle
"#;
    let top = br#"module top
use middle, only: TopName => MiddleName
end module top
"#;
    let target = br#"program p
use top
print *, topname
end program p
"#;
    let project = analyze_project([
        (Path::new("base.f90"), base.as_slice()),
        (Path::new("middle.f90"), middle.as_slice()),
        (Path::new("top.f90"), top.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .unwrap();
    let local = analyze_file(target).unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 2, b"topname"),
        Some(b"TopName".to_vec())
    );
}

#[test]
fn uses_are_scoped_to_the_program_unit_that_owns_them() {
    let first = br#"module first
integer :: SharedName
end module first
"#;
    let second = br#"module second
integer :: SHAREDNAME
end module second
"#;
    let target = br#"module left
use first
contains
subroutine l
print *, sharedname
end
end module left
module right
use second
contains
subroutine r
print *, sharedname
end
end module right
"#;
    let project = analyze_project([
        (Path::new("first.f90"), first.as_slice()),
        (Path::new("second.f90"), second.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .unwrap();
    let local = analyze_file(target).unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 4, b"sharedname"),
        Some(b"SharedName".to_vec())
    );
    assert_eq!(
        project.visible_symbol_spelling(&local, 10, b"sharedname"),
        Some(b"SHAREDNAME".to_vec())
    );
}

#[test]
fn block_variable_types_do_not_escape_the_construct() {
    let source = br#"module m
type :: First
integer :: A
end type First
type :: Second
integer :: B
end type Second
contains
subroutine s
type(Second) :: item
block
type(First) :: item
item%A = 1
end block
item%B = 2
end subroutine s
end module m
"#;
    let project = analyze_project([(Path::new("scope.f90"), source.as_slice())]).unwrap();
    let local = analyze_file(source).unwrap();
    assert_eq!(
        project
            .visible_variable_type(&local, 12, b"item")
            .map(|ty| ty.name),
        Some(b"first".to_vec())
    );
    assert_eq!(
        project
            .visible_variable_type(&local, 14, b"item")
            .map(|ty| ty.name),
        Some(b"second".to_vec())
    );
}

#[test]
fn include_fragments_join_the_host_scope_and_obey_module_accessibility() {
    let host = br#"module host
private
include 'parts/decls.inc'
public :: IncludedName
end module host
"#;
    let decls = br#"include '../nested.inc'
integer :: HiddenName
"#;
    let nested = br#"integer :: IncludedName
"#;
    let target = br#"program p
use host
print *, includedname
print *, hiddenname
end program p
"#;
    let project = analyze_project([
        (Path::new("src/host.f90"), host.as_slice()),
        (Path::new("src/parts/decls.inc"), decls.as_slice()),
        (Path::new("src/nested.inc"), nested.as_slice()),
        (Path::new("src/target.f90"), target.as_slice()),
    ])
    .unwrap();
    let local = analyze_file(target).unwrap();
    assert_eq!(
        project.visible_symbol_spelling(&local, 2, b"includedname"),
        Some(b"IncludedName".to_vec())
    );
    assert_eq!(
        project.visible_symbol_spelling(&local, 3, b"hiddenname"),
        None
    );
}
