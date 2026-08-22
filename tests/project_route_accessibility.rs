use forformat::{analyze_project, format_source_with_context, FormatConfig, FormatMode};
use std::path::Path;

fn normalize<'a, I>(target: &[u8], sources: I) -> String
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let project = analyze_project(sources).unwrap();
    String::from_utf8(
        format_source_with_context(
            target,
            &project,
            &FormatConfig {
                mode: FormatMode::NormalizeOnly,
                ..FormatConfig::default()
            },
        )
        .unwrap()
        .bytes,
    )
    .unwrap()
}

#[test]
fn private_named_use_route_blocks_reexport() {
    let base = b"module Base\ninteger :: ExportedName\nend module Base\n";
    let middle = b"module Middle\nuse Base\nprivate :: Base\nend module Middle\n";
    let target = b"program p\nuse Middle\nprint *, exportedname\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("print *, exportedname"));
}

#[test]
fn public_named_use_route_overrides_private_default() {
    let base = b"module Base\ninteger :: ExportedName\nend module Base\n";
    let middle = b"module Middle\nuse Base\nprivate\npublic :: Base\nend module Middle\n";
    let target = b"program p\nuse Middle\nprint *, exportedname\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("print *, ExportedName"));
}

#[test]
fn explicit_public_entity_overrides_private_route() {
    let base = b"module Base\ninteger :: ExportedName\nend module Base\n";
    let middle =
        b"module Middle\nuse Base\nprivate :: Base\npublic :: ExportedName\nend module Middle\n";
    let target = b"program p\nuse Middle\nprint *, exportedname\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("print *, ExportedName"));
}

#[test]
fn named_use_routes_are_merged_independently() {
    let first = b"module First\ninteger :: SharedName\nend module First\n";
    let second = b"module Second\ninteger :: SHAREDNAME\nend module Second\n";
    let middle = b"module Middle\nuse First\nuse Second\nprivate :: First\npublic :: Second\nend module Middle\n";
    let target = b"program p\nuse Middle\nprint *, sharedname\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("first.f90"), first.as_slice()),
            (Path::new("second.f90"), second.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("print *, SHAREDNAME"));

    let all_private =
        b"module Middle\nuse First\nuse Second\nprivate :: First, Second\nend module Middle\n";
    let output = normalize(
        target,
        [
            (Path::new("first.f90"), first.as_slice()),
            (Path::new("second.f90"), second.as_slice()),
            (Path::new("middle.f90"), all_private.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("print *, sharedname"));
}

#[test]
fn route_accessibility_does_not_hide_same_spelled_local_entity() {
    let base = b"module Base\ninteger :: OtherName\nend module Base\n";
    let middle = b"module Middle\nuse Base\ninteger :: Base\nprivate :: Base\nend module Middle\n";
    let target = b"program p\nuse Middle\nprint *, base\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), middle.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("print *, Base"));
}

#[test]
fn named_use_route_gates_type_and_component_identity() {
    let base = b"module Base\ntype :: ModelState\ninteger :: FieldCase\nend type ModelState\ntype(ModelState) :: StateCase\nend module Base\n";
    let hidden = b"module Middle\nuse Base\nprivate :: Base\nend module Middle\n";
    let target = b"program p\nuse Middle\ntype(modelstate) :: item\nprint *, statecase%fieldcase\nend program p\n";
    let output = normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), hidden.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("type(modelstate) :: item"));
    assert!(output.contains("print *, statecase%fieldcase"));

    let exposed = b"module Middle\nuse Base\nprivate\npublic :: Base\nend module Middle\n";
    let output = normalize(
        target,
        [
            (Path::new("base.f90"), base.as_slice()),
            (Path::new("middle.f90"), exposed.as_slice()),
            (Path::new("target.f90"), target.as_slice()),
        ],
    );
    assert!(output.contains("type(ModelState) :: item"));
    assert!(output.contains("print *, StateCase%FieldCase"));
}
