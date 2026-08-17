use super::{parse, parse_inner, Command, DEFAULT_EXCLUDES};
use std::path::PathBuf;

fn run(args: &[&str]) -> crate::config::FormatConfig {
    let mut argv = vec!["forformat".to_string()];
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    match parse(argv).unwrap() {
        Command::Run(invocation) => invocation.config,
        _ => panic!("expected a formatting command"),
    }
}

fn selection(args: &[&str]) -> Result<(bool, Option<PathBuf>), crate::error::FormatError> {
    let mut argv = vec!["forformat".to_string()];
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    parse_inner(argv).and_then(|parsed| parsed.config_selection.resolve())
}

#[test]
fn config_selection_uses_the_parser_value_grammar() {
    let consumed = parse_inner([
        "forformat".to_string(),
        "-D".to_string(),
        "--config".to_string(),
        "foo.toml".to_string(),
    ])
    .unwrap();
    assert_eq!(consumed.config_selection.resolve().unwrap(), (false, None));
    match consumed.command {
        Command::Run(invocation) => {
            assert_eq!(invocation.config.defines[0].name, "--config");
            assert_eq!(invocation.paths, [PathBuf::from("foo.toml")]);
        }
        _ => panic!("expected run"),
    }

    assert_eq!(
        selection(&["-D", "VALUE", "--config", "foo.toml"]).unwrap(),
        (false, Some(PathBuf::from("foo.toml")))
    );
    assert_eq!(selection(&["--define=--config"]).unwrap(), (false, None));
    assert_eq!(selection(&["-D", "--no-config"]).unwrap(), (false, None));
}

#[test]
fn config_selection_preserves_spellings_conflicts_and_termination() {
    assert_eq!(
        selection(&["--config=path.toml"]).unwrap(),
        (false, Some(PathBuf::from("path.toml")))
    );
    assert_eq!(
        selection(&["--config", "path.toml"]).unwrap(),
        (false, Some(PathBuf::from("path.toml")))
    );
    assert_eq!(selection(&["--no-config"]).unwrap(), (true, None));

    assert!(matches!(
        selection(&["--config=one.toml", "--config=two.toml"]),
        Err(crate::error::FormatError::InvalidOption(message))
            if message == "--config may be specified only once"
    ));
    assert!(matches!(
        selection(&["--no-config", "--config=path.toml"]),
        Err(crate::error::FormatError::InvalidOption(message))
            if message == "--config cannot be combined with --no-config"
    ));

    let terminated = parse_inner([
        "forformat".to_string(),
        "--".to_string(),
        "--config".to_string(),
        "foo.toml".to_string(),
    ])
    .unwrap();
    assert_eq!(
        terminated.config_selection.resolve().unwrap(),
        (false, None)
    );
    match terminated.command {
        Command::Run(invocation) => {
            assert_eq!(
                invocation.paths,
                [PathBuf::from("--config"), PathBuf::from("foo.toml")]
            );
        }
        _ => panic!("expected run"),
    }
}

#[test]
fn overloaded_short_options_accept_attached_and_separated_values() {
    let attached = run(&["-i4", "-C-", "-k5", "-M9"]);
    let separated = run(&["-i", "4", "-C", "-", "-k", "5", "-M", "9"]);
    assert_eq!(attached, separated);
    assert!(!run(&["-i-"]).apply_indent);
    assert!(run(&["-Ia"]).auto_start_indent);
    assert!(parse(["forformat".to_string(), "-iauto".to_string()].into_iter()).is_ok());
}

