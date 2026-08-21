use forformat::{format_source, FormatConfig, FormatMode};

fn canonicalize_config() -> FormatConfig {
    let mut config = FormatConfig {
        mode: FormatMode::NormalizeOnly,
        ..FormatConfig::default()
    };
    config.style.normalize_whitespace = false;
    config
}

#[test]
fn canonicalize_only_keeps_incidental_whitespace_and_exact_line_endings() {
    let source = b"\tENDDO   \r\nx  .EQ.  y ! gap\nENDIF\t";
    let output = format_source(source, &canonicalize_config()).unwrap().bytes;

    assert_eq!(output, b"\tend do   \r\nx  ==  y ! gap\nend if\t");
}

#[test]
fn canonicalize_only_refactor_end_keeps_authored_layout() {
    let source = b"module M\r\n\tEND   ! note\n";
    let mut config = canonicalize_config();
    config.refactor_end = true;

    let output = format_source(source, &config).unwrap().bytes;
    assert_eq!(output, b"module M\r\n\tend module M   ! note\n");
}

#[test]
fn rewrap_reconsiders_fitting_authored_continuations_and_is_idempotent() {
    let source = b"program p\ncall work(alpha, &\n    beta)\nend program p\n";
    let config = FormatConfig {
        rewrap: true,
        ..FormatConfig::default()
    };

    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;

    assert_eq!(once, twice);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("call work(alpha, beta)"), "{output}");
    assert!(!output.contains("work(alpha, &"), "{output}");
}

#[test]
fn rewrap_leaves_comment_bearing_continuations_on_the_existing_safe_path() {
    let source = b"program p\ncall work(alpha, & ! keep\n    beta)\nend program p\n";
    let config = FormatConfig {
        rewrap: true,
        ..FormatConfig::default()
    };

    let output = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("! keep"), "{text}");
    assert!(text.contains('&'), "{text}");
}

#[test]
fn rewrap_restores_a_wider_wrap_after_a_narrower_wrap() {
    let source = concat!(
        "subroutine test\n",
        "s(6) = ((-vb - sigma)*w%ddwinV(j) + (-4.d0*adotoa**2*sigma - ",
        "(18.d0*gpres + 6.d0*grho)*sigma/18.d0)*w%winV(j) + ",
        "((-4.d0*sigma - vb)*adotoa - vbdot + (grho*sigma/2.d0 + ",
        "vb*grho/3.d0)/adotoa + (-grho**2*sigma/18.d0 - ",
        "vb*grho**2/18.d0)/adotoa**3)*w%wing(j) + w%dwing(j)*vb + ",
        "(-w%ddwing(j)*sigma - w%ddwing(j)*vb)/adotoa + ",
        "4.d0*w%dwinV(j)*sigma*adotoa + 4.d0*w%dwing(j)*sigma + ",
        "(-w%dwing(j)*grho*sigma/3.d0 - w%dwing(j)*vb*grho/3.d0)/adotoa**2 - ",
        "w%dwinV(j)*vbdot + ((2.d0*etak - etak*grho/adotoa**2/3.d0)*w%wing(j) - ",
        "2.d0*w%dwing(j)*etak/adotoa - 2.d0*w%dwinV(j)*etak + ",
        "2.d0*etak*adotoa*w%winV(j))/EV%Kf(1))*exptau - visibility*w%dwinV(j)*vb + ",
        "(4.d0*visibility*sigma*adotoa - dvisibility*sigma)*w%winV(j)\n",
        "end subroutine test\n",
    )
    .as_bytes();

    let mut width_120 = FormatConfig::default();
    width_120.wrap.line_length = 120;
    let at_120 = format_source(source, &width_120).unwrap().bytes;

    let mut width_77 = width_120.clone();
    width_77.wrap.line_length = 77;
    let at_77 = format_source(&at_120, &width_77).unwrap().bytes;
    assert_ne!(at_77, at_120, "77-column pass did not exercise wrapping");

    let mut rewrap_120 = width_120;
    rewrap_120.rewrap = true;
    let restored = format_source(&at_77, &rewrap_120).unwrap().bytes;

    assert_eq!(restored, at_120);
}
