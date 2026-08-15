use forformat::{cli, format_source, FormatError};
use std::{fs, path::PathBuf};

#[derive(Debug, Default)]
struct Case {
    name: String,
    source_test: String,
    oracle: String,
    category: String,
    support: String,
    normalization: String,
    input: String,
    stdout: String,
    stderr: String,
    status: i32,
    args: String,
    /// Formatting mode this case pins. A checked-in case remains explicit so
    /// its expected bytes do not depend on the CLI default.
    mode: String,
}

#[test]
fn checked_in_manifest_covers_success_and_rejection_paths() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/manifests/core.manifest");
    let cases = parse_manifest(&manifest, &fs::read_to_string(&manifest).unwrap());
    assert_eq!(cases.len(), 54);
    for case in cases {
        assert!(!case.source_test.is_empty(), "{} source test", case.name);
        assert!(!case.oracle.is_empty(), "{} oracle", case.name);
        assert!(!case.category.is_empty(), "{} category", case.name);
        assert!(!case.support.is_empty(), "{} support", case.name);
        assert!(
            !case.normalization.is_empty(),
            "{} normalization",
            case.name
        );
        let input = fs::read(manifest.parent().unwrap().join(&case.input)).unwrap();
        let expected_stdout = if case.stdout.is_empty() {
            Vec::new()
        } else {
            fs::read(manifest.parent().unwrap().join(&case.stdout)).unwrap()
        };
        let argv = std::iter::once("forformat".to_string())
            .chain(case.args.split_whitespace().map(str::to_owned));
        let mode = match case.mode.as_str() {
            "" | "indent-only" => forformat::FormatMode::IndentOnly,
            "normalize-only" => forformat::FormatMode::NormalizeOnly,
            "full" => forformat::FormatMode::Full,
            other => panic!("unknown manifest mode {other} in case {}", case.name),
        };
        let (stdout, stderr, status) = match cli::parse(argv) {
            Ok(cli::Command::Run(mut invocation)) => {
                invocation.config.mode = mode;
                match format_source(&input, &invocation.config) {
                    Ok(result) => (result.bytes, String::new(), 0),
                    Err(error) => (Vec::new(), format_error(error), 1),
                }
            }
            Ok(cli::Command::Help) | Ok(cli::Command::Version) => (Vec::new(), String::new(), 0),
            Err(error) => (Vec::new(), format_error(error), 2),
        };
        assert_eq!(stdout, expected_stdout, "case {} stdout", case.name);
        assert_eq!(stderr, case.stderr, "case {} stderr", case.name);
        assert_eq!(status, case.status, "case {} status", case.name);
    }
}

#[test]
fn fixed_form_rejection_contract() {
    let argv = ["forformat", "-ifixed"].into_iter().map(str::to_owned);
    match cli::parse(argv) {
        Err(error) => assert_eq!(
            format_error(error),
            "forformat: unsupported: fixed-form input/output is not supported\n"
        ),
        Ok(_) => panic!("expected fixed-form rejection, got Ok"),
    }
}

#[test]
fn unknown_option_rejection_contract() {
    let argv = ["forformat", "--not-an-option"]
        .into_iter()
        .map(str::to_owned);
    match cli::parse(argv) {
        Err(error) => assert_eq!(
            format_error(error),
            "forformat: invalid option: --not-an-option\n"
        ),
        Ok(_) => panic!("expected unknown-option rejection, got Ok"),
    }
}

fn format_error(error: FormatError) -> String {
    format!("forformat: {error}\n")
}

fn parse_manifest(path: &std::path::Path, source: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut current = None;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix("[case.")
            .and_then(|s| s.strip_suffix(']'))
        {
            if let Some(case) = current.take() {
                cases.push(case);
            }
            current = Some(Case {
                name: name.to_string(),
                ..Case::default()
            });
            continue;
        }
        let (key, value) = line.split_once('=').expect("manifest key/value");
        let value = unquote(value.trim());
        let case = current.as_mut().expect("manifest key outside case");
        match key.trim() {
            "source_test" => case.source_test = value,
            "oracle" => case.oracle = value,
            "category" => case.category = value,
            "support" => case.support = value,
            "normalization" => case.normalization = value,
            "input" => case.input = value,
            "stdout" => case.stdout = value,
            "stderr" => case.stderr = value,
            "status" => case.status = value.parse().expect("manifest status"),
            "args" => case.args = value,
            "mode" => case.mode = value,
            other => panic!("unknown manifest key {other} in {}", path.display()),
        }
    }
    if let Some(case) = current {
        cases.push(case);
    }
    cases
}

fn unquote(value: &str) -> String {
    let value = value.strip_prefix('"').unwrap_or(value);
    let value = value.strip_suffix('"').unwrap_or(value);
    value.replace("\\n", "\n").replace("\\\"", "\"")
}