#[test]
fn optional_values_are_not_taken_from_the_next_argument() {
    let bare = run(&["--align_paren"]);
    assert!(bare.align_paren);
    assert_eq!(bare.align_paren_value, 1);
    assert!(!run(&["--align-paren=0"]).align_paren);
    assert_eq!(run(&["--align_paren=4"]).align_paren_value, 4);
    assert!(run(&["--ws-remred"]).ws_remred);
    assert_eq!(run(&["--ws_remred=0"]).ws_remred_value, 0);

    let Command::Run(invocation) = parse(
        ["forformat", "--no-config", "--wrap", "source.f90"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap()
    else {
        panic!("expected run")
    };
    assert_eq!(invocation.paths, [PathBuf::from("source.f90")]);
}

#[test]
fn no_submodules_accepts_explicit_boolean_values() {
    let parse_no_submodules = |args: &[&str]| {
        let argv = std::iter::once("forformat")
            .chain(args.iter().copied())
            .map(str::to_owned);
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected a formatting command");
        };
        invocation.no_submodules
    };

    assert!(parse_no_submodules(&["--no-config", "--no-submodules"]));
    assert!(parse_no_submodules(&[
        "--no-config",
        "--no-submodules=true"
    ]));
    assert!(!parse_no_submodules(&[
        "--no-config",
        "--no-submodules=false"
    ]));
}

#[test]
fn optional_boolean_switches_accept_bare_and_explicit_values() {
    let parse_config = |args: &[&str]| {
        let argv = std::iter::once("forformat")
            .chain(args.iter().copied())
            .map(str::to_owned);
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected a formatting command");
        };
        invocation.config
    };

    assert!(parse_config(&["--no-config", "--wrap"]).wrap.enabled);
    assert!(!parse_config(&["--no-config", "--wrap=false"]).wrap.enabled);
    assert!(!parse_config(&["--no-config", "--no-wrap"]).wrap.enabled);
    assert!(
        parse_config(&["--no-config", "--no-wrap=false"])
            .wrap
            .enabled
    );
    assert!(
        !parse_config(&["--no-config", "--no-wrap=true"])
            .wrap
            .enabled
    );
    assert!(parse_config(&["--no-config", "--indent-ampersand"]).indent_ampersand);
    assert!(!parse_config(&["--no-config", "--indent-ampersand=false"]).indent_ampersand);
}

#[test]
fn legacy_boolean_and_refactor_options_honor_explicit_values() {
    let parse_invocation = |args: &[&str]| {
        let argv = std::iter::once("forformat")
            .chain(args.iter().copied())
            .map(str::to_owned);
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected a formatting command");
        };
        invocation
    };

    let invocation = parse_invocation(&[
        "--no-config",
        "--last-indent",
        "--last-usable",
        "--uppercase-single-l=false",
        "--refactor-end=false",
    ]);
    assert!(invocation.config.last_indent);
    assert!(invocation.config.last_usable);
    assert!(!invocation.config.uppercase_single_l);
    assert!(!invocation.config.refactor_end);

    let invocation = parse_invocation(&[
        "--no-config",
        "--uppercase-single-l=true",
        "--refactor-end=true",
    ]);
    assert!(invocation.config.uppercase_single_l);
    assert!(invocation.config.refactor_end);
    assert!(!invocation.config.uppercase_end);

    let invocation = parse_invocation(&["--no-config", "--refactor-end=upcase"]);
    assert!(invocation.config.refactor_end);
    assert!(invocation.config.uppercase_end);

    let invocation = parse_invocation(&["--no-config", "--refactor-procedures=false"]);
    assert!(!invocation.config.refactor_end);

    for option in ["last-indent", "last-usable"] {
        let argument = format!("--{option}=false");
        assert!(parse(["forformat".to_string(), argument].into_iter()).is_err());
    }
    assert!(parse(
        [
            "forformat".to_string(),
            "--refactor-end=unexpected".to_string()
        ]
        .into_iter()
    )
    .is_err());
}

#[test]
fn valueless_workflow_and_mode_switches_reject_attached_values() {
    for option in [
        "all",
        "all-files",
        "stdin",
        "stdout",
        "isolated",
        "check",
        "diff",
        "show-files",
        "query-format",
        "full",
        "indent-only",
        "normalize-only",
        "no-config",
    ] {
        let argument = format!("--{option}=false");
        let argv = ["forformat".to_string(), argument.clone()];
        assert!(parse(argv.into_iter()).is_err(), "{argument} was accepted");
    }
}

#[test]
fn format_aliases_and_option_termination_are_explicit() {
    assert!(matches!(
        parse(["forformat".to_string(), "--input_format=free".to_string()].into_iter())
            .unwrap(),
        Command::Run(_)
    ));
    assert!(matches!(
        parse(["forformat".to_string(), "--output-format=same".to_string()].into_iter())
            .unwrap(),
        Command::Run(_)
    ));
    let terminated =
        parse(["forformat".to_string(), "--".to_string(), "-i4".to_string()]).unwrap();
    match terminated {
        Command::Run(invocation) => assert_eq!(invocation.paths, [PathBuf::from("-i4")]),
        _ => panic!("expected run"),
    }
}

