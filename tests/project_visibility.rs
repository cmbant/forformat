use forformat::{analyze_project, format_source_with_context, FormatConfig, FormatMode};
use std::path::Path;

fn normalize<'a, I>(target: &[u8], sources: I) -> Vec<u8>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let project = analyze_project(sources).unwrap();
    format_source_with_context(
        target,
        &project,
        &FormatConfig {
            mode: FormatMode::NormalizeOnly,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes
}

#[test]
fn private_module_names_do_not_poison_public_import_case() {
    let private = b"module PrivateNames\nprivate\ninteger :: PUBLICCASE\nend module PrivateNames\n";
    let public = b"module PublicNames\ninteger :: PublicCase\nend module PublicNames\n";
    let target = b"program p\nuse PublicNames\nimplicit none\nprint *, publiccase\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("private.f90"), private.as_slice()),
            (Path::new("public.f90"), public.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(String::from_utf8(output)
        .unwrap()
        .contains("print *, PublicCase"));
}

#[test]
fn unrelated_public_module_names_do_not_make_an_import_ambiguous() {
    let relevant = b"module Relevant\ninteger :: RelevantCase\nend module Relevant\n";
    let unrelated = b"module Unrelated\ninteger :: RELEVANTcase\nend module Unrelated\n";
    let target = b"program p\nuse Relevant\nimplicit none\nprint *, relevantcase\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("relevant.f90"), relevant.as_slice()),
            (Path::new("unrelated.f90"), unrelated.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, RelevantCase"));
}

#[test]
fn module_accessibility_applies_to_default_and_declaration_attributes() {
    let module = b"module Access\nprivate\ninteger :: HiddenCase\ninteger, public :: ExposedCase\ninteger :: AlsoCase\npublic :: AlsoCase\nend module Access\n";
    let target = b"program p\nuse Access\nimplicit none\nprint *, hiddencase, exposedcase, alsocase\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("access.f90"), module.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, hiddencase, ExposedCase, AlsoCase"));
}

#[test]
fn use_association_is_scoped_to_each_program_unit() {
    let a = b"module A\ninteger :: SharedName\nend module A\n";
    let b = b"module B\ninteger :: SHAREDname\nend module B\n";
    let target = b"module First\nuse A\nimplicit none\ncontains\nsubroutine one\nprint *, sharedname\nend subroutine one\nend module First\nmodule Second\nuse B\nimplicit none\ncontains\nsubroutine two\nprint *, sharedname\nend subroutine two\nend module Second\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("a.f90"), a.as_slice()),
            (Path::new("b.f90"), b.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("subroutine one\nprint *, SharedName"));
    assert!(output.contains("subroutine two\nprint *, SHAREDname"));
}

#[test]
fn transitive_use_exports_keep_variable_type_and_component_owner() {
    let base = b"module Base\ntype :: ModelState\ninteger :: FieldCase\nend type ModelState\ntype(ModelState) :: StateCase\nend module Base\n";
    let middle = b"module Middle\nuse Base\nend module Middle\n";
    let top = b"module Top\nuse Middle\nend module Top\n";
    let target =
        b"program p\nuse Top\nimplicit none\nprint *, statecase%fieldcase\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("top.f90"), top.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, StateCase%FieldCase"));
}

#[test]
fn private_on_an_intermediate_module_cuts_transitive_visibility() {
    let base = b"module Base\ntype :: ModelState\ninteger :: FieldCase\nend type ModelState\ntype(ModelState) :: StateCase\nend module Base\n";
    let middle = b"module Middle\nuse Base\nprivate :: StateCase\nend module Middle\n";
    let top = b"module Top\nuse Middle\nend module Top\n";
    let target =
        b"program p\nuse Top\nimplicit none\nprint *, statecase%fieldcase\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("top.f90"), top.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, statecase%fieldcase"));
}

#[test]
fn transitive_rename_preserves_the_local_alias_and_remote_component() {
    let base = b"module Base\ntype :: ModelState\ninteger :: FieldCase\nend type ModelState\ntype(ModelState) :: StateCase\nend module Base\n";
    let middle = b"module Middle\nuse Base, only: AliasState => StateCase\nend module Middle\n";
    let target =
        b"program p\nuse Middle\nimplicit none\nprint *, aliasstate%fieldcase\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, AliasState%FieldCase"));
}

