use forformat::io::{repository_root, tracked_sources, validate_extension};
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

fn git_commit(path: &Path) {
    Command::new("git")
        .args([
            "-c",
            "user.name=forformat-test",
            "-c",
            "user.email=forformat-test@example.invalid",
            "commit",
            "-qm",
            "initial",
        ])
        .current_dir(path)
        .status()
        .unwrap();
}

/// A listing's separator is the platform's, but which files were listed is not.
/// Compare listings through this so the assertion pins the selection rather
/// than the POSIX spelling of it.
fn listing(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace('\\', "/")
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
fn all_files_excludes_submodules_from_targets_but_keeps_them_as_context() {
    let submodule = temp_repo();
    let submodule_source = submodule.join("submodule.f90");
    let submodule_bytes = b"module SharedName\nend module SharedName\n";
    fs::write(&submodule_source, submodule_bytes).unwrap();
    git_add(&submodule);
    git_commit(&submodule);

    let repo = temp_repo();
    let root_source = repo.join("root.f90");
    let root_bytes = b"program p\nuse sharedname\nend program p\n";
    fs::write(&root_source, root_bytes).unwrap();
    git_add(&repo);
    git_commit(&repo);
    Command::new("git")
        .args(["-c", "protocol.file.allow=always", "submodule", "add", "-q"])
        .arg(&submodule)
        .arg("vendor")
        .current_dir(&repo)
        .status()
        .unwrap();
    git_commit(&repo);
    let submodule_worktree_bytes = fs::read(repo.join("vendor/submodule.f90")).unwrap();

    let output = run(&repo, &["--full", "--all-files", "--context-path", "."]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        fs::read(&root_source).unwrap(),
        b"program p\n   use SharedName\nend program p\n"
    );
    assert_eq!(
        fs::read(repo.join("vendor/submodule.f90")).unwrap(),
        submodule_worktree_bytes
    );

    fs::write(&root_source, root_bytes).unwrap();
    fs::write(repo.join(".forformat.toml"), b"no_submodules = true\n").unwrap();
    let output = run(&repo, &["--full", "--all-files", "--context-path", "."]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        fs::read(&root_source).unwrap(),
        b"program p\n   use sharedname\nend program p\n"
    );

    fs::write(&root_source, root_bytes).unwrap();
    let output = run(
        &repo,
        &[
            "--full",
            "--all-files",
            "--context-path",
            ".",
            "--no-submodules=false",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        fs::read(&root_source).unwrap(),
        b"program p\n   use SharedName\nend program p\n"
    );

    let listed = run(&repo, &["--indent-only", "--all-files", "--show-files"]);
    assert_eq!(listed.status.code(), Some(0));
    assert_eq!(listing(&listed.stdout), "root.f90\n");

    let _ = fs::remove_dir_all(repo);
    let _ = fs::remove_dir_all(submodule);
}

#[test]
fn show_files_accepts_an_optional_directory_and_does_not_modify_sources() {
    let repo = temp_repo();
    fs::create_dir(repo.join("src")).unwrap();
    let source = b"program p\nx=1\nend program p\n";
    fs::write(repo.join("src/main.f90"), source).unwrap();
    fs::write(repo.join("other.f90"), source).unwrap();
    git_add(&repo);

    let output = run(
        &repo,
        &["--all-files", "src", "--show-files", "--exclude=main.f90"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(repo.join("src/main.f90")).unwrap(), source);
    assert_eq!(fs::read(repo.join("other.f90")).unwrap(), source);

    let output = run(&repo, &["--all-files", "src", "--show-files"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(listing(&output.stdout), "src/main.f90\n");
    assert_eq!(fs::read(repo.join("src/main.f90")).unwrap(), source);
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn a_lone_directory_argument_behaves_like_all_files() {
    let repo = temp_repo();
    fs::create_dir(repo.join("src")).unwrap();
    let source = b"program p\nx=1\nend program p\n";
    fs::write(repo.join("src/main.f90"), source).unwrap();
    fs::write(repo.join("other.f90"), source).unwrap();
    git_add(&repo);

    let listed = run(&repo, &["--indent-only", "src", "--show-files"]);
    assert_eq!(listed.status.code(), Some(0));
    assert_eq!(listing(&listed.stdout), "src/main.f90\n");

    let formatted = run(&repo, &["--indent-only", "src"]);
    assert_eq!(formatted.status.code(), Some(0));
    assert_eq!(listing(&formatted.stdout), "src/main.f90\n");
    assert_eq!(
        fs::read(repo.join("src/main.f90")).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );
    assert_eq!(fs::read(repo.join("other.f90")).unwrap(), *source);

    // Explicit `--all`/`--all-files` and multiple paths keep their existing
    // single-directory-only semantics; a directory is not promoted there.
    let two_paths = run(&repo, &["--indent-only", "src", "other.f90"]);
    assert_eq!(two_paths.status.code(), Some(2));

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn an_excluded_explicit_path_is_still_formatted() {
    let repo = temp_repo();
    fs::create_dir(repo.join("vendor")).unwrap();
    let path = repo.join("vendor/source.f90");
    fs::write(&path, b"program p\nx=1\nend program p\n").unwrap();
    git_add(&repo);

    let output = run(
        &repo,
        &["--indent-only", "--exclude", "vendor/", "vendor/source.f90"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"vendor/source.f90\n");
    assert_eq!(
        fs::read(&path).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn an_excluded_tracked_source_is_not_an_all_target() {
    let repo = temp_repo();
    fs::create_dir(repo.join("generated")).unwrap();
    let generated = repo.join("generated/source.f90");
    let kept = repo.join("kept.f90");
    let source = b"program p\nx=1\nend program p\n";
    fs::write(&generated, source).unwrap();
    fs::write(&kept, source).unwrap();
    git_add(&repo);

    let output = run(&repo, &["--indent-only", "--all", "--exclude=generated/"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("kept.f90"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("generated/source.f90"));
    assert_eq!(fs::read(&generated).unwrap(), source);
    assert_eq!(
        fs::read(&kept).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn excluding_a_project_source_removes_its_name_resolution() {
    let repo = temp_repo();
    let declarations = repo.join("declarations.f90");
    let dependent = repo.join("dependent.f90");
    fs::write(&declarations, b"module SharedName\nend module SharedName\n").unwrap();
    fs::write(
        &dependent,
        b"program p\nuse sharedname\nprint *, 1\nend program p\n",
    )
    .unwrap();
    git_add(&repo);

    let output = run(&repo, &["--full", "--all", "--exclude=/declarations.f90"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        fs::read(&dependent).unwrap(),
        b"program p\n   use sharedname\n   print *, 1\nend program p\n"
    );
    assert_eq!(
        fs::read(&declarations).unwrap(),
        b"module SharedName\nend module SharedName\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn exclude_arrays_work_in_standalone_and_pyproject_configuration() {
    let repo = temp_repo();
    fs::create_dir(repo.join("vendor")).unwrap();
    let kept = repo.join("kept.f90");
    let vendor = repo.join("vendor/source.f90");
    let generated = repo.join("generated.f90");
    let source = b"program p\nx=1\nend program p\n";
    fs::write(&kept, source).unwrap();
    fs::write(&vendor, source).unwrap();
    fs::write(&generated, source).unwrap();
    fs::write(
        repo.join(".forformat.toml"),
        b"exclude = [\"vendor/\"]\nextend-exclude = [\"generated.f90\"]\n",
    )
    .unwrap();
    git_add(&repo);

    let output = run(&repo, &["--indent-only", "--all"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(fs::read(&vendor).unwrap(), source);
    assert_eq!(fs::read(&generated).unwrap(), source);
    assert_eq!(
        fs::read(&kept).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );

    for path in [&kept, &vendor, &generated] {
        fs::write(path, source).unwrap();
    }
    fs::remove_file(repo.join(".forformat.toml")).unwrap();
    fs::write(
        repo.join("pyproject.toml"),
        b"[tool.forformat]\nexclude = [\"vendor/\"]\nextend_exclude = [\"generated.f90\"]\n",
    )
    .unwrap();
    let output = run(&repo, &["--indent-only", "--all"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(fs::read(&vendor).unwrap(), source);
    assert_eq!(fs::read(&generated).unwrap(), source);
    assert_eq!(
        fs::read(&kept).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

/// `--exclude` selects a set rather than adding to one, so it replaces the
/// configured `exclude` instead of accumulating with it — the way ruff and
/// black treat the same pair of options. `extend-exclude` is the additive
/// spelling and survives from the configuration file.
#[test]
fn a_command_line_exclude_replaces_the_configured_one() {
    let repo = temp_repo();
    fs::create_dir(repo.join("vendor")).unwrap();
    let kept = repo.join("kept.f90");
    let vendor = repo.join("vendor/source.f90");
    let generated = repo.join("generated.f90");
    let source = b"program p\nx=1\nend program p\n";
    let formatted = b"program p\n   x=1\nend program p\n";
    for path in [&kept, &vendor, &generated] {
        fs::write(path, source).unwrap();
    }
    fs::write(
        repo.join(".forformat.toml"),
        b"exclude = [\"vendor/\"]\nextend-exclude = [\"generated.f90\"]\n",
    )
    .unwrap();
    git_add(&repo);

    let output = run(&repo, &["--indent-only", "--all", "--exclude=kept.f90"]);
    assert_eq!(output.status.code(), Some(0));
    // The configured `exclude = ["vendor/"]` is gone, so vendor is formatted.
    assert_eq!(fs::read(&vendor).unwrap(), formatted);
    // `extend-exclude` is additive and still applies.
    assert_eq!(fs::read(&generated).unwrap(), source);
    // The command-line pattern is the one in force.
    assert_eq!(fs::read(&kept).unwrap(), source);

    for path in [&kept, &vendor, &generated] {
        fs::write(path, source).unwrap();
    }
    // `--extend-exclude` adds instead, so every configured pattern survives.
    let output = run(
        &repo,
        &["--indent-only", "--all", "--extend-exclude=kept.f90"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(fs::read(&vendor).unwrap(), source);
    assert_eq!(fs::read(&generated).unwrap(), source);
    assert_eq!(fs::read(&kept).unwrap(), source);
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn project_config_applies_to_all_and_cli_options_override_it() {
    let repo = temp_repo();
    fs::write(
        repo.join("pyproject.toml"),
        b"[tool.forformat]\nmode = \"indent-only\"\nindent = 4\n",
    )
    .unwrap();
    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    git_add(&repo);

    let configured = run(&repo, &["--all"]);
    assert_eq!(configured.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n    x=1\nend program p\n"
    );

    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    let overridden = run(&repo, &["--all", "--indent=2"]);
    assert_eq!(overridden.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n  x=1\nend program p\n"
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn standalone_project_config_is_used_and_can_be_disabled() {
    let repo = temp_repo();
    fs::write(repo.join(".forformat.toml"), b"indent = 5\n").unwrap();
    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    git_add(&repo);

    let configured = run(&repo, &["--indent-only", "--all"]);
    assert_eq!(configured.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n     x=1\nend program p\n"
    );

    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    let defaults = run(&repo, &["--no-config", "--indent-only", "--all"]);
    assert_eq!(defaults.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn config_selector_spellings_and_explicit_pyproject_are_honored() {
    let repo = temp_repo();
    fs::write(
        repo.join("pyproject.toml"),
        b"[tool.forformat]\nindent = 7\n",
    )
    .unwrap();
    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    git_add(&repo);

    let disabled = run(&repo, &["--indent-only", "--all", "--no_config"]);
    assert_eq!(disabled.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n   x=1\nend program p\n"
    );

    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    let explicit = run(
        &repo,
        &["--indent-only", "--all", "--config", "pyproject.toml"],
    );
    assert_eq!(explicit.status.code(), Some(0));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n       x=1\nend program p\n"
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn help_and_version_ignore_malformed_project_config() {
    let repo = temp_repo();
    fs::write(repo.join(".forformat.toml"), b"indent = \"").unwrap();

    let help = run(&repo, &["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("Free-form Fortran formatter."));

    let version = run(&repo, &["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("forformat "));

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn all_directory_scope_uses_only_the_nested_repo_and_its_config() {
    let parent = temp_repo();
    fs::write(
        parent.join("outer.f90"),
        b"program outer\nx=1\nend program outer\n",
    )
    .unwrap();
    git_add(&parent);
    let repo = parent.join("nested");
    fs::create_dir(&repo).unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap();
    fs::write(repo.join(".forformat.toml"), b"indent = 6\n").unwrap();
    fs::write(repo.join("source.f90"), b"program p\nx=1\nend program p\n").unwrap();
    git_add(&repo);

    let context_root = repository_root(&repo).unwrap().unwrap();
    assert_eq!(context_root, fs::canonicalize(&repo).unwrap());
    assert_eq!(
        tracked_sources(&context_root).unwrap(),
        vec![context_root.join("source.f90")]
    );

    let output = run(&parent, &["--indent-only", "--all", "./nested"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("source.f90"));
    assert_eq!(
        fs::read(repo.join("source.f90")).unwrap(),
        b"program p\n      x=1\nend program p\n"
    );

    let _ = fs::remove_dir_all(parent);
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
fn fixed_form_targets_are_skipped_without_affecting_status_or_stdout() {
    let repo = temp_repo();
    let source = b"* legacy fixed-form comment\n      x = 1\n";
    let path = repo.join("legacy.f");
    fs::write(&path, source).unwrap();
    git_add(&repo);

    let stdout = run(&repo, &["--full", "--no-config", "--stdout", "legacy.f"]);
    assert_eq!(stdout.status.code(), Some(0));
    assert_eq!(stdout.stdout, source);
    assert_eq!(
        stdout.stderr,
        b"forformat: legacy.f: fixed-form source, skipped\n"
    );

    let check = run(&repo, &["--full", "--no-config", "--check", "legacy.f"]);
    assert_eq!(check.status.code(), Some(0));
    assert_eq!(fs::read(&path).unwrap(), source);
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn ifree_forces_formatting_of_a_detected_fixed_form_target() {
    let repo = temp_repo();
    let source = b"* legacy fixed-form comment\n      x = 1\n";
    let path = repo.join("legacy.f");
    fs::write(&path, source).unwrap();
    git_add(&repo);

    let output = run(
        &repo,
        &["--full", "--no-config", "-ifree", "--stdout", "legacy.f"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_ne!(output.stdout, source);
    assert!(output.stderr.is_empty());
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn query_format_reports_each_input_without_writing() {
    let repo = temp_repo();
    fs::write(repo.join("legacy.f"), b"* comment\n").unwrap();
    fs::write(repo.join("modern.F90"), b"MODULE m\nEND MODULE m\n").unwrap();
    git_add(&repo);

    let output = run(&repo, &["--query-format", "legacy.f", "modern.F90"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"fixed\nfree\n");
    assert!(output.stderr.is_empty());
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn query_format_bulk_selection_applies_exclusions() {
    let repo = temp_repo();
    fs::create_dir(repo.join("vendor")).unwrap();
    fs::write(repo.join("main.f90"), b"program p\nend program p\n").unwrap();
    fs::write(
        repo.join("vendor/third_party.f90"),
        b"program vendor\nend program vendor\n",
    )
    .unwrap();
    git_add(&repo);

    let output = run(&repo, &["--query-format", "--all", "--exclude=vendor/"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"free\n");
    assert!(output.stderr.is_empty());
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn query_format_rejects_project_context_only_options() {
    let repo = temp_repo();
    fs::write(repo.join("main.f90"), b"program p\nend program p\n").unwrap();
    git_add(&repo);

    for option in ["--context-path", "--isolated"] {
        let args = if option == "--context-path" {
            vec!["--query-format", option, "."]
        } else {
            vec!["--query-format", option, "main.f90"]
        };
        let output = run(&repo, &args);
        assert_eq!(output.status.code(), Some(2), "{option}");
    }
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn isolated_and_query_format_ignore_configured_context_paths() {
    let repo = temp_repo();
    fs::write(
        repo.join(".forformat.toml"),
        b"context_paths = [\"does-not-exist\"]\n",
    )
    .unwrap();
    fs::write(repo.join("main.f90"), b"program p\nend program p\n").unwrap();
    git_add(&repo);

    let isolated = run(&repo, &["--isolated", "--stdout", "main.f90"]);
    assert_eq!(isolated.status.code(), Some(0));
    assert_eq!(isolated.stdout, b"program p\nend program p\n");
    assert!(isolated.stderr.is_empty());

    let query = run(&repo, &["--query-format", "main.f90"]);
    assert_eq!(query.status.code(), Some(0));
    assert_eq!(query.stdout, b"free\n");
    assert!(query.stderr.is_empty());

    let listed = run(&repo, &["--all-files", "--show-files"]);
    assert_eq!(listed.status.code(), Some(0));
    assert_eq!(listing(&listed.stdout), "main.f90\n");
    assert!(listed.stderr.is_empty());

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn stdin_and_file_routes_produce_identical_bytes_for_the_same_source() {
    let repo = temp_repo();
    let mut fixtures: Vec<_> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            // The tool's own predicate, so the route comparison covers every
            // fixture it would accept — `.F90` and `.F` included.
            .filter(|path| validate_extension(path).is_ok())
            .collect();
    fixtures.sort();

    for fixture in fixtures {
        let source = fs::read(&fixture).unwrap();
        fs::write(repo.join("source.f90"), &source).unwrap();
        git_add(&repo);

        let stdin = run_stdin(&repo, &["--full", "--no-config"], &source);
        let isolated = run(
            &repo,
            &[
                "--full",
                "--no-config",
                "--stdout",
                "--isolated",
                "source.f90",
            ],
        );
        let project = run(&repo, &["--full", "--no-config", "--stdout", "source.f90"]);
        assert_eq!(stdin.status.code(), Some(0), "stdin: {}", fixture.display());
        assert_eq!(
            isolated.status.code(),
            Some(0),
            "isolated: {}\n{}",
            fixture.display(),
            String::from_utf8_lossy(&isolated.stderr)
        );
        assert_eq!(
            project.status.code(),
            Some(0),
            "project: {}\n{}",
            fixture.display(),
            String::from_utf8_lossy(&project.stderr)
        );
        assert_eq!(
            stdin.stdout,
            isolated.stdout,
            "stdin vs isolated: {}",
            fixture.display()
        );
        assert_eq!(
            stdin.stdout,
            project.stdout,
            "stdin vs project: {}",
            fixture.display()
        );
    }
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
        b"program p\n   implicit none\n   integer :: x\n   x = SIZE\n   print *, SIZE\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn a_macro_name_broken_across_a_continuation_is_cased_on_the_first_pass() {
    // Reduced from Q-E `Modules/lmdif.f90`.  The author split `zero` as
    // `zer&` / `&o`, which the macro pass reads one physical line at a time and
    // so never recognizes.  Wrapping then rejoined the halves, and the *next*
    // run cased the name it could finally see -- an I1 failure that only shows
    // up once a macro of that name is in scope.
    let repo = temp_repo();
    let source = b"subroutine s(n, aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbb, cccccccccccccccc)\n\
integer n\n\
real aaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbb, cccccccccccccccc\n\
if (n.le.0.or.aaaaaaaaaaaaaaaa.lt.ZERO.or.bbbbbbbbbbbbbbbb.lt.zer&\n\
 &o.or.cccccccccccccccc.lt.ZERO) goto 300\n\
300 continue\n\
end subroutine s\n";
    let once = run_stdin(&repo, &["--full", "-D", "ZERO"], source);
    assert_eq!(once.status.code(), Some(0));
    let twice = run_stdin(&repo, &["--full", "-D", "ZERO"], &once.stdout);
    assert_eq!(
        String::from_utf8_lossy(&once.stdout)
            .matches("ZERO")
            .count(),
        3,
        "the split macro name was not cased on the first pass:\n{}",
        String::from_utf8_lossy(&once.stdout)
    );
    assert_eq!(
        once.stdout,
        twice.stdout,
        "first pass:\n{}\nsecond pass:\n{}",
        String::from_utf8_lossy(&once.stdout),
        String::from_utf8_lossy(&twice.stdout)
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn project_context_supplies_declarations_without_discovering_config() {
    let repo = temp_repo();
    fs::write(repo.join(".forformat.toml"), b"indent = 8\n").unwrap();
    fs::write(
        repo.join("project.f90"),
        b"module SharedName\nend module SharedName\n",
    )
    .unwrap();
    git_add(&repo);

    let source = b"program p\nuse sharedname\nprint *, 1\nend program p\n";
    let output = run_stdin(
        &repo,
        &["--stdin", "--full", "--no-config", "--project-context", "."],
        source,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        b"program p\n   use SharedName\n   print *, 1\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn context_paths_limit_project_context_and_form_a_union() {
    let repo = temp_repo();
    fs::create_dir(repo.join("src")).unwrap();
    fs::create_dir(repo.join("modules")).unwrap();
    fs::write(
        repo.join("src/first.f90"),
        b"module FirstName\nend module FirstName\n",
    )
    .unwrap();
    fs::write(
        repo.join("modules/second.f90"),
        b"module SecondName\nend module SecondName\n",
    )
    .unwrap();
    let target = repo.join("target.f90");
    fs::write(
        &target,
        b"program p\nuse firstname\nuse secondname\nend program p\n",
    )
    .unwrap();
    git_add(&repo);

    let output = run(
        &repo,
        &["--full", "--stdout", "target.f90", "--context-path", "src"],
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use FirstName"));
    assert!(output.contains("use secondname"));

    let output = run(
        &repo,
        &[
            "--full",
            "--stdout",
            "target.f90",
            "--context-path",
            "src",
            "--context-path",
            "modules",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use FirstName"));
    assert!(output.contains("use SecondName"));
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn context_paths_discover_recursive_filesystem_sources_for_anonymous_stdin() {
    let directory = std::env::temp_dir().join(format!(
        "forformat-context-filesystem-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("src/nested")).unwrap();
    fs::write(
        directory.join("src/nested/first.f90"),
        b"module FirstName\nend module FirstName\n",
    )
    .unwrap();
    fs::write(
        directory.join("src/generated.f90"),
        b"module GeneratedName\nend module GeneratedName\n",
    )
    .unwrap();
    fs::write(directory.join("src/notes.txt"), b"module NotFortran\n").unwrap();

    let output = run_stdin(
        &directory,
        &[
            "--stdin",
            "--full",
            "--no-config",
            "--context-path",
            "src",
            "--exclude",
            "generated.f90",
        ],
        b"program p\nuse firstname\nuse generatedname\nend program p\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use FirstName"));
    assert!(output.contains("use generatedname"));

    fs::write(
        directory.join("target.f90"),
        b"program p\nuse firstname\nend program p\n",
    )
    .unwrap();
    let output = run(
        &directory,
        &[
            "--full",
            "--no-config",
            "--stdout",
            "target.f90",
            "--context-path",
            "src",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("use FirstName"));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn configured_context_paths_resolve_from_config_directory_outside_git() {
    let project = std::env::temp_dir().join(format!(
        "forformat-config-context-project-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let cwd = std::env::temp_dir().join(format!(
        "forformat-config-context-cwd-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    fs::write(
        project.join(".forformat.toml"),
        b"context_paths = [\"src\"]\n",
    )
    .unwrap();
    fs::write(
        project.join("src/shared.f90"),
        b"module SharedName\nend module SharedName\n",
    )
    .unwrap();
    let target = project.join("main.f90");
    fs::write(&target, b"program p\nuse sharedname\nend program p\n").unwrap();

    let output = run(&cwd, &["--full", "--stdout", target.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("use SharedName"));

    let _ = fs::remove_dir_all(project);
    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn filesystem_context_exclusions_are_relative_to_each_context_root() {
    let directory = std::env::temp_dir().join(format!(
        "forformat-context-exclusions-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("a/vendor")).unwrap();
    fs::create_dir_all(directory.join("a/nested/vendor")).unwrap();
    fs::create_dir_all(directory.join("b/vendor")).unwrap();
    fs::write(
        directory.join("a/vendor/a_root.f90"),
        b"module ARootName\nend module ARootName\n",
    )
    .unwrap();
    fs::write(
        directory.join("a/nested/vendor/a_nested.f90"),
        b"module ANestedName\nend module ANestedName\n",
    )
    .unwrap();
    fs::write(
        directory.join("b/vendor/b_root.f90"),
        b"module BRootName\nend module BRootName\n",
    )
    .unwrap();
    fs::write(
        directory.join("a/visible.f90"),
        b"module VisibleName\nend module VisibleName\n",
    )
    .unwrap();

    let source = b"program p\nuse arootname\nuse anestedname\nuse brootname\nuse visiblename\nend program p\n";
    let output = run_stdin(
        &directory,
        &[
            "--stdin",
            "--full",
            "--no-config",
            "--context-path",
            "a",
            "--context-path",
            "b",
            "--exclude=vendor/",
        ],
        source,
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use arootname"));
    assert!(output.contains("use anestedname"));
    assert!(output.contains("use brootname"));
    assert!(output.contains("use VisibleName"));

    let output = run_stdin(
        &directory,
        &[
            "--stdin",
            "--full",
            "--no-config",
            "--context-path",
            "a",
            "--context-path",
            "b",
            "--exclude=/vendor/",
        ],
        source,
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use arootname"));
    assert!(output.contains("use ANestedName"));
    assert!(output.contains("use brootname"));
    assert!(output.contains("use VisibleName"));

    let _ = fs::remove_dir_all(directory);
}

#[test]
fn overlapping_filesystem_context_roots_cannot_bypass_exclusions() {
    let directory = std::env::temp_dir().join(format!(
        "forformat-context-overlap-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("vendor")).unwrap();
    fs::write(
        directory.join("visible.f90"),
        b"module VisibleName
end module VisibleName
",
    )
    .unwrap();
    fs::write(
        directory.join("vendor/hidden.f90"),
        b"module HiddenName
end module HiddenName
",
    )
    .unwrap();

    let output = run_stdin(
        &directory,
        &[
            "--stdin",
            "--full",
            "--no-config",
            "--context-path",
            directory.to_str().unwrap(),
            "--context-path",
            directory.join("vendor").to_str().unwrap(),
            "--exclude=vendor/",
        ],
        b"program p
use visiblename
use hiddenname
end program p
",
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use VisibleName"));
    assert!(output.contains("use hiddenname"));

    let _ = fs::remove_dir_all(directory);
}

#[cfg(unix)]
#[test]
fn filesystem_context_does_not_follow_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = std::env::temp_dir().join(format!(
        "forformat-context-symlink-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let outside = std::env::temp_dir().join(format!(
        "forformat-context-symlink-outside-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(
        directory.join("src/visible.f90"),
        b"module VisibleName\nend module VisibleName\n",
    )
    .unwrap();
    fs::write(
        outside.join("hidden.f90"),
        b"module HiddenName\nend module HiddenName\n",
    )
    .unwrap();
    symlink(&outside, directory.join("src/outside")).unwrap();

    let output = run_stdin(
        &directory,
        &["--stdin", "--full", "--no-config", "--context-path", "src"],
        b"program p\nuse visiblename\nuse hiddenname\nend program p\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use VisibleName"));
    assert!(output.contains("use hiddenname"));

    let _ = fs::remove_dir_all(&directory);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn cli_context_paths_replace_configured_paths_and_exclusions_apply() {
    let repo = temp_repo();
    fs::create_dir(repo.join("src")).unwrap();
    fs::create_dir(repo.join("modules")).unwrap();
    fs::write(
        repo.join("src/first.f90"),
        b"module FirstName\nend module FirstName\n",
    )
    .unwrap();
    fs::write(
        repo.join("modules/second.f90"),
        b"module SecondName\nend module SecondName\n",
    )
    .unwrap();
    fs::write(
        repo.join("src/generated.f90"),
        b"module GeneratedName\nend module GeneratedName\n",
    )
    .unwrap();
    let target = repo.join("target.f90");
    fs::write(
        &target,
        b"program p\nuse firstname\nuse secondname\nuse generatedname\nend program p\n",
    )
    .unwrap();
    fs::write(
        repo.join(".forformat.toml"),
        b"context_paths = [\"modules\"]\n",
    )
    .unwrap();
    git_add(&repo);

    let output = run(
        &repo,
        &[
            "--full",
            "--stdout",
            "target.f90",
            "--context-path",
            "src",
            "--exclude=src/generated.f90",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("use FirstName"));
    assert!(output.contains("use secondname"));
    assert!(output.contains("use generatedname"));
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn context_paths_validate_directories_inside_the_checkout() {
    let repo = temp_repo();
    let target = repo.join("target.f90");
    fs::write(&target, b"program p\nend program p\n").unwrap();
    git_add(&repo);
    let outside = std::env::temp_dir().join(format!(
        "forformat-context-outside-{}",
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&outside).unwrap();

    for path in ["missing", "target.f90", outside.to_str().unwrap()] {
        let output = run(
            &repo,
            &["--full", "--stdout", "target.f90", "--context-path", path],
        );
        assert_eq!(output.status.code(), Some(2), "{path}");
    }
    let _ = fs::remove_dir_all(&outside);
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn file_project_context_replacement_respects_context_scope() {
    let repo = temp_repo();
    fs::create_dir(repo.join("src")).unwrap();
    let target = repo.join("src/target.f90");
    fs::write(&target, b"module StaleName\nend module StaleName\n").unwrap();
    git_add(&repo);

    let output = run_stdin(
        &repo,
        &[
            "--full",
            "--no-config",
            "--project-context",
            "src/target.f90",
            "--context-path",
            "src",
        ],
        b"module CurrentName\nend module CurrentName\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        b"module CurrentName\nend module CurrentName\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn project_context_implies_stdin_and_anchors_config_discovery() {
    let repo = temp_repo();
    fs::create_dir(repo.join("src")).unwrap();
    fs::write(repo.join(".forformat.toml"), b"indent = 4\n").unwrap();
    fs::write(repo.join("src/.forformat.toml"), b"indent = 8\n").unwrap();

    let output = run_stdin(
        &repo,
        &["--indent-only", "--project-context", "src"],
        b"program p\nx=1\nend program p\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"program p\n        x=1\nend program p\n");
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn file_project_context_matches_normal_file_config_discovery() {
    let repo = temp_repo();
    let source = b"program p\nx=1\nend program p\n";
    fs::create_dir(repo.join("src")).unwrap();
    fs::write(repo.join(".forformat.toml"), b"indent = 4\n").unwrap();
    fs::write(repo.join("src/.forformat.toml"), b"indent = 8\n").unwrap();
    fs::write(repo.join("src/foo.f90"), source).unwrap();
    git_add(&repo);

    let file = run(&repo, &["--indent-only", "--stdout", "src/foo.f90"]);
    let project = run_stdin(
        &repo,
        &["--indent-only", "--project-context", "src/foo.f90"],
        source,
    );
    assert_eq!(file.status.code(), Some(0));
    assert_eq!(project.status.code(), Some(0));
    assert_eq!(project.stdout, file.stdout);
    assert_eq!(project.stdout, b"program p\n        x=1\nend program p\n");

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn file_project_context_uses_the_context_filename_for_detection() {
    let repo = temp_repo();
    let target = repo.join("target.f90");
    fs::write(&target, b"program p\nend program p\n").unwrap();
    git_add(&repo);

    let output = run_stdin(
        &repo,
        &["--full", "--no-config", "--project-context", "target.f90"],
        b"      program p\n      end program p\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("fixed-form source, skipped"));
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn file_project_context_excludes_the_stale_on_disk_target() {
    let repo = temp_repo();
    let target = repo.join("target.f90");
    fs::write(&target, b"program p\ninteger :: StaleName\nend program p\n").unwrap();
    fs::write(
        repo.join("shared.f90"),
        b"module SharedName\nend module SharedName\n",
    )
    .unwrap();
    git_add(&repo);

    let source = b"program p\nuse sharedname\nprint *, stalename\nend program p\n";
    let output = run_stdin(
        &repo,
        &[
            "--stdin",
            "--full",
            "--no-config",
            "--project-context",
            target.to_str().unwrap(),
        ],
        source,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        b"program p\n   use SharedName\n   print *, stalename\nend program p\n"
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn project_context_rejects_a_non_repository_directory() {
    let directory = std::env::temp_dir().join(format!(
        "forformat-non-repository-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir(&directory).unwrap();
    let output = run_stdin(
        &directory,
        &["--stdin", "--no-config", "--project-context", "."],
        b"program p\nend program p\n",
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--project-context requires a valid Git checkout"));
    let _ = fs::remove_dir_all(directory);
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

#[test]
fn a_glued_end_construct_split_over_an_unmarked_continuation_keeps_its_case() {
    // Reduced from Q-E `PW/src/exx_bp.f90` / `exx_std.f90` under
    // `style-lex-spacing-layout`, found only by the harness's whole-tree run
    // since it needs project context that per-file sweeps do not carry. The
    // continuation line naming the loop has no leading `&`, so joining it to
    // the split `ENDDO&` line without the blank the continuation rule
    // requires spliced `DO` and the construct name into one bogus token;
    // `END` then failed the end-construct check on the second pass and was
    // recast to the project's declared spelling of `end`.
    let repo = temp_repo();
    fs::write(
        repo.join("project.f90"),
        b"module m\nreal :: end\nend module m\n",
    )
    .unwrap();
    git_add(&repo);

    let args = [
        "--stdin",
        "--full",
        "--no-config",
        "--keyword-case=preserve",
        "--continuation-markers=0",
        "--project-context",
        ".",
    ];
    let source = b"SUBROUTINE p\n   IMPLICIT NONE\n   INTEGER :: i\n   loop : &\n   DO i = 1, 2\n   ENDDO&\n   loop\nEND SUBROUTINE p\n";
    let once = run_stdin(&repo, &args, source);
    assert_eq!(once.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&once.stdout).contains("END DO"),
        "expected END DO in {:?}",
        String::from_utf8_lossy(&once.stdout)
    );
    let twice = run_stdin(&repo, &args, &once.stdout);
    assert_eq!(once.stdout, twice.stdout);
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

#[test]
fn indentation_queries_emit_only_metadata_from_stdin() {
    let repo = temp_repo();
    let source = b"program p\nx=1\n";

    let indent = run_stdin(&repo, &["-lastindent"], source);
    assert_eq!(indent.status.code(), Some(0));
    assert_eq!(indent.stdout, b"3\n");

    let usable = run_stdin(&repo, &["--last-usable"], source);
    assert_eq!(usable.status.code(), Some(0));
    assert_eq!(usable.stdout, b"2\n");

    let both = run_stdin(&repo, &["-lastindent", "-lastusable"], source);
    assert_eq!(both.status.code(), Some(0));
    assert_eq!(both.stdout, b"2\n");

    let contextual = run_stdin(&repo, &["-lastindent", "--project-context", "."], source);
    assert_eq!(contextual.status.code(), Some(0));
    assert_eq!(contextual.stdout, b"3\n");

    let missing_context = run_stdin(
        &repo,
        &["-lastindent", "--project-context", "DOES_NOT_EXIST"],
        source,
    );
    assert_eq!(missing_context.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_context.stderr)
        .contains("--project-context path does not exist"));

    let empty_indent = run_stdin(&repo, &["-lastindent"], b"");
    assert_eq!(empty_indent.stdout, b"0\n");
    let empty_usable = run_stdin(&repo, &["-lastusable"], b"");
    assert_eq!(empty_usable.stdout, b"1\n");

    let _ = fs::remove_dir_all(repo);
}