#[test]
fn project_context_implies_stdin_and_rejects_file_workflows() {
    for args in [
        &["--project-context", ".", "source.f90"][..],
        &["--project-context", ".", "--check"][..],
        &["--query-format", "--project-context", "."][..],
        &["--project-context", ".", "--project-context", "."][..],
    ] {
        assert!(
            parse(
                std::iter::once("forformat".to_string())
                    .chain(args.iter().map(|arg| (*arg).to_string())),
            )
            .is_err(),
            "{args:?}"
        );
    }
    let Command::Run(invocation) = parse(
        ["forformat", "--stdin", "--project-context", "."]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap()
    else {
        panic!("expected run")
    };
    assert!(invocation.stdin);
    assert!(parse(
        ["forformat", "--project-context", "."]
            .into_iter()
            .map(str::to_owned),
    )
    .is_ok());
    assert!(parse(
        [
            "forformat",
            "--isolated",
            "source.f90",
            "--context-path",
            "src"
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .is_err());
}

#[test]
fn long_and_short_construct_aliases_produce_the_same_config() {
    assert_eq!(
        run(&[
            "-a5", "-b6", "-c2", "-d7", "-e4", "-E8", "-F9", "-f5", "-j6", "-m7", "-r8", "-s9",
            "-t4", "-w5", "-x6"
        ]),
        run(&[
            "--indent-associate=5",
            "--indent-block=6",
            "--indent-case=2",
            "--indent-do=7",
            "--indent-entry=4",
            "--indent-enum=8",
            "--indent-forall=9",
            "--indent-if=5",
            "--indent-interface=6",
            "--indent-module=7",
            "--indent-procedure=8",
            "--indent-select=9",
            "--indent-type=4",
            "--indent-where=5",
            "--indent-critical=6",
        ])
    );
    assert_eq!(run(&["-C-"]), run(&["--indent-contains=restart"]));
    assert_eq!(run(&["-K"]), run(&["--indent-ampersand"]));
    assert_eq!(run(&["-Rr"]), run(&["--refactor-end"]));
    assert_eq!(run(&["-RR"]), run(&["--refactor-end=upcase"]));
}

#[test]
fn long_alias_spellings_and_optional_values_have_a_matrix() {
    let aliases = [
        ("--indent_associate=5", "--indent-associate=5"),
        ("--indent_contains=restart", "--indent-contains=restart"),
        ("--include_left=1", "--include-left=1"),
        ("--label_left=0", "--label-left=0"),
        ("--input_format=free", "--input-format=free"),
        ("--output_format=same", "--output-format=same"),
    ];
    for (underscore, hyphen) in aliases {
        assert_eq!(
            run(&[underscore]),
            run(&[hyphen]),
            "{underscore} != {hyphen}"
        );
    }

    assert_eq!(run(&["--align_paren"]).align_paren_value, 1);
    assert!(!run(&["--align-paren=0"]).align_paren);
    assert!(run(&["--align_paren=1"]).align_paren);
    assert_eq!(run(&["--align-paren=7"]).align_paren_value, 7);
    assert_eq!(run(&["--ws_remred"]).ws_remred_value, 1);
    assert!(!run(&["--ws-remred=0"]).ws_remred);
    assert!(run(&["--ws_remred=1"]).ws_remred);
}

#[test]
fn every_documented_long_option_family_parses_with_a_value() {
    let options = [
        "--indent=4",
        "--start-indent=2",
        "--indent-contains=4",
        "--include-left=1",
        "--label-left=0",
        "--max-indent=12",
        "--openmp=0",
        "--indent-ampersand",
        "--indent-continuation=7",
        "--align-paren=3",
        "--ws-remred=1",
        "--align-declarations=0",
        "--align-comments=1",
        "--indent-changeteam=4",
        "--indent-associate=4",
        "--indent-block=4",
        "--indent-case=4",
        "--indent-contains=4",
        "--indent-do=4",
        "--indent-entry=4",
        "--indent-enum=4",
        "--indent-forall=4",
        "--indent-if=4",
        "--indent-interface=4",
        "--indent-module=4",
        "--indent-procedure=4",
        "--indent-select=4",
        "--indent-type=4",
        "--indent-where=4",
        "--refactor-end",
        "--refactor-end=upcase",
        "--last-indent",
        "--last-usable",
        "--input-format=free",
        "--output-format=free",
        "--output-format=same",
    ];
    for option in options {
        assert!(
            parse(["forformat".to_string(), option.to_string()].into_iter()).is_ok(),
            "{option}"
        );
    }
}

#[test]
fn missing_and_invalid_long_values_have_stable_diagnostics() {
    for option in [
        "--indent",
        "--start-indent",
        "--indent-contains",
        "--include-left",
        "--label-left",
        "--max-indent",
        "--openmp",
        "--indent-continuation",
        "--indent-changeteam",
        "--indent-if",
    ] {
        match parse(["forformat".to_string(), option.to_string()]) {
            Err(crate::error::FormatError::InvalidOption(message)) => {
                assert_eq!(message, "missing option value", "{option}")
            }
            _ => panic!("unexpected result for {option}"),
        }
    }
    for option in ["--include-left=2", "--label-left=maybe", "--openmp=maybe"] {
        assert!(matches!(
            parse(["forformat".to_string(), option.to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
    }
}

#[test]
fn rejected_long_values_have_stable_diagnostics() {
    for (args, expected) in [
        (&["--input-format=unknown"][..], "--input-format=unknown"),
        (&["--output_format=unknown"][..], "--output-format=unknown"),
        (
            &["--align_paren=-1"][..],
            "expected non-negative integer, got -1",
        ),
        (
            &["--ws_remred=no"][..],
            "expected non-negative integer, got no",
        ),
    ] {
        match parse(
            std::iter::once("forformat".to_string())
                .chain(args.iter().map(|arg| (*arg).to_string())),
        ) {
            Err(crate::error::FormatError::InvalidOption(value)) => {
                assert_eq!(value, expected)
            }
            _ => panic!("unexpected result for {args:?}"),
        }
    }
}

#[test]
fn mode_and_full_format_options_parse_and_do_not_collide_with_construct_names() {
    use crate::config::FormatMode;
    assert_eq!(run(&[]).mode, FormatMode::Full);
    assert_eq!(run(&["--full"]).mode, FormatMode::Full);
    assert_eq!(run(&["--normalize-only"]).mode, FormatMode::NormalizeOnly);
    assert_eq!(
        run(&["--full", "--indent-only"]).mode,
        FormatMode::IndentOnly
    );
    assert_eq!(run(&["--indent_only"]).mode, FormatMode::IndentOnly);

    assert!(run(&[]).wrap.enabled);
    assert!(!run(&["--no-wrap"]).wrap.enabled);
    assert!(!run(&["--wrap=0"]).wrap.enabled);
    assert_eq!(run(&["--line-length=100"]).wrap.line_length, 100);
    assert!(run(&["--uppercase-single-l"]).uppercase_single_l);
}

#[test]
fn style_options_parse_all_values_and_underscore_spellings() {
    use crate::config::KeywordCase;

    assert!(run(&[]).style.join_goto);
    assert!(run(&[]).style.split_compound_keywords);

    let config = run(&[
        "--keyword_case",
        "upper",
        "--relational-symbols=0",
        "--array_brackets",
        "0",
        "--compact-multiplicative=0",
        "--join_goto=0",
        "--split-compound-keywords",
        "0",
        "--strip_empty_args=0",
        "--remove-redundant-parens",
        "0",
        "--remove_terminal_return=0",
        "--program-unit-spacing",
        "0",
        "--max_blank_lines",
        "preserve",
        "--delimiter-spacing=0",
        "--comment_spacing",
        "0",
        "--continuation-markers=0",
    ]);
    assert_eq!(config.style.keyword_case, KeywordCase::Upper);
    assert!(!config.style.relational_symbols);
    assert!(!config.style.array_brackets);
    assert!(!config.style.compact_multiplicative);
    assert!(!config.style.join_goto);
    assert!(!config.style.split_compound_keywords);
    assert!(!config.style.strip_empty_args);
    assert!(!config.style.remove_redundant_parens);
    assert!(!config.style.remove_terminal_return);
    assert!(!config.style.program_unit_spacing);
    assert_eq!(config.style.max_blank_lines, None);
    assert!(!config.style.delimiter_spacing);
    assert!(!config.style.comment_spacing);
    assert!(!config.style.continuation_markers);

    fn style_bool(config: &crate::config::FormatConfig, option: &str) -> bool {
        match option {
            "relational-symbols" => config.style.relational_symbols,
            "array-brackets" => config.style.array_brackets,
            "compact-multiplicative" => config.style.compact_multiplicative,
            "join-goto" => config.style.join_goto,
            "split-compound-keywords" => config.style.split_compound_keywords,
            "strip-empty-args" => config.style.strip_empty_args,
            "remove-redundant-parens" => config.style.remove_redundant_parens,
            "remove-terminal-return" => config.style.remove_terminal_return,
            "program-unit-spacing" => config.style.program_unit_spacing,
            "delimiter-spacing" => config.style.delimiter_spacing,
            "comment-spacing" => config.style.comment_spacing,
            "continuation-markers" => config.style.continuation_markers,
            _ => unreachable!(),
        }
    }
    let bools = [
        "relational-symbols",
        "array-brackets",
        "compact-multiplicative",
        "join-goto",
        "split-compound-keywords",
        "strip-empty-args",
        "remove-redundant-parens",
        "remove-terminal-return",
        "program-unit-spacing",
        "delimiter-spacing",
        "comment-spacing",
        "continuation-markers",
    ];
    for option in bools {
        for spelling in [option.to_string(), option.replace('-', "_")] {
            let zero = format!("--{spelling}=0");
            let one = format!("--{spelling}=1");
            assert!(!style_bool(&run(&[zero.as_str()]), option));
            assert!(style_bool(&run(&[one.as_str()]), option));
        }
    }

    assert_eq!(run(&["--max-blank-lines=0"]).style.max_blank_lines, Some(0));
    assert_eq!(
        run(&["--max-blank-lines", "7"]).style.max_blank_lines,
        Some(7)
    );
}

#[test]
fn style_options_report_the_option_bad_value_and_allowed_values() {
    let result = parse([
        "forformat".to_string(),
        "--strip-empty-args=maybe".to_string(),
    ]);
    assert!(matches!(
        result,
        Err(crate::error::FormatError::InvalidOption(message))
            if message == "expected 0 or 1, got maybe"
    ));
    let result = parse([
        "forformat".to_string(),
        "--keyword-case=invalid".to_string(),
    ]);
    assert!(matches!(
        result,
        Err(crate::error::FormatError::InvalidOption(message))
            if message.contains("keyword-case")
                && message.contains("invalid")
                && message.contains("allowed values")
    ));
    assert!(matches!(
        parse(["forformat", "--max-blank-lines=bad"].into_iter().map(str::to_owned)),
        Err(crate::error::FormatError::InvalidOption(message))
            if message.contains("bad")
    ));
    for option in [
        "--keyword-case",
        "--array-brackets",
        "--compact-multiplicative",
        "--join-goto",
        "--split-compound-keywords",
        "--strip-empty-args",
        "--relational-symbols",
        "--remove-redundant-parens",
        "--remove-terminal-return",
        "--program-unit-spacing",
        "--delimiter-spacing",
        "--comment-spacing",
        "--continuation-markers",
        "--max-blank-lines",
    ] {
        assert!(matches!(
            parse(["forformat".to_string(), option.to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(message))
                if message == "missing option value"
        ));
    }
}

#[test]
fn alignment_reduction_toggles_default_on_for_declarations_and_off_for_comments() {
    let default = run(&[]);
    assert!(default.align_declarations);
    assert!(!default.align_comments);
    assert!(!run(&["--align-declarations=0"]).align_declarations);
    assert!(run(&["--align-comments=1"]).align_comments);
}

#[test]
fn macro_definitions_accumulate_in_order_from_both_spellings() {
    let config = run(&["-DFIRST", "-D", "Second=2", "--define=Third"]);
    let names: Vec<&str> = config
        .defines
        .iter()
        .map(|define| define.name.as_str())
        .collect();
    assert_eq!(names, ["FIRST", "Second", "Third"]);
    assert_eq!(config.defines[1].value.as_deref(), Some("2"));
    assert_eq!(config.defines[0].value, None);
}

#[test]
fn exclude_accepts_repeatable_separated_and_normalized_spellings() {
    let mut argv = vec!["forformat".to_string()];
    argv.extend(
        [
            "--no-config",
            "--exclude=vendor/**",
            "--exclude",
            "generated/",
        ]
        .iter()
        .map(|arg| (*arg).to_string()),
    );
    let Command::Run(invocation) = parse(argv).unwrap() else {
        panic!("expected a formatting command");
    };
    assert_eq!(invocation.exclude_patterns(), ["vendor/**", "generated/"]);

    let Command::Run(invocation) = parse(
        ["forformat", "--no_config", "--EXCLUDE=vendor/**"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap()
    else {
        panic!("expected a formatting command");
    };
    assert_eq!(invocation.exclude_patterns(), ["vendor/**"]);
}

#[test]
fn extend_exclude_adds_to_the_set_exclude_selects() {
    let run = |args: &[&str]| {
        let argv = std::iter::once("forformat")
            .chain(args.iter().copied())
            .map(str::to_owned);
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected a formatting command");
        };
        invocation
    };

    let invocation = run(&["--no-config", "--extend-exclude=generated/"]);
    assert!(invocation.exclude.is_none());
    assert_eq!(invocation.extend_exclude, ["generated/"]);
    assert_eq!(
        invocation.exclude_patterns(),
        DEFAULT_EXCLUDES
            .iter()
            .map(|s| (*s).to_string())
            .chain(["generated/".to_string()])
            .collect::<Vec<_>>()
    );

    let invocation = run(&[
        "--no-config",
        "--exclude=vendor/",
        "--extend_exclude=generated/",
    ]);
    assert_eq!(invocation.exclude_patterns(), ["vendor/", "generated/"]);
}

#[test]
fn unsupported_and_invalid_cli_paths_have_stable_categories() {
    assert!(matches!(
        parse(["forformat".to_string(), "-ifixed".to_string()].into_iter()),
        Err(crate::error::FormatError::Unsupported(_))
    ));
    assert!(matches!(
        parse(["forformat".to_string(), "--not-an-option".to_string()].into_iter()),
        Err(crate::error::FormatError::InvalidOption(_))
    ));
    assert!(matches!(
        parse(["forformat".to_string(), "-i".to_string()].into_iter()),
        Err(crate::error::FormatError::InvalidOption(_))
    ));
    assert!(matches!(
        parse(["forformat".to_string(), "--include-left=maybe".to_string()].into_iter()),
        Err(crate::error::FormatError::InvalidOption(_))
    ));
}

#[test]
fn single_dash_long_option_typos_explain_the_required_spelling() {
    for (typo, expected) in [
        (
            "-all",
            "-all (did you mean --all? Long options use two dashes.)",
        ),
        (
            "-indent_module=0",
            "-indent_module=0 (did you mean --indent_module=0? Long options use two dashes.)",
        ),
    ] {
        match parse(["forformat".to_string(), typo.to_string()]) {
            Err(crate::error::FormatError::InvalidOption(message)) => {
                assert_eq!(message, expected)
            }
            _ => panic!("unexpected result for {typo}"),
        }
    }

    assert!(parse(["forformat".to_string(), "-i4".to_string()].into_iter()).is_ok());
    assert!(parse(["forformat".to_string(), "-ifree".to_string()].into_iter()).is_ok());
}

#[test]
fn file_workflow_flags_and_query_mode_validation_are_explicit() {
    use crate::config::FormatMode;
    let parsed = parse(
        ["forformat", "--full", "--all"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    match parsed {
        Command::Run(invocation) => {
            assert!(invocation.all);
            assert_eq!(invocation.config.mode, FormatMode::Full);
        }
        _ => panic!("expected run"),
    }
    assert!(parse(
        ["forformat", "-lastindent", "--check", "x.f90"]
            .into_iter()
            .map(str::to_owned),
    )
    .is_err());
    assert!(parse(
        ["forformat", "--stdout", "x.f90", "y.f90"]
            .into_iter()
            .map(str::to_owned),
    )
    .is_err());
}

#[test]
fn option_metadata_drives_help_and_single_dash_suggestions() {
    let usage = super::usage();
    for spelling in ["--all", "--project-context=<path>", "--keyword-case"] {
        assert!(usage.contains(spelling), "missing {spelling} from help");
    }
    assert!(matches!(
        parse(["forformat", "-all"].into_iter().map(str::to_owned)),
        Err(crate::error::FormatError::InvalidOption(message))
            if message.contains("did you mean --all?")
    ));
}
