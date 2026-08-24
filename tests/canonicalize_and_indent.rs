use forformat::{
    cli::{parse, Command},
    format_source, FormatConfig, FormatMode,
};
use std::fs;

fn format_mode(source: &[u8], base: &FormatConfig, mode: FormatMode) -> Vec<u8> {
    let mut config = base.clone();
    config.mode = mode;
    format_source(source, &config).unwrap().bytes
}

#[test]
fn canonicalize_and_indent_matches_the_two_step_pipeline() {
    let source = concat!(
        "PROGRAM P\r\n",
        "IF (x  .EQ.  y) THEN\n",
        "integer      :: value\r\n",
        "call work(alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa, lambda, mu, nu, xi, omicron, pi, rho, sigma)\n",
        "ENDIF\r\n",
        "ENDPROGRAM P",
    )
    .as_bytes();
    let base = FormatConfig::default();

    let canonical = format_mode(source, &base, FormatMode::CanonicalizeOnly);
    let expected = format_mode(&canonical, &base, FormatMode::IndentOnly);
    let actual = format_mode(source, &base, FormatMode::CanonicalizeAndIndent);

    assert_eq!(actual, expected);
    let text = String::from_utf8(actual).unwrap();
    assert!(text.contains("if (x  ==  y) then"), "{text}");
    assert!(text.contains("integer      :: value"), "{text}");
    assert!(
        !text.contains(" &\n"),
        "combined mode unexpectedly wrapped:\n{text}"
    );
}

#[test]
fn canonicalize_and_indent_is_idempotent() {
    let source = b"program p\nif (x .eq. y) then\nx=1\nendif\nendprogram p\n";
    let config = FormatConfig {
        mode: FormatMode::CanonicalizeAndIndent,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(twice, once);
}

#[test]
fn canonicalize_and_indent_has_the_expected_capabilities() {
    let mode = FormatMode::CanonicalizeAndIndent;
    assert!(mode.normalizes());
    assert!(mode.lays_out());
    assert!(!mode.normalizes_whitespace());
    assert!(!mode.aligns_after_layout());
    assert!(!mode.wraps());
}

#[test]
fn cli_selects_the_combined_mode_and_last_mode_wins() {
    let mode = |args: &[&str]| {
        let argv = std::iter::once("forformat")
            .chain(args.iter().copied())
            .map(str::to_owned);
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected formatting command");
        };
        invocation.config.mode
    };

    assert_eq!(
        mode(&["--no-config", "--canonicalize-and-indent"]),
        FormatMode::CanonicalizeAndIndent
    );
    assert_eq!(
        mode(&["--no-config", "--canonicalize_and_indent"]),
        FormatMode::CanonicalizeAndIndent
    );
    assert_eq!(
        mode(&["--no-config", "--canonicalize-and-indent", "--full"]),
        FormatMode::Full
    );
    assert_eq!(
        mode(&["--no-config", "--full", "--canonicalize-and-indent"]),
        FormatMode::CanonicalizeAndIndent
    );

    assert!(parse(
        [
            "forformat",
            "--no-config",
            "--canonicalize-and-indent=false"
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .is_err());
    let error = parse(
        [
            "forformat",
            "--no-config",
            "--canonicalize-and-indent",
            "--rewrap",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap_err();
    assert!(error.to_string().contains("--rewrap requires full mode"));
}

#[test]
fn toml_can_select_canonicalize_and_indent() {
    let path = std::env::temp_dir().join(format!(
        "forformat-canonicalize-and-indent-{}.toml",
        std::process::id()
    ));
    fs::write(&path, "mode = 'canonicalize-and-indent'\n").unwrap();
    let argv = [
        "forformat".to_string(),
        format!("--config={}", path.display()),
    ];
    let parsed = parse(argv).unwrap();
    let _ = fs::remove_file(&path);
    let Command::Run(invocation) = parsed else {
        panic!("expected formatting command");
    };
    assert_eq!(invocation.config.mode, FormatMode::CanonicalizeAndIndent);
}
