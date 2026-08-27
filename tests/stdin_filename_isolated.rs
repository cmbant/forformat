use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_forformat"))
}

#[test]
fn isolated_named_stdin_expands_relative_include_declarations() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "forformat-isolated-stdin-include-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/defs.inc"), b"integer :: IncludedName\n").unwrap();

    let mut child = Command::new(binary())
        .args([
            "--full",
            "--no-config",
            "--isolated",
            "--stdin-filename",
            "src/new.f90",
        ])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"program p\ninclude 'defs.inc'\nprint *, includedname\nend program p\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("print *, IncludedName"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let _ = fs::remove_dir_all(root);
}
