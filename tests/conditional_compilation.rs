use forformat::{format_source, FormatConfig, FormatMode};

fn full() -> FormatConfig {
    FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    }
}

fn conditional_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|line| line.starts_with("!$")).collect()
}

#[test]
fn conditional_continuation_spellings_converge_to_valid_exact_output() {
    for continuation in ["!$ & arg = 1)", "!$\t& arg = 1)", "!$& arg = 1)"] {
        let source = format!(
            "program p\n!$ call f( &\n{continuation}\nend program p\n"
        );
        let config = full();
        let output = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(output.clone()).unwrap();

        assert_eq!(
            conditional_lines(&text),
            ["!$ call f( &", "!$    arg=1)"],
            "{continuation:?} produced:\n{text}"
        );
        assert_eq!(format_source(&output, &config).unwrap().bytes, output);
    }
}

#[test]
fn compact_conditional_literal_continuation_keeps_the_required_ampersand() {
    let source = b"program p\ncharacter(len=40) :: s\n!$ s = 'abc &\n!$& def!ghi'\nend program p\n";

    for config in [
        full(),
        FormatConfig {
            mode: FormatMode::Full,
            align_comments: true,
            ..FormatConfig::default()
        },
    ] {
        let output = format_source(source, &config).unwrap().bytes;
        let text = String::from_utf8(output.clone()).unwrap();
        assert_eq!(
            conditional_lines(&text),
            ["!$ s = 'abc &", "!$ & def!ghi'"],
            "{text}"
        );
        assert_eq!(format_source(&output, &config).unwrap().bytes, output);
    }
}
