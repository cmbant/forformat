use crate::{
    cli::{parse, Command, IndentQuery},
    error::FormatError,
};

fn error(args: &[&str]) -> String {
    let argv = std::iter::once("forformat")
        .chain(args.iter().copied())
        .map(str::to_owned);
    match parse(argv) {
        Err(FormatError::InvalidOption(message)) => message,
        Err(error) => panic!("expected an invalid option error for {args:?}, got {error}"),
        Ok(_) => panic!("expected an error for {args:?}"),
    }
}

#[test]
fn typed_actions_preserve_supported_combinations() {
    let Command::Run(invocation) = parse(
        [
            "forformat",
            "--no-config",
            "source.f90",
            "--check",
            "--diff",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap() else {
        panic!("expected run")
    };
    assert!(invocation.check);
    assert!(invocation.diff);

    let Command::Run(invocation) = parse(
        ["forformat", "--no-config", "--last-indent", "--last-usable"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap() else {
        panic!("expected run")
    };
    assert_eq!(invocation.indent_query, Some(IndentQuery::Both));
}

#[test]
fn stdin_filename_implies_stdin_and_combines_with_project_override() {
    let Command::Run(invocation) = parse(
        [
            "forformat",
            "--no-config",
            "--stdin-filename=src/new.f90",
            "--project-context=other",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap() else {
        panic!("expected run")
    };
    assert!(invocation.stdin);
    assert_eq!(
        invocation.stdin_filename.as_deref(),
        Some(std::path::Path::new("src/new.f90"))
    );
    assert_eq!(
        invocation.project_context.as_deref(),
        Some(std::path::Path::new("other"))
    );
}

#[test]
fn query_format_accepts_a_stdin_filename_without_project_context() {
    let Command::Run(invocation) = parse(
        [
            "forformat",
            "--no-config",
            "--query-format",
            "--stdin-filename=source.f90",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap() else {
        panic!("expected run")
    };
    assert!(invocation.query_format);
    assert!(invocation.stdin);
}

#[test]
fn stdin_filename_can_be_isolated_in_either_argument_order() {
    for args in [
        &["--no-config", "--stdin-filename=source.f90", "--isolated"][..],
        &["--no-config", "--isolated", "--stdin-filename=source.f90"][..],
    ] {
        let Command::Run(invocation) = parse(
            std::iter::once("forformat".to_string())
                .chain(args.iter().map(|arg| (*arg).to_string())),
        )
        .unwrap() else {
            panic!("expected run")
        };
        assert!(invocation.stdin);
        assert!(invocation.isolated);
        assert_eq!(
            invocation.stdin_filename.as_deref(),
            Some(std::path::Path::new("source.f90"))
        );
    }
}

#[test]
fn validation_priority_is_independent_of_argv_order() {
    let stdin = "--stdin cannot be combined with paths, --all, --all-files, --stdout, --check, --diff, --show-files, or --isolated";
    for args in [
        &["--no-config", "--stdin", "--last-indent", "--check"][..],
        &["--no-config", "--last-indent", "--check", "--stdin"][..],
    ] {
        assert_eq!(error(args), stdin, "{args:?}");
    }

    let filename = "--stdin-filename cannot be combined with paths, --all, --all-files, --stdout, --check, --diff, or --show-files";
    for args in [
        &["--no-config", "--stdin-filename=source.f90", "--check"][..],
        &["--no-config", "--check", "--stdin-filename=source.f90"][..],
    ] {
        assert_eq!(error(args), filename, "{args:?}");
    }

    let project = "--project-context cannot be combined with paths, --all, --all-files, --stdout, --isolated, --check, --diff, or --show-files";
    for args in [
        &["--no-config", "--project-context=.", "--check"][..],
        &["--no-config", "--check", "--project-context=."][..],
    ] {
        assert_eq!(error(args), project, "{args:?}");
    }

    for args in [
        &["--no-config", "--last-indent", "--check"][..],
        &["--no-config", "--check", "--last-indent"][..],
    ] {
        assert_eq!(
            error(args),
            "--check requires paths, --all, or --all-files",
            "{args:?}"
        );
    }
}
