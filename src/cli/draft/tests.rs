use crate::cli::{parse, Command};

fn error(args: &[&str]) -> String {
    let argv = std::iter::once("forformat")
        .chain(args.iter().copied())
        .map(str::to_owned);
    match parse(argv) {
        Err(error) => error.to_string(),
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
    assert!(invocation.config.last_indent);
    assert!(invocation.config.last_usable);
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
