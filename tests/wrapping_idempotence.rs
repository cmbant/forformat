use forformat::{format_source, FormatConfig};

#[test]
fn a_wrap_inside_a_relational_run_is_idempotent() {
    let mut source = b"program p\ncon".to_vec();
    source.extend(std::iter::repeat(b'>').take(113));
    source.extend_from_slice(b"tinue\nend program p\n");

    let config = FormatConfig::default();
    let once = format_source(&source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice);
}
