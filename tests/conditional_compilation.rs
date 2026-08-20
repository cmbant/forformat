use forformat::{format_source, FormatConfig, FormatMode};

#[test]
fn compact_conditional_continuation_uses_full_statement_state() {
    let source = b"program p\n!$ call f( &\n!$& arg = 1)\nend program p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };

    let output = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(output.clone()).unwrap();

    assert!(text.contains("arg=1)"), "{text}");
    assert_eq!(text.matches("!$").count(), 2, "{text}");
    assert_eq!(format_source(&output, &config).unwrap().bytes, output);
}

#[test]
fn compact_conditional_literal_continuation_preserves_protected_text() {
    let source = b"program p\ncharacter(len=40) :: s\n!$ s = 'abc &\n!$& def!ghi'\nend program p\n";

    for config in [
        FormatConfig {
            mode: FormatMode::Full,
            ..FormatConfig::default()
        },
        FormatConfig {
            mode: FormatMode::Full,
            align_comments: true,
            ..FormatConfig::default()
        },
    ] {
        let output = format_source(source, &config).unwrap().bytes;
        let text = String::from_utf8(output.clone()).unwrap();
        assert!(text.contains("def!ghi"), "{text}");
        assert_eq!(text.matches("!$").count(), 2, "{text}");
        assert_eq!(format_source(&output, &config).unwrap().bytes, output);
    }
}
