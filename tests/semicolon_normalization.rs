use forformat::{format_source, FormatConfig, FormatMode};

#[test]
fn redundant_semicolons_are_normalization_not_indentation() {
    let source = b"call work();;\n";
    let normalize = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };
    let once = format_source(source, &normalize).unwrap().bytes;
    assert_eq!(once, b"call work()\n");
    assert_eq!(format_source(&once, &normalize).unwrap().bytes, once);

    let indent_only = FormatConfig {
        mode: FormatMode::IndentOnly,
        ..FormatConfig::default()
    };
    assert_eq!(
        format_source(source, &indent_only).unwrap().bytes,
        b"call work();;\n"
    );
}

#[test]
fn the_style_toggle_turns_separator_normalization_off() {
    let source = b"call a();;; call b();\n";
    let mut config = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };
    assert_eq!(
        format_source(source, &config).unwrap().bytes,
        b"call a(); call b()\n"
    );

    config.style.normalize_semicolons = false;
    assert_eq!(format_source(source, &config).unwrap().bytes, source);
}

#[test]
fn separator_normalization_survives_canonicalize_only() {
    // Semicolon runs are a token-level spelling choice, not presentation
    // whitespace, so the canonicalization preset keeps the rule active.
    let mut config = FormatConfig {
        mode: FormatMode::CanonicalizeOnly,
        ..FormatConfig::default()
    };
    // The interior blanks around the surviving `;` are incidental whitespace
    // this preset does not own, so they stay. The blank the dropped trailing
    // `;` left behind is at end of line, which no mode keeps.
    assert_eq!(
        format_source(b"\tx = 1 ;; y = 2 ;\n", &config)
            .unwrap()
            .bytes,
        b"\tx = 1 ; y = 2\n"
    );

    config.style.normalize_semicolons = false;
    assert_eq!(
        format_source(b"\tx = 1 ;; y = 2 ;\n", &config)
            .unwrap()
            .bytes,
        b"\tx = 1 ;; y = 2 ;\n"
    );
}
