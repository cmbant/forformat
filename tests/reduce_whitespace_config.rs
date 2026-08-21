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
        "forformat-whitespace-option-{name}-{}-{number}.toml",
        std::process::id()
    ));
    fs::write(&path, config).unwrap();

    let mut child = Command::new(binary())
        .arg("--config")
        .arg(&path)
        .arg("--input-format=free")
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
fn native_and_legacy_whitespace_config_keys_match() {
    let source = b"program p\ncall   work\nend program p\n";
    let native = run_with_config("native", "reduce_whitespace = true\n", source);
    let legacy = run_with_config("legacy", "ws_remred = true\n", source);

    assert!(
        native.status.success(),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert!(
        legacy.status.success(),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    assert_eq!(native.stdout, legacy.stdout);
    assert!(
        String::from_utf8_lossy(&native.stdout).contains("call work"),
        "{}",
        String::from_utf8_lossy(&native.stdout)
    );
}
