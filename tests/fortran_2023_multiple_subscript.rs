use forformat::{format_source, FormatConfig, FormatMode};

fn normalize(source: &[u8]) -> Vec<u8> {
    format_source(
        source,
        &FormatConfig {
            mode: FormatMode::NormalizeOnly,
            apply_indent: false,
            ..FormatConfig::default()
        },
    )
    .unwrap()
    .bytes
}

#[test]
fn nested_multiple_subscripts_leave_nested_section_colons_alone() {
    let source = b"x = e(@ f(i : j), @ [g(1,2),h(3,4)])\n";
    let once = normalize(source);
    assert_eq!(normalize(&once), once);
    assert_eq!(
        String::from_utf8(once).unwrap(),
        "x = e(@f(i : j), @[g(1, 2), h(3, 4)])\n"
    );
}

#[test]
fn authored_continuation_after_multiple_subscript_prefix_is_preserved() {
    let source = b"x = c(@ &\n& v1, @ lo : &\n& hi : step)\n";
    let once = normalize(source);
    assert_eq!(normalize(&once), once);
    let output = String::from_utf8(once).unwrap();

    // Trailing continuation markers are structural and remain. The formatter's
    // established continuation policy removes optional leading `&` markers.
    assert!(output.contains("@ &\nv1"));
    assert!(output.contains("@lo: &\nhi:step)"));
}

#[test]
fn triplet_state_survives_authored_continuations() {
    for (source, expected) in [
        (
            b"x = c(@ lo : &\n& hi : step)\n".as_slice(),
            "x = c(@lo: &\nhi:step)\n",
        ),
        (
            b"x = c(@ lo &\n& : hi : step)\n",
            "x = c(@lo &\n:hi:step)\n",
        ),
        (
            b"x = c(@ lo : hi : &\n& step)\n",
            "x = c(@lo:hi: &\nstep)\n",
        ),
    ] {
        let once = normalize(source);
        assert_eq!(normalize(&once), once);
        assert_eq!(String::from_utf8(once).unwrap(), expected);
    }
}

#[test]
fn continued_triplet_depth_ignores_nested_and_sibling_section_colons() {
    let source = b"x = c(@ f(i : &\n& j) : hi : step, @ lo : &\n& hi, j : k)\n";
    let once = normalize(source);
    assert_eq!(normalize(&once), once);
    let output = String::from_utf8(once).unwrap();

    assert!(output.contains("@f(i : &\nj):hi:step"));
    assert!(output.contains("@lo: &\nhi, j : k)"));
}

#[test]
fn post_layout_alignment_keeps_continued_double_colons_compact() {
    let config = FormatConfig {
        apply_indent: false,
        ..FormatConfig::default()
    };

    for (source, first_line) in [
        (b"x = c(@ &\n& ::stride, other)\n".as_slice(), "@ &"),
        (b"x = c(@lo &\n& ::stride, other)\n".as_slice(), "@lo &"),
    ] {
        let once = format_source(source, &config).unwrap().bytes;
        let output = String::from_utf8(once.clone()).unwrap();
        assert!(
            output.contains(&format!("{first_line}\n::stride")),
            "continued multiple-subscript lost compact `::`: {output:?}"
        );
        assert!(
            !output.lines().any(|line| line.contains(":: stride")),
            "post-layout alignment respaced multiple-subscript :: {output:?}"
        );
        assert_eq!(format_source(&once, &config).unwrap().bytes, once);
    }
}

#[test]
fn wrapping_cannot_split_after_at_when_delimiter_spacing_is_disabled() {
    let mut config = FormatConfig {
        apply_indent: false,
        ..FormatConfig::default()
    };
    config.style.delimiter_spacing = false;
    config.wrap.line_length = 44;
    let output = String::from_utf8(
        format_source(
            b"x = some_really_long_array_name(@ [1,2,3,4,5,6,7,8], another_argument)\n",
            &config,
        )
        .unwrap()
        .bytes,
    )
    .unwrap();

    assert!(output.contains("@[1,2"));
    assert!(output.contains("&\n"));
    assert!(!output.lines().any(|line| line.trim_end().ends_with("@ &")));
}

#[test]
fn wrapping_keeps_multiple_subscript_double_colons_compact() {
    let mut config = FormatConfig {
        apply_indent: false,
        ..FormatConfig::default()
    };
    // Both commas are reachable within the first-line budget, while the full
    // statements still require reflow. This makes the regression exercise the
    // wrapper's preference rather than an unbreakable-budget decline.
    config.wrap.line_length = 55;

    for (source, designator) in [
        (
            b"x = some_really_long_array_name(@::stride, another_long_argument)\n".as_slice(),
            "@::stride",
        ),
        (
            b"x = some_really_long_array_name(@lo::stride, another_long_argument)\n".as_slice(),
            "@lo::stride",
        ),
    ] {
        let output = String::from_utf8(format_source(source, &config).unwrap().bytes).unwrap();
        assert!(output.contains("&\n"), "expected wrapping: {output:?}");
        assert!(
            output.contains(designator),
            "designator was respaced: {output:?}"
        );
        assert!(
            !output.lines().any(|line| line.trim_end().ends_with(":: &")),
            "multiple-subscript :: became a wrap seam: {output:?}"
        );
    }
}
