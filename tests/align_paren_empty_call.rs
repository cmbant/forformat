use forformat::{format_source, FormatConfig, FormatMode};

const SOURCE: &[u8] = b"\
subroutine s(obj)
  call obj%a%Initiate(mesh=obj%mesh, &
                      material=obj%solid(1)%ptr, &
                      name=obj%b%chars(), &
                      engine=obj%engine%chars())
end subroutine
";

fn config(mode: FormatMode) -> FormatConfig {
    FormatConfig {
        mode,
        align_paren: true.into(),
        ..FormatConfig::default()
    }
}

#[test]
fn empty_nested_call_keeps_the_enclosing_alignment_in_every_layout_mode() {
    // An empty `chars()` must not make the next item the parent's alignment
    // target; findent 4.3.8 does exactly that and indents `engine=` far right.
    for mode in [FormatMode::IndentOnly, FormatMode::Full] {
        let once = format_source(SOURCE, &config(mode)).unwrap().bytes;
        let twice = format_source(&once, &config(mode)).unwrap().bytes;
        let text = String::from_utf8(once).unwrap();

        assert_eq!(text.as_bytes(), twice, "{mode:?}");
        for line in text.lines().skip(2).take(3) {
            assert_eq!(line.len() - line.trim_start().len(), 23, "{mode:?}\n{text}");
        }
    }
}