#[test]
fn include_fragments_participate_in_module_exports() {
    let base = b"module Base\ninclude 'defs.inc'\nend module Base\n";
    let include = b"type :: IncludedState\ninteger :: IncludedField\nend type IncludedState\ntype(IncludedState) :: IncludedRoot\n";
    let target =
        b"program p\nuse Base\nimplicit none\nprint *, includedroot%includedfield\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("defs.inc"), include.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, IncludedRoot%IncludedField"));
}

#[test]
fn include_accessibility_and_nested_relative_paths_are_respected() {
    let base = b"module Base\ninclude 'inc/defs.inc'\nend module Base\n";
    let defs =
        b"include '../types.inc'\nprivate :: IncludedRoot\ntype(IncludedState) :: IncludedRoot\n";
    let types = b"type :: IncludedState\ninteger :: IncludedField\nend type IncludedState\n";
    let target =
        b"program p\nuse Base\nimplicit none\nprint *, includedroot%includedfield\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("src/base.f90"), base.as_slice()),
            (Path::new("src/inc/defs.inc"), defs.as_slice()),
            (Path::new("src/types.inc"), types.as_slice()),
            (Path::new("src/target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, includedroot%includedfield"));
}

#[test]
fn include_dependencies_are_loaded_from_the_source_directory() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "forformat-project-visibility-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("inc")).unwrap();
    let base_path = root.join("base.f90");
    let include_path = root.join("inc/defs.inc");
    let target_path = root.join("target.f90");
    let base = b"module Base\ninclude 'inc/defs.inc'\nend module Base\n";
    let include = b"type :: IncludedState\ninteger :: IncludedField\nend type IncludedState\ntype(IncludedState) :: IncludedRoot\n";
    let target =
        b"program p\nuse Base\nimplicit none\nprint *, includedroot%includedfield\nend program p\n";
    fs::write(&base_path, base).unwrap();
    fs::write(&include_path, include).unwrap();

    let output = String::from_utf8(normalize(
        target,
        [
            (base_path.as_path(), base.as_slice()),
            (target_path.as_path(), target.as_slice()),
        ],
    ))
    .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert!(output.contains("print *, IncludedRoot%IncludedField"));
}

