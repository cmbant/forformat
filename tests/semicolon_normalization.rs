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
