use forformat::{format_source, FormatConfig};
use std::{fs, path::PathBuf, process::Command};

#[test]
fn formatted_fixtures_remain_fortran_syntax_valid_when_gfortran_is_available() {
    if Command::new("gfortran").arg("--version").output().is_err() {
        return;
    }
    for name in ["core", "lexical", "procedure_decl", "align", "ws_remred"] {
        let input: &[u8] = match name {
            "core" => include_bytes!("fixtures/core.f90").as_slice(),
            "lexical" => include_bytes!("fixtures/lexical.f90").as_slice(),
            "procedure_decl" => include_bytes!("fixtures/procedure_decl.f90").as_slice(),
            "align" => include_bytes!("fixtures/align.f90").as_slice(),
            "ws_remred" => include_bytes!("fixtures/ws_remred.f90").as_slice(),
            _ => unreachable!(),
        };
        let output = format_source(input, &FormatConfig::default())
            .expect("fixture formats")
            .bytes;
        let path = temp_path(name);
        fs::write(&path, output).unwrap();
        let result = Command::new("gfortran")
            .args([
                "-ffree-form",
                "-ffree-line-length-none",
                "-fopenmp",
                "-fsyntax-only",
            ])
            .arg(&path)
            .current_dir(std::env::temp_dir())
            .output()
            .unwrap();
        let _ = fs::remove_file(&path);
        assert!(
            result.status.success(),
            "gfortran rejected {name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("forformat-smoke-{}-{name}.f90", std::process::id()))
}