#[test]
fn include_fragment_types_follow_the_containing_procedure_scope() {
    let source = b"module Host\ncontains\nsubroutine Work\ninclude 'local.inc'\nprint *, state%fieldcase\nend subroutine Work\nend module Host\n";
    let include = b"type :: LocalState\ninteger :: FieldCase\nend type LocalState\ntype(LocalState) :: State\n";
    let unrelated = b"module Other\ntype :: OtherState\ninteger :: FIELDcase\nend type OtherState\ntype(OtherState) :: State\nend module Other\n";
    let output = String::from_utf8(normalize(
        source,
        [
            (Path::new("host.f90"), source.as_slice()),
            (Path::new("local.inc"), include.as_slice()),
            (Path::new("other.f90"), unrelated.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, State%FieldCase"));
}

#[test]
fn same_named_procedures_keep_their_own_variable_type_scope() {
    let source = b"module Left\ntype :: LeftState\ninteger :: LeftField\nend type LeftState\ncontains\nsubroutine Work\ntype(LeftState) :: State\nprint *, state%leftfield\nend subroutine Work\nend module Left\nmodule Right\ntype :: RightState\ninteger :: RightField\nend type RightState\ncontains\nsubroutine Work\ntype(RightState) :: State\nprint *, state%rightfield\nend subroutine Work\nend module Right\n";
    let output = String::from_utf8(normalize(
        source,
        [(Path::new("same-name.f90"), source.as_slice())],
    ))
    .unwrap();
    assert!(output.contains("print *, State%LeftField"));
    assert!(output.contains("print *, State%RightField"));
}

#[test]
fn identical_sources_keep_directory_relative_include_identity() {
    let target = b"program p\ninclude 'defs.inc'\nprint *, valuecase\nend program p\n";
    let a_defs = b"integer :: ValueCase\n";
    let b_defs = b"integer :: VALUECASE\n";
    let project = analyze_project([
        (Path::new("a/target.f90"), target.as_slice()),
        (Path::new("a/defs.inc"), a_defs.as_slice()),
        (Path::new("b/target.f90"), target.as_slice()),
        (Path::new("b/defs.inc"), b_defs.as_slice()),
    ])
    .unwrap();
    let config = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };

    let a = String::from_utf8(
        project
            .format_source_at(Path::new("a/target.f90"), target, &config)
            .unwrap()
            .bytes,
    )
    .unwrap();
    let b = String::from_utf8(
        project
            .format_source_at(Path::new("b/target.f90"), target, &config)
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(a.contains("print *, ValueCase"));
    assert!(b.contains("print *, VALUECASE"));

    // A pathless buffer is intentionally conservative when identical project
    // members have different include expansions: it must not pick either one.
    let pathless = String::from_utf8(
        format_source_with_context(target, &project, &config)
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(pathless.contains("print *, valuecase"));
}

#[test]
fn unrelated_same_named_types_do_not_ambiguate_visible_member_owner() {
    let a = b"module A\ntype :: Child\ninteger :: LeafName\nend type Child\ntype :: State\ntype(Child) :: ChildLink\ninteger :: FieldName\nend type State\ntype(State) :: Root\nend module A\n";
    let b = b"module B\ntype :: Child\ninteger :: LEAFNAME\nend type Child\ntype :: State\ntype(Child) :: CHILDLINK\ninteger :: FIELDNAME\nend type State\ntype(State) :: ROOT\nend module B\n";
    let target = b"program p\nuse A\nimplicit none\nprint *, root%fieldname, root%childlink%leafname\nend program p\n";
    let output = String::from_utf8(normalize(
        target,
        [
            (Path::new("a.f90"), a.as_slice()),
            (Path::new("b.f90"), b.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    ))
    .unwrap();
    assert!(output.contains("print *, Root%FieldName, Root%ChildLink%LeafName"));
}

#[test]
fn submodules_follow_semantic_parent_hosts_including_private_entities() {
    let parent = b"module Parent\nprivate\ninteger :: HostValue\ntype :: SecretState\ninteger :: FieldCase\nend type SecretState\ntype(SecretState) :: State\npublic :: work\ninterface\nmodule subroutine work()\nend subroutine work\nend interface\nend module Parent\n";
    let middle = b"submodule (Parent) Mid\ninteger :: MidValue\nend submodule Mid\n";
    let leaf = b"submodule (Parent:Mid) Impl\ncontains\nmodule procedure work\nprint *, hostvalue, midvalue, state%fieldcase\nend procedure\nend submodule Impl\n";
    let output = String::from_utf8(normalize(
        leaf,
        [
            (Path::new("parent.f90"), parent.as_slice()),
            (Path::new("mid.f90"), middle.as_slice()),
            (Path::new("impl.f90"), leaf.as_slice()),
        ],
    ))
    .unwrap();
    assert!(
        output.contains("print *, HostValue, MidValue, State%FieldCase"),
        "{output}"
    );
}

#[test]
fn import_controls_host_association_and_interface_defaults() {
    let source = b"module ImportHost\ninteger :: HostName, OtherName\ntype :: HostType\nend type HostType\ninterface\nsubroutine Plain(arg)\ntype(hosttype) :: arg\nend subroutine Plain\nsubroutine NamedIface(arg)\nimport :: HostType\ntype(hosttype) :: arg\nend subroutine NamedIface\nmodule subroutine ModuleIface(arg)\ntype(hosttype) :: arg\nend subroutine ModuleIface\nend interface\ncontains\nsubroutine NoneCase\nimport, none\nprint *, hostname, othername\nend subroutine NoneCase\nsubroutine NamedCase\nimport :: HostName\nprint *, hostname, othername\nend subroutine NamedCase\nsubroutine OnlyCase\nimport, only: HostName\nprint *, hostname, othername\nend subroutine OnlyCase\nsubroutine AllCase\nimport, all\nprint *, hostname, othername\nend subroutine AllCase\nend module ImportHost\n";
    let output = String::from_utf8(normalize(
        source,
        [(Path::new("imports.f90"), source.as_slice())],
    ))
    .unwrap();

    assert!(output.contains("subroutine Plain(arg)\ntype(hosttype) :: arg"));
    assert!(
        output.contains("subroutine NamedIface(arg)\nimport :: HostType\ntype(HostType) :: arg")
    );
    assert!(output.contains("module subroutine ModuleIface(arg)\ntype(HostType) :: arg"));
    assert!(
        output.contains("subroutine NoneCase\nimport, none\nprint *, hostname, othername"),
        "{output}"
    );
    assert!(
        output.contains("subroutine NamedCase\nimport :: HostName\nprint *, HostName, othername")
    );
    assert!(output
        .contains("subroutine OnlyCase\nimport, only: HostName\nprint *, HostName, othername"));
    assert!(output.contains("subroutine AllCase\nimport, all\nprint *, HostName, OtherName"));
}
