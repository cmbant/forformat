//! The OpenMP case policy and the boundary it draws.
//!
//! `!$OMP` is a reserved directive sentinel and its directive words are OpenMP
//! keywords, so `--openmp-case` governs both. `!$ ` is not a directive at all:
//! it is ordinary Fortran that only the OpenMP compiler sees, so its body stays
//! with `--keyword-case` like any other statement.

use forformat::{format_source, FormatConfig, KeywordCase, StyleConfig};

fn config(style: StyleConfig) -> FormatConfig {
    FormatConfig {
        style,
        ..FormatConfig::default()
    }
}

/// The body of every conditional-compilation line, without the `!$` sentinel or
/// the indentation the layout engine gives it.
fn conditional_bodies(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.trim_start().strip_prefix("!$"))
        .filter(|body| body.starts_with(' ') || body.starts_with('\t'))
        .map(str::trim)
        .collect()
}

fn format(source: &[u8], style: StyleConfig) -> String {
    let config = config(style);
    let output = format_source(source, &config).unwrap().bytes;
    // Nothing here is worth pinning if it is not also a fixed point.
    assert_eq!(
        format_source(&output, &config).unwrap().bytes,
        output,
        "not a fixed point"
    );
    String::from_utf8(output).unwrap()
}

const SOURCE: &[u8] = b"program p
integer :: i, x
!$omp Parallel Do Private(i)
do i = 1, 2
!$ x = Omp_Get_Thread_Num()
!$ If (x .gt. 1) Then
!$ End If
x = x + i
end do
!$OmP end parallel do
end program p
";

#[test]
fn directives_are_uppercase_by_default_and_conditional_code_is_not() {
    let text = format(SOURCE, StyleConfig::default());
    assert!(text.contains("!$OMP PARALLEL DO PRIVATE(i)"), "{text}");
    assert!(text.contains("!$OMP END PARALLEL DO"), "{text}");
    // `!$ ` bodies are ordinary statements: their keywords follow
    // `keyword_case`, which defaults to lower, while `Omp_Get_Thread_Num` is a
    // name rather than a directive word and keeps its authored spelling.
    assert_eq!(
        conditional_bodies(&text),
        ["x = Omp_Get_Thread_Num()", "if (x > 1) then", "end if"],
        "{text}"
    );
}

#[test]
fn keyword_case_does_not_reach_a_directive_while_the_openmp_policy_is_on() {
    for case in [
        KeywordCase::Lower,
        KeywordCase::Upper,
        KeywordCase::Preserve,
    ] {
        let text = format(
            SOURCE,
            StyleConfig {
                keyword_case: case,
                ..StyleConfig::default()
            },
        );
        assert!(
            text.contains("!$OMP PARALLEL DO PRIVATE(i)"),
            "{case:?}\n{text}"
        );
        assert!(text.contains("!$OMP END PARALLEL DO"), "{case:?}\n{text}");
    }
}

#[test]
fn turning_the_openmp_policy_off_hands_directives_back_to_keyword_case() {
    for (case, parallel, end) in [
        (
            KeywordCase::Lower,
            "!$omp parallel do private(i)",
            "!$omp end parallel do",
        ),
        (
            KeywordCase::Upper,
            "!$OMP PARALLEL DO PRIVATE(i)",
            "!$OMP END PARALLEL DO",
        ),
        // `preserve` preserves only once the OpenMP policy stops overriding it,
        // which is the whole point of the switch.
        (
            KeywordCase::Preserve,
            "!$omp Parallel Do Private(i)",
            "!$OmP end parallel do",
        ),
    ] {
        let text = format(
            SOURCE,
            StyleConfig {
                keyword_case: case,
                openmp_case: false,
                ..StyleConfig::default()
            },
        );
        assert!(text.contains(parallel), "{case:?}\n{text}");
        assert!(text.contains(end), "{case:?}\n{text}");
    }
}

#[test]
fn conditional_code_follows_keyword_case_under_either_openmp_policy() {
    for openmp_case in [true, false] {
        let upper = format(
            SOURCE,
            StyleConfig {
                keyword_case: KeywordCase::Upper,
                openmp_case,
                ..StyleConfig::default()
            },
        );
        assert_eq!(
            conditional_bodies(&upper),
            ["x = Omp_Get_Thread_Num()", "IF (x > 1) THEN", "END IF"],
            "openmp_case={openmp_case}\n{upper}"
        );

        let preserved = format(
            SOURCE,
            StyleConfig {
                keyword_case: KeywordCase::Preserve,
                openmp_case,
                ..StyleConfig::default()
            },
        );
        assert_eq!(
            conditional_bodies(&preserved),
            ["x = Omp_Get_Thread_Num()", "If (x > 1) Then", "End If"],
            "openmp_case={openmp_case}\n{preserved}"
        );
    }
}

#[test]
fn an_unreserved_sentinel_is_not_an_openmp_directive_under_either_policy() {
    // `!$acc` is a comment to this formatter, not a reserved OpenMP sentinel,
    // so neither case policy may rewrite it.
    let source = b"program p\n!$acc enter data create(rho)\nend program p\n";
    for openmp_case in [true, false] {
        let text = format(
            source,
            StyleConfig {
                openmp_case,
                ..StyleConfig::default()
            },
        );
        assert!(text.contains("!$acc enter data create(rho)"), "{text}");
    }
}
