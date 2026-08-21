use forformat::{format_source, FormatConfig, FormatMode};

fn canonicalize_config() -> FormatConfig {
    FormatConfig {
        mode: FormatMode::CanonicalizeOnly,
        ..FormatConfig::default()
    }
}

#[test]
fn canonicalize_only_keeps_incidental_whitespace_and_exact_line_endings() {
    // The authored indent, the double blanks around `==`, the comment gap and
    // the CRLF all survive; the trailing runs do not, because whitespace at end
    // of line is invisible rather than a formatting choice. Note that the
    // absent final newline is still not supplied: that half of step 20 is
    // layout, which this mode does not run.
    let source = b"\tENDDO   \r\nx  .EQ.  y ! gap\nENDIF\t";
    let output = format_source(source, &canonicalize_config()).unwrap().bytes;

    assert_eq!(output, b"\tend do\r\nx  ==  y ! gap\nend if");
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

#[test]
fn every_mode_removes_line_terminal_whitespace() {
    // Interior whitespace can be deliberate alignment, so the modes that do not
    // own presentation preserve it. Whitespace at end of line is invisible in
    // every mode and is never a formatting choice, so no mode keeps it.
    let source = b"program p   \n   x  =  1\t\nend program p  \n";
    for mode in [
        FormatMode::IndentOnly,
        FormatMode::NormalizeOnly,
        FormatMode::CanonicalizeOnly,
        FormatMode::Full,
    ] {
        let config = FormatConfig {
            mode,
            ..FormatConfig::default()
        };
        let output = format_source(source, &config).unwrap().bytes;
        let text = String::from_utf8(output).unwrap();
        for line in text.lines() {
            assert_eq!(
                line.trim_end(),
                line,
                "{mode:?} kept trailing space: {line:?}"
            );
        }
        // Indent-only runs no normalization at all, and canonicalization runs
        // it without presentation whitespace; both owe the authored gaps.
        if !mode.normalizes() || !mode.normalizes_whitespace() {
            assert!(
                text.contains("x  =  1"),
                "{mode:?} lost interior spacing: {text}"
            );
        }
    }
}

#[test]
fn a_hollerith_payload_is_not_trailing_whitespace() {
    // `3Hab ` promises three characters and the third is a blank. Trimming it
    // truncates the constant into one that no longer has three, so the payload
    // outranks the trailing-whitespace rule in every mode.
    let source = b"      x = 3Hab \n      y = 5Ha  b \n      end\n";
    for mode in [
        FormatMode::IndentOnly,
        FormatMode::NormalizeOnly,
        FormatMode::CanonicalizeOnly,
        FormatMode::Full,
    ] {
        let config = FormatConfig {
            mode,
            ..FormatConfig::default()
        };
        let output = format_source(source, &config).unwrap().bytes;
        let text = String::from_utf8(output).unwrap();
        assert!(
            text.contains("x = 3Hab \n"),
            "{mode:?} truncated 3Hab: {text:?}"
        );
        assert!(
            text.contains("y = 5Ha  b \n"),
            "{mode:?} truncated 5Ha  b: {text:?}"
        );
    }
}

#[test]
fn a_literal_keeps_its_blanks_while_the_line_around_it_is_trimmed() {
    let source = b"program p\ny = 'tail spaces   '   \n!$ z = 3Hab \nend program p\n";
    let config = FormatConfig::default();
    let output = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("y = 'tail spaces   '\n"), "{text:?}");
    assert!(text.contains("!$ z = 3Hab \n"), "{text:?}");
}
