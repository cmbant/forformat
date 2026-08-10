use findent::{format_source, FormatConfig};

#[test]
fn core_fixture_is_idempotent() {
    let source = include_bytes!("fixtures/core.f90");
    let config = FormatConfig::default();
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice);
}

#[test]
fn default_mode_preserves_source_body_bytes_except_trailing_space() {
    let source = b"program p\n  ! caf\xe9\nx=1  +  2 ! keep ! punctuation\nend\n";
    let output = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(output
        .windows(b"caf\xe9".len())
        .any(|window| window == b"caf\xe9"));
    assert!(output
        .windows(b"! keep ! punctuation".len())
        .any(|window| { window == b"! keep ! punctuation" }));

    assert!(!output.windows(3).any(|window| window == b"  \n"));
}
