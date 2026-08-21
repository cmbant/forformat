use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_forformat"))
}

fn run(args: &[&str], source: &[u8]) -> Output {
    let mut child = Command::new(binary())
        .args(args)
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_reduce_whitespace_matches_findent_alias() {
    let source = b"program p\ncall   work\nend program p\n";
    let native = run(
        &[
            "--input-format=free",
            "--indent-only",
            "--reduce-whitespace",
        ],
        source,
    );
    let legacy = run(
        &["--input-format=free", "--indent-only", "--ws_remred"],
        source,
    );
    let baseline = run(&["--input-format=free", "--indent-only"], source);

    assert_success(&native);
    assert_success(&legacy);
    assert_success(&baseline);
    assert_eq!(native.stdout, legacy.stdout);
    assert_ne!(native.stdout, baseline.stdout);
    assert!(
        String::from_utf8_lossy(&native.stdout).contains("call work"),
        "{}",
        String::from_utf8_lossy(&native.stdout)
    );
    assert!(
        String::from_utf8_lossy(&baseline.stdout).contains("call   work"),
        "{}",
        String::from_utf8_lossy(&baseline.stdout)
    );
}

#[test]
fn canonicalize_only_ignores_reduce_whitespace() {
    let source = b"program p\r\nflag  =  x  .EQ.  y   ! gap\r\nend program p\r\n";
    let canonical = run(&["--input-format=free", "--canonicalize-only"], source);
    let with_reduction = run(
        &[
            "--input-format=free",
            "--canonicalize-only",
            "--reduce-whitespace",
        ],
        source,
    );

    assert_success(&canonical);
    assert_success(&with_reduction);
    assert_eq!(with_reduction.stdout, canonical.stdout);
    assert!(
        String::from_utf8_lossy(&canonical.stdout).contains("flag  =  x  ==  y   ! gap"),
        "{}",
        String::from_utf8_lossy(&canonical.stdout)
    );
    assert!(canonical.stdout.windows(2).any(|window| window == b"\r\n"));
}
