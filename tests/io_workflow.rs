use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_forformat"))
}

fn temp_repo() -> PathBuf {
    let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "forformat-workflow-{}-{number}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&path)
        .status()
        .unwrap();
    path
}

fn git_add(path: &Path) {
    Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .status()
        .unwrap();
}

fn run(path: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .current_dir(path)
        .output()
        .unwrap()
}

fn run_stdin(path: &Path, args: &[&str], source: &[u8]) -> Output {
    let mut child = Command::new(binary())
        .args(args)
        .current_dir(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(source).unwrap();
    child.wait_with_output().unwrap()
}

fn run_with_bogus_git_env(path: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .current_dir(path)
        .env("GIT_DIR", "/definitely/not-a-repository")
        .env("GIT_WORK_TREE", "/definitely/not-a-work-tree")
        .env("GIT_COMMON_DIR", "/definitely/not-a-common-dir")
        .env("GIT_INDEX_FILE", "/definitely/not-an-index")
        .output()
        .unwrap()
}

#[test]
fn all_discovers_uppercase_extensions_and_ignores_hook_git_environment() {
    let repo = temp_repo();
    fs::write(repo.join("source.F90"), b"program p\nx=1\nend program p\n").unwrap();
    fs::write(repo.join("ignored.txt"), b"x\n").unwrap();
    git_add(&repo);
    let output = run_with_bogus_git_env(&repo, &["--indent-only", "--all", "--check"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("source.F90"));
    let output = run_with_bogus_git_env(&repo, &["--indent-only", "--all"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.F90")).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn section_9_1_checks_extension_before_existence_with_distinct_status2_errors() {
    let repo = temp_repo();
    let valid = run(&repo, &["missing.F90"]);
    assert_eq!(valid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&valid.stderr).contains("does not exist"));
    let invalid = run(&repo, &["missing.txt"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("expected a free-form Fortran source")
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn check_diff_and_query_mode_have_real_process_statuses() {
    let repo = temp_repo();
    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    git_add(&repo);
    let diff = run(&repo, &["--indent-only", "--diff", "source.f90"]);
    assert_eq!(diff.status.code(), Some(1));
    let text = String::from_utf8_lossy(&diff.stdout);
    assert!(text.contains("--- a/source.f90"));
    assert!(text.contains("+++ b/source.f90"));
    let check = run(&repo, &["--indent-only", "--check", "source.f90"]);
    assert_eq!(check.status.code(), Some(1));
    let stdout = run(&repo, &["--indent-only", "--stdout", "source.f90"]);
    assert_eq!(stdout.status.code(), Some(0));
    assert_eq!(stdout.stdout, b"program p\n   x=1\nend program p\n");
    let update = run(&repo, &["--indent-only", "source.f90"]);
    assert_eq!(update.status.code(), Some(0));
    let clean = run(&repo, &["--indent-only", "--check", "source.f90"]);
    assert_eq!(clean.status.code(), Some(0));
    let query = run(&repo, &["-lastindent", "--check", "source.f90"]);
    assert_eq!(query.status.code(), Some(2));
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn stdin_and_file_routes_produce_identical_bytes_for_the_same_source() {
    let repo = temp_repo();
    let source = b"program p\n!$ nthread = OMP_GET_MAX_THREADS()\nend program p\n";
    fs::write(repo.join("source.f90"), source).unwrap();
    let args = [
        "--full",
        "--indent=4",
        "--indent_module=0",
        "--indent_procedure=0",
        "--start_indent=4",
        "--indent_contains=0",
        "--openmp=0",
        "--indent_contains=restart",
        "--indent_select=4",
        "--indent_case=4",
        "--indent_interface=0",
        "--indent_continuation=4",
        "--indent_ampersand",
    ];
    let stdin = run_stdin(&repo, &args, source);
    let file = run(
        &repo,
        &[
            "--full",
            "--stdout",
            "--isolated",
            "--indent=4",
            "--indent_module=0",
            "--indent_procedure=0",
            "--start_indent=4",
            "--indent_contains=0",
            "--openmp=0",
            "--indent_contains=restart",
            "--indent_select=4",
            "--indent_case=4",
            "--indent_interface=0",
            "--indent_continuation=4",
            "--indent_ampersand",
            "source.f90",
        ],
    );
    assert_eq!(stdin.status.code(), Some(0));
    assert_eq!(file.status.code(), Some(0));
    assert_eq!(stdin.stdout, file.stdout);
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn stdin_applies_command_line_defines_in_full_mode() {
    let repo = temp_repo();
    let source = b"program p\nimplicit none\ninteger :: x\nx=size\nprint *, size\nend program p\n";
    let output = run_stdin(&repo, &["--full", "-D", "SIZE"], source);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        b"program p\n   implicit none\n   integer :: x\n   x = SIZE\n   print *, SIZE\n\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn isolated_keeps_local_component_resolution_like_stdin() {
    let repo = temp_repo();
    let source = b"module m
type :: Parent
real :: INTEGRATE_TOL
procedure :: ParentRun
end type Parent
contains
subroutine work(this)
class(Parent) :: THIS
this%integrate_tol = 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10 + 11 + 12 + 13 + 14 + 15 + 16 + 17 + 18 + 19 + 20 + 21 + 22 + 23 + 24 + 25 + 26 + 27 + 28 + 29 + 30
call this%parentrun()
end subroutine work
end module m
";
    fs::write(repo.join("source.f90"), source).unwrap();
    let args = ["--full", "--indent=4", "--start-indent=4"];
    let stdin = run_stdin(&repo, &args, source);
    let file = run(
        &repo,
        &[
            "--full",
            "--stdout",
            "--isolated",
            "--indent=4",
            "--start-indent=4",
            "source.f90",
        ],
    );
    assert_eq!(stdin.status.code(), Some(0));
    assert_eq!(file.status.code(), Some(0));
    assert_eq!(stdin.stdout, file.stdout);
    assert!(String::from_utf8_lossy(&file.stdout).contains("THIS%INTEGRATE_TOL"));
    assert!(String::from_utf8_lossy(&file.stdout).contains("call THIS%ParentRun"));
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn a_local_declaration_outranks_conflicting_project_spelling() {
    let repo = temp_repo();
    fs::write(
        repo.join("project.f90"),
        b"module other\ninteger :: sigma\nend module other\n",
    )
    .unwrap();
    fs::write(
        repo.join("target.f90"),
        b"module local\nreal(dl) Sigma\nend module local\n",
    )
    .unwrap();
    git_add(&repo);
    let output = run(&repo, &["--full", "--stdout", "target.f90"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("real(dl) Sigma"));
    let _ = fs::remove_dir_all(repo);
}

#[cfg(unix)]
#[test]
fn in_place_update_preserves_symlink_and_target_mode() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let repo = temp_repo();
    let target = repo.join("target.f90");
    let link = repo.join("link.f90");
    fs::write(&target, b"program p\nx=1\nend program p\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    symlink("target.f90", &link).unwrap();
    let output = run(&repo, &["--indent-only", "link.f90"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("target.f90"));
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::read(&target).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}
