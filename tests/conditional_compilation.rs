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
        let source = format!("program p\n!$ call f( &\n{continuation}\nend program p\n");
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
fn standalone_compact_prefix_is_not_promoted_to_conditional_code() {
    let source = b"program p\n!$& standalone\nend program p\n";
    for config in [
        full(),
        FormatConfig {
            mode: FormatMode::IndentOnly,
            ..FormatConfig::default()
        },
    ] {
        let output = format_source(source, &config).unwrap().bytes;
        let text = String::from_utf8(output).unwrap();
        assert!(text.lines().any(|line| line == "!$& standalone"), "{text}");
    }
}

#[test]
fn hollerith_payload_ampersand_does_not_promote_following_compact_prefix() {
    let source = b"program p
!$ x = 1H&
!$& standalone
end program p
";
    let config = full();
    let output = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(output.clone()).unwrap();

    assert!(text.lines().any(|line| line == "!$& standalone"), "{text}");
    assert!(text.lines().any(|line| line.contains("1H&")), "{text}");
    assert!(!text.lines().any(|line| line.contains("1H &")), "{text}");
    assert_eq!(format_source(&output, &config).unwrap().bytes, output);
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

#[test]
fn conditional_multiple_subscripts_survive_continuation_and_forced_wrapping() {
    // The first multiple-subscript triplet is physically continued through a
    // compact conditional sentinel. The joined prefix remains short enough to
    // offer a safe comma break, while the long outer expression forces more
    // wrapper-created physical lines after that state has been carried. This
    // exercises both PR #16's `@` state and PR #18's conditional wrapper path.
    let source = b"program p\n!$ x = values(@ lo : &\n!$& hi : step, @ base :: stride) + first_term + second_term + third_term + fourth_term + fifth_term + sixth_term\nend program p\n";
    let mut config = full();
    config.wrap.line_length = 48;

    let output = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(output.clone()).unwrap();
    let conditional: Vec<_> = text
        .lines()
        .filter(|line| line.trim_start().starts_with("!$"))
        .collect();

    assert!(
        conditional.len() > 2,
        "expected forced wrapping to add a conditional line:\n{text}"
    );
    assert!(
        conditional
            .iter()
            .all(|line| line.trim_start().starts_with("!$ ")),
        "generated conditional line lost its sentinel:\n{text}"
    );
    assert!(
        conditional.iter().all(|line| line.len() <= 48),
        "conditional wrap exceeded its budget:\n{text}"
    );

    let logical: String = conditional
        .iter()
        .flat_map(|line| {
            line.trim_start()
                .strip_prefix("!$ ")
                .expect("all conditional lines use the canonical sentinel")
                .chars()
        })
        .filter(|character| !character.is_whitespace() && *character != '&')
        .collect();
    assert!(
        logical.contains("@lo:hi:step"),
        "triplet state lost:\n{text}"
    );
    assert!(
        logical.contains("@base::stride"),
        "continued multiple-subscript :: was reinterpreted:\n{text}"
    );
    assert!(
        !conditional.iter().any(|line| line.contains(":: "))
            && !conditional
                .iter()
                .any(|line| line.trim_end().ends_with(":: &")),
        "multiple-subscript :: was treated as declaration/type-spec punctuation:\n{text}"
    );
    assert_eq!(format_source(&output, &config).unwrap().bytes, output);
}
