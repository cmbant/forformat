use forformat::{format_source, FormatConfig, FormatMode};

fn full(source: &[u8]) -> Vec<u8> {
    let config = FormatConfig {
        mode: FormatMode::Full,
        apply_indent: false,
        ..FormatConfig::default()
    };
    format_source(source, &config).unwrap().bytes
}

#[test]
fn if_condition_spacing_is_per_statement_after_a_semicolon() {
    let once = full(b"y = 1; if(a)x=1\n");
    assert_eq!(once, b"y = 1; if (a) x = 1\n");
    assert_eq!(full(&once), once);
}

#[test]
fn noncanonical_code_whitespace_canonicalizes_in_one_pass() {
    for &control in b"\x0b\x0c\r" {
        let mut source = b"real a ".to_vec();
        source.push(control);
        source.extend_from_slice(b"  ! c\n");
        let once = full(&source);
        assert_eq!(once, b"real a ! c\n", "control byte {control:#04x}");
        assert_eq!(full(&once), once, "control byte {control:#04x}");
    }
}

#[test]
fn rule_one_sees_canonical_presentation_whitespace() {
    for &control in b"\x0b\x0c\r" {
        let mut source = b"NOTIFY".to_vec();
        source.push(control);
        source.extend_from_slice(b"WAIT\n");
        let once = full(&source);
        assert_eq!(once, b"notify wait\n", "control byte {control:#04x}");
        assert_eq!(full(&once), once, "control byte {control:#04x}");
    }
}

#[test]
fn protected_control_whitespace_is_preserved() {
    let literal = full(b"print *, 'a\x0cb' ! c\n");
    assert!(literal.windows(3).any(|window| window == b"a\x0cb"));
    assert_eq!(full(&literal), literal);

    let hollerith = full(b"call p(3Ha\x0cb, x)\n");
    assert!(hollerith.windows(5).any(|window| window == b"3Ha\x0cb"));
    assert_eq!(full(&hollerith), hollerith);
}

#[test]
fn hollerith_payload_ampersand_does_not_continue_the_statement() {
    let once = full(b"x = 1H&\ny = 2\n");
    assert_eq!(once, b"x = 1H&\ny = 2\n");
    assert_eq!(full(&once), once);
}
