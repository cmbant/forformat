//! A pass must not move bytes to the start of a physical line where syntax
//! that was inert becomes active.
//!
//! `#`, `??`, `!$`, `!$omp` and a continuation `&` are all anchored to the start
//! of a line and mean nothing in the middle of one. Two full-mode passes move
//! bytes leftwards — dropping a continuation marker, lifting a trailing inline
//! comment above its statement — and each used to be able to manufacture one of
//! these openings out of ordinary content. Every case below is a byte sequence
//! that changed meaning, and half of them were fixed points, so idempotence
//! alone could never have found them.
//!
//! See `source::syntax::LineStartSyntax`.

use forformat::{format_source, FormatConfig, FormatMode};

fn full(source: &[u8]) -> Vec<u8> {
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    format_source(source, &config).unwrap().bytes
}

/// Format twice and require both a fixed point and that `forbidden` never
/// appears at the start of any line of the result.
fn stable_without_promoted(source: &[u8], forbidden: &[&[u8]]) -> Vec<u8> {
    let once = full(source);
    let twice = full(&once);
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "not a fixed point"
    );
    for line in once.split(|byte| *byte == b'\n') {
        let trimmed = line.trim_ascii_start();
        for opening in forbidden {
            assert!(
                !trimmed.starts_with(opening),
                "{:?} was promoted to the start of a line in {:?}",
                String::from_utf8_lossy(opening),
                String::from_utf8_lossy(&once),
            );
        }
    }
    once
}

/// `&# 2` is continuation content: the statement is `x = 1 + # 2`. Consuming
/// the marker would leave `# 2` in column zero, where the same classifier reads
/// a preprocessor directive — and a directive is stepped over rather than
/// joined, silently truncating the statement to `x = 1 +`.
#[test]
fn a_continuation_body_is_never_promoted_to_a_preprocessor_directive() {
    stable_without_promoted(b"x = 1 + &\n&# 2\n", &[b"#"]);
    stable_without_promoted(b"x = 1 + &\n&??cpp\n", &[b"??"]);
}

/// Each pass consumed one leading `&`, so `& & 4` needed three passes to reach
/// a fixed point (`& & 4` -> `& 4` -> `4`). Whatever a doubled marker means, a
/// pass that strips one and leaves the next in a position to be stripped again
/// is the idempotence break.
#[test]
fn a_doubled_continuation_marker_is_not_stripped_one_per_pass() {
    let once = full(b"a = &\n& & 4\n");
    assert_eq!(full(&once), once);
    let once = full(b"&\n&&\n");
    assert_eq!(full(&once), once);
}

/// The OpenMP directive stream consumes its own repeated marker, and had the
/// same one-per-run break: `!$omp &&do` needed two runs to settle. The sentinel
/// keeps the line start here, so only the doubled marker is at stake — a single
/// one is still consumed.
#[test]
fn a_doubled_openmp_continuation_marker_is_not_stripped_one_per_pass() {
    let once = full(b"!$omp parallel &\n!$omp &&do\n");
    assert_eq!(full(&once), once);

    let single = full(b"!$omp parallel &\n!$omp &do\n");
    assert!(
        single.windows(9).any(|window| window == b"!$OMP DO\n"),
        "expected the single marker consumed in {:?}",
        String::from_utf8_lossy(&single),
    );
    assert_eq!(full(&single), single);
}

/// Dropping the marker from a `&`-only continuation line left an empty line,
/// turning a code line into a blank one and dropping it from the output.
#[test]
fn a_bare_continuation_line_keeps_its_marker() {
    let once = full(b"x = 1 + &\n&\n");
    assert_eq!(once.iter().filter(|byte| **byte == b'\n').count(), 2);
    assert_eq!(full(&once), once);
}

