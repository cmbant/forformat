use forformat::{
    analysis::analyze_project, format_source_with_context, FormatConfig, FormatMode, ProjectContext,
};
use std::path::{Path, PathBuf};

fn config() -> FormatConfig {
    let mut config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    config.style.modernize_declarations = true;
    config
}

fn context(sources: &[(PathBuf, Vec<u8>)]) -> ProjectContext {
    analyze_project(
        sources
            .iter()
            .map(|(path, source)| (path.as_path(), source.as_slice())),
    )
    .unwrap()
}

fn format_project_once(sources: &[(PathBuf, Vec<u8>)]) -> Vec<(PathBuf, Vec<u8>)> {
    let project = context(sources);
    let config = config();
    sources
        .iter()
        .map(|(path, source)| {
            (
                path.clone(),
                format_source_with_context(source, &project, &config)
                    .unwrap()
                    .bytes,
            )
        })
        .collect()
}

fn assert_project_fixed_point(sources: Vec<(PathBuf, Vec<u8>)>) -> Vec<(PathBuf, Vec<u8>)> {
    let once = format_project_once(&sources);
    let twice = format_project_once(&once);
    assert_eq!(once, twice);
    once
}

#[test]
fn legacy_dimension_exports_the_same_case_before_and_after_modernization() {
    let once = assert_project_fixed_point(vec![
        (
            Path::new("m.f90").to_path_buf(),
            b"module m\ndimension RADSAV(2)\nend module m\n".to_vec(),
        ),
        (
            Path::new("p.f90").to_path_buf(),
            b"program p\nuse m\nimplicit none\nprint *, radsav(1)\nend program p\n".to_vec(),
        ),
    ]);
    let program = &once[1].1;
    assert!(program
        .windows(b"RADSAV(1)".len())
        .any(|window| window == b"RADSAV(1)"));
}

#[test]
fn public_accessibility_does_not_override_use_associated_spelling() {
    let once = assert_project_fixed_point(vec![
        (
            Path::new("base.f90").to_path_buf(),
            b"module base\ninteger :: ExportedName\nend module base\n".to_vec(),
        ),
        (
            Path::new("middle.f90").to_path_buf(),
            b"module middle\nuse base, only: ExportedName\npublic exportedname\ncontains\nsubroutine s\nprint *, EXPORTEDNAME\nend subroutine s\nend module middle\n".to_vec(),
        ),
    ]);
    let middle = &once[1].1;
    assert!(middle
        .windows(b"print *, ExportedName".len())
        .any(|window| window == b"print *, ExportedName"));
}
