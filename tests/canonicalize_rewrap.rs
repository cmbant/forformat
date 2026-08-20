use forformat::{format_source, FormatConfig, FormatMode};

fn canonicalize_config() -> FormatConfig {
    let mut config = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };
    config.style.normalize_whitespace = false;
    config
}

#[test]
fn canonicalize_only_keeps_incidental_whitespace_and_exact_line_endings() {
    let source = b"\tENDDO   \r\nx  .EQ.  y ! gap\nENDIF\t";
    let output = format_source(source, &canonicalize_config()).unwrap().bytes;

    assert_eq!(output, b"\tend do   \r\nx  ==  y ! gap\nend if\t");
}

#[test]
fn canonicalize_only_refactor_end_keeps_authored_layout() {
    let source = b"module M\r\n\tEND   ! note\n";
    let mut config = canonicalize_config();
    config.refactor_end = true;

    let output = format_source(source, &config).unwrap().bytes;
    assert_eq!(output, b"module M\r\n\tend module M   ! note\n");
}

#[test]
fn rewrap_reconsiders_fitting_authored_continuations_and_is_idempotent() {
    let source = b"program p\ncall work(alpha, &\n    beta)\nend program p\n";
    let config = FormatConfig {
        rewrap: true,
        ..FormatConfig::default()
    };

    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;

    assert_eq!(once, twice);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("call work(alpha, beta)"), "{output}");
    assert!(!output.contains("work(alpha, &"), "{output}");
}

#[test]
fn rewrap_leaves_comment_bearing_continuations_on_the_existing_safe_path() {
    let source = b"program p\ncall work(alpha, & ! keep\n    beta)\nend program p\n";
    let config = FormatConfig {
        rewrap: true,
        ..FormatConfig::default()
    };

    let output = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("! keep"), "{text}");
    assert!(text.contains('&'), "{text}");
}
