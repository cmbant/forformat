use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_forformat"))
}

fn run_with_config(name: &str, config: &str, source: &[u8]) -> Output {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "forformat-wrap-policy-{name}-{}-{number}.toml",
        std::process::id()
    ));
    fs::write(&path, config).unwrap();

    let mut child = Command::new(binary())
        .arg("--config")
        .arg(&path)
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(source).unwrap();
    let output = child.wait_with_output().unwrap();
    let _ = fs::remove_file(path);
    output
}

#[test]
fn toml_rewrap_respects_both_wrap_disable_spellings() {
    let source = b"program p\ncall work(alpha, &\n    beta)\nend program p\n";
    let no_wrap = run_with_config("no-wrap", "rewrap = true\nno_wrap = true\n", source);
    let wrap_false = run_with_config("wrap-false", "rewrap = true\nwrap = false\n", source);

    assert!(
        no_wrap.status.success(),
        "{}",
        String::from_utf8_lossy(&no_wrap.stderr)
    );
    assert!(
        wrap_false.status.success(),
        "{}",
        String::from_utf8_lossy(&wrap_false.stderr)
    );
    assert_eq!(no_wrap.stdout, wrap_false.stdout);
    let output = String::from_utf8(no_wrap.stdout).unwrap();
    assert!(output.contains("work(alpha, &"), "{output}");
}