/// The sentinels this crate parses are not the only ones a compiler acts on.
/// `&!$acc loop` and `&!DEC$ ATTRIBUTES ...` are continuation content, and
/// dropping the marker leaves an active OpenACC or Intel directive at column
/// zero — the second of which names the entity it sits on, so promoting it
/// retargets it rather than merely moving it.
#[test]
fn a_continuation_body_is_never_promoted_to_an_unmodelled_directive() {
    for body in [
        b"!$acc loop".as_slice(),
        b"!DEC$ ATTRIBUTES ALIGN: 64 :: b",
        b"!dir$ vector always",
        b"!GCC$ unroll 4",
    ] {
        let mut source = b"x = 1 + &\n&".to_vec();
        source.extend_from_slice(body);
        source.extend_from_slice(b"\n2\n");
        let once = stable_without_promoted(&source, &[b"!$acc", b"!DEC$", b"!dir$", b"!GCC$"]);
        assert!(
            once.windows(body.len() + 1)
                .any(|window| window[0] == b'&' && &window[1..] == body),
            "expected the marker kept in {:?}",
            String::from_utf8_lossy(&once),
        );
    }
}

/// The wrapper picks where a statement breaks, so it can manufacture one of
/// these openings too. A stray `&` inside a statement is data; put first on the
/// continuation the wrapper just created, it is the optional leading marker,
/// and the next pass eats it. `program bf=&,(zzz...)` wrapped to
/// `program bf = &` / `&, (zzz...` and the run after that wrote `   , (zzz...`,
/// deleting a byte the author wrote.
///
/// Found by `FUZZ_TIME=120 ./tools/check_fuzz_regression.sh` in the
/// `properties` target and reduced from the 134-byte artifact.
#[test]
fn a_wrapped_line_never_breaks_onto_a_continuation_marker() {
    let mut source = b"program bf=&,(".to_vec();
    source.extend(std::iter::repeat_n(b'z', 101));
    source.extend_from_slice(b"a&c\n");
    // No line may *open* on the marker; the wrapper's own marker closes a line
    // and is not at stake.
    let once = stable_without_promoted(&source, &[b"&"]);
    // Both authored `&` survive, alongside the one marker the wrap added.
    assert_eq!(
        once.iter().filter(|byte| **byte == b'&').count(),
        3,
        "expected both authored & plus one wrap marker in {:?}",
        String::from_utf8_lossy(&once),
    );
}

/// A continuation body that is only a comment is genuinely safe to promote:
/// `&! c` is already a continuation line carrying a trailing comment, so
/// emitting `! c` changes neither the statement nor where the group ends.
#[test]
fn a_comment_continuation_body_is_still_promoted() {
    let once = full(b"x = 1 + &\n&! c\ny = 2\n");
    assert!(
        once.windows(4).any(|window| window == b"\n! c"),
        "expected the comment at column zero in {:?}",
        String::from_utf8_lossy(&once),
    );
    assert_eq!(full(&once), once);
}

/// A long statement's trailing inline comment is detached above it so the
/// wrapper can break the code. `!$omp parallel` is a comment where the author
/// wrote it and an OpenMP directive once it reaches column zero — and the run
/// after that applied the directive case rule, spelling it `!$OMP`.
#[test]
fn a_trailing_sentinel_comment_is_not_detached_above_its_statement() {
    for comment in [
        b"!$omp parallel".as_slice(),
        b"!$ y = 2",
        b"!$acc loop",
        b"!$&x",
        b"!$",
    ] {
        let mut source =
            b"aaaaaaaaaaaaaaaa = 111111111 + 222222222 + 333333333 + 444444444 + 555555555 \
              + 666666666 + 777777777 + 888888888 + 999999999 + 101010 "
                .to_vec();
        source.extend_from_slice(comment);
        source.push(b'\n');
        stable_without_promoted(&source, &[b"!$"]);
    }
}

/// The guard is a refusal to *promote*, not a refusal to detach: an ordinary
/// trailing comment still moves above its statement so the code can wrap.
#[test]
fn an_ordinary_trailing_comment_is_still_detached() {
    let source = b"aaaaaaaaaaaaaaaa = 111111111 + 222222222 + 333333333 + 444444444 + 555555555 \
                   + 666666666 + 777777777 + 888888888 + 999999999 + 101010 ! ordinary\n";
    let once = full(source);
    assert!(
        once.starts_with(b"! ordinary\n"),
        "expected the comment detached above the statement in {:?}",
        String::from_utf8_lossy(&once),
    );
    assert_eq!(full(&once), once);
}
