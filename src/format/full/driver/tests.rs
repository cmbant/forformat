use super::format_with_context;
use crate::{
    analysis::{analyze_project, ProjectContext},
    config::{FormatConfig, FormatMode},
    format_source,
    source::{LogicalGroup, SourceBuffer},
    transform::document::Document,
};
use std::path::Path;

fn full(config_setup: impl FnOnce(&mut FormatConfig), source: &[u8]) -> Vec<u8> {
    let mut config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    config_setup(&mut config);
    format_with_context(source, &ProjectContext::empty(), &config)
        .unwrap()
        .bytes
}

fn profile_full(source: &[u8]) -> Vec<u8> {
    full(
        |config| {
            config.indent = 4;
            config.start_indent = 4;
            config.contains_indent = 0;
            config.openmp = false;
            config.contains_restart = true;
            config.indent_continuation = true;
            config.continuation_indent = 4;
            config.indent_ampersand = true;
            config.construct_indents.set_all(4);
            config.construct_indents.module = 0;
            config.construct_indents.procedure = 0;
            config.construct_indents.interface = 0;
        },
        source,
    )
}

#[test]
fn fixed_point_progress_distinguishes_stability_from_cycles() {
    let history = [1, 2];
    assert_eq!(
        super::fixed_point_progress(&history, &3),
        super::FixedPointProgress::New
    );
    assert_eq!(
        super::fixed_point_progress(&history, &2),
        super::FixedPointProgress::Stable
    );
    assert_eq!(
        super::fixed_point_progress(&history, &1),
        super::FixedPointProgress::Cycle
    );
}

#[test]
fn conditional_sentinel_body_follows_declared_case_with_or_without_project_tables() {
    let source = b"module t\ninteger :: MyVar\ncontains\nsubroutine s()\n!$ myvar = 1\nmyvar = 2\nend subroutine s\nend module t\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let expected = b"module t\n   integer :: MyVar\n\ncontains\n\n   subroutine s\n!$    MyVar = 1\n      MyVar = 2\n   end subroutine s\n\nend module t\n";
    let empty = format_with_context(source, &ProjectContext::empty(), &config)
        .unwrap()
        .bytes;
    let project = analyze_project([(Path::new("sentinel.f90"), source.as_slice())]).unwrap();
    let single_file = format_with_context(source, &project, &config)
        .unwrap()
        .bytes;
    assert_eq!(empty, expected);
    assert_eq!(single_file, expected);
}

#[test]
fn reflow_reuses_component_case_from_the_unjoined_statement() {
    let source = b"module m\ntype :: T\ninteger :: FIRST\ninteger :: SECOND\nend type T\ncontains\nsubroutine s(this)\ntype(T) :: this\nthis%first = this%second + 12345678901234567890 + 12345678901234567890 + 12345678901234567890\nend subroutine s\nend module m\n";
    let one_line = full(|config| config.wrap.line_length = 120, source);
    let continued = full(|config| config.wrap.line_length = 70, source);
    let one_line_text = String::from_utf8(one_line).unwrap();
    let continued_text = String::from_utf8(continued).unwrap();
    assert!(one_line_text.contains("this%FIRST = this%SECOND"));
    assert!(continued_text.contains("this%FIRST = this%SECOND"));
    assert!(continued_text.contains("&\n"));
}

#[test]
fn a_nested_type_spec_colon_is_a_stable_wrap_point() {
    let source = b"subroutine s\nif (a) then\nif (b) then\nif (c) then\nallocate(TMetropolisSampler::this%SamplingAlgorithm)\nend if\nend if\nend if\nend subroutine s\n";
    let setup = |config: &mut FormatConfig| {
        config.indent = 8;
        config.construct_indents.set_all(8);
        config.wrap.line_length = 80;
    };
    let once = full(setup, source);
    let twice = full(setup, &once);
    assert_eq!(once, twice);
    assert!(String::from_utf8_lossy(&once).contains("TMetropolisSampler :: &"));
}

#[test]
fn detached_comment_uses_the_single_line_layout_indent() {
    let source = br#"module m
implicit none
contains
subroutine s(a)
real :: a
call some_procedure_with_a_long_name(argument_number_1, argument_number_2, argument_number_3, argument_number_4, argument_number_5, argument_number_6, argument_number_7, argument_number_8, argument_number_9, argument_number_10, argument_number_11) ! short note
end subroutine s
end module m
"#;
    let once = profile_full(source);
    let twice = profile_full(&once);
    assert_eq!(once, twice);
    assert!(String::from_utf8_lossy(&once).contains("    ! short note\n"));
}

#[test]
fn a_fitting_joined_group_is_emitted_as_one_statement() {
    let source = br#"module m
implicit none
contains
subroutine s(a)
real :: a
a = 1 ! this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long
call f(a, &
a) ! this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long this trailing comment is deliberately long
b = 2 ! short
end subroutine s
end module m
"#;
    let once = profile_full(source);
    let twice = profile_full(&once);
    assert_eq!(once, twice);
    let output = String::from_utf8_lossy(&once);
    assert!(output.contains("    ! this trailing comment"));
    assert!(output.contains("    call f(a, a)\n"));
    assert!(!output.contains("call f(a, &\n"));
}

#[test]
fn only_the_final_line_comment_is_stripped() {
    let document = Document::from_bytes(b"  code ! keep\n  code ! strip\n");
    let group = LogicalGroup {
        lines: 0..2,
        statements: Vec::new(),
        pieces: Vec::new(),
    };
    let mut once = Vec::new();
    let document_bytes = document.to_lf_bytes();
    let buffer = SourceBuffer::new(&document_bytes).unwrap();
    super::copy_group_without_final_comment(&document, &buffer, &group, &mut once);
    let transformed = Document::from_bytes(b"  code ! keep\n  code\n");
    let mut twice = Vec::new();
    let transformed_bytes = transformed.to_lf_bytes();
    let transformed_buffer = SourceBuffer::new(&transformed_bytes).unwrap();
    super::copy_group_without_final_comment(&transformed, &transformed_buffer, &group, &mut twice);
    assert_eq!(once, [b"  code ! keep".to_vec(), b"  code".to_vec()]);
    assert_eq!(twice, once);
}

#[test]
fn full_output_is_a_findent_fixed_point() {
    let source = b"PROGRAM Main\nIF (X > 1) THEN\nCALL DoThing(Value)\nEND IF\nEND PROGRAM Main\n";
    let once = full(|_| {}, source);
    let indent_only = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert_eq!(
        String::from_utf8_lossy(&indent_only),
        String::from_utf8_lossy(&once)
    );
}

#[test]
fn empty_openmp_sentinel_is_trimmed_and_remains_an_indent_fixed_point() {
    let once = full(|_| {}, b"\n!$ \n");
    assert_eq!(once, b"\n!$\n");
    let indent_only = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert_eq!(indent_only, once);
}

#[test]
fn full_formatting_reaches_its_fixed_point_in_one_pass() {
    for source in [
        b"PROGRAM p\nX = 1\nEND PROGRAM p\n".as_slice(),
        b"module m\ncontains\nSUBROUTINE s()\nEND SUBROUTINE s\nend module m\n".as_slice(),
        b"".as_slice(),
        b"! just a comment\n".as_slice(),
    ] {
        let once = full(|_| {}, source);
        let twice = full(|_| {}, &once);
        assert_eq!(
            String::from_utf8_lossy(&twice),
            String::from_utf8_lossy(&once),
            "not idempotent for {source:?}"
        );
    }
}

#[test]
fn the_dominant_line_ending_is_restored() {
    let crlf = full(|_| {}, b"PROGRAM p\r\nX = 1\r\nEND PROGRAM p\r\n");
    assert!(crlf.windows(2).any(|pair| pair == b"\r\n"));
    assert_eq!(
        String::from_utf8_lossy(&crlf),
        "program p\r\n   X = 1\r\nend program p\r\n"
    );
}

#[test]
fn full_mode_normalizes_the_final_newline() {
    assert_eq!(full(|_| {}, b""), b"");
    assert_eq!(full(|_| {}, b"X = 1"), b"X = 1\n");
    assert_eq!(full(|_| {}, b"X = 1\n\n\n"), b"X = 1\n");
    assert_eq!(full(|_| {}, b"X = 1\r\n\r\n"), b"X = 1\r\n");
}

#[test]
fn a_long_statement_is_wrapped_within_its_budget() {
    let source = b"program p\ntotal = alpha + beta + gamma + delta + epsilon + zeta + eta + theta\nend program p\n";
    let wrapped = full(|config| config.wrap.line_length = 40, source);
    let text = String::from_utf8_lossy(&wrapped).into_owned();
    for line in text.lines() {
        assert!(line.len() <= 40, "overlong line {line:?} in\n{text}");
    }
    assert!(text.contains(" &\n"), "no continuation produced:\n{text}");
    let again = format_source(&wrapped, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert_eq!(String::from_utf8_lossy(&again), text);
}

#[test]
fn statements_settle_on_the_first_run_when_later_passes_widen_them() {
    let cases: [&[u8]; 4] = [
        b"module m\ncontains\nsubroutine s\n    if (Feedback >1 ) write(*,*) &\n     ' Parameter '//trim(BaseParams%UsedParamNameOrNumber(i))//' is weakly constrained, neglect correlations'\nend subroutine s\nend module m\n",
        b"module m\ncontains\nsubroutine s\ndo i = 1, n\ndo j = 1, n\n!$OMP PARALLEL DO DEFAULT(SHARED), SCHEDULE(STATIC), PRIVATE(zpeak, sigma_z, zpeakstart, zpeakend, nu_i, Win)\ndo k = 1, n\nx = 1\nend do\nend do\nend do\nend subroutine s\nend module m\n",
        b"module m\ncontains\nsubroutine s\nreal (dl):: dif_old,dif,max,min,dlm,binz,m_min,m_max,mp,yp,zp,thp,xk1,xk2,xk3,yk1,yk2,yk3,fact,qmin,qmax,dlogy\nend subroutine s\nend module m\n",
        b"module m\ncontains\nsubroutine s\nif (fb == zero) then\nxzero = b\nelseif (fa*(fb/abs(fb))<zero) then  ! check that f(ax) and f(bx) have different signs\nc = a\nend if\nend subroutine s\nend module m\n",
    ];
    for source in cases {
        for length in [80usize, 100, 120] {
            let once = full(|config| config.wrap.line_length = length, source);
            let twice = full(|config| config.wrap.line_length = length, &once);
            assert_eq!(
                String::from_utf8_lossy(&once),
                String::from_utf8_lossy(&twice),
                "not a fixed point at {length} columns"
            );
        }
    }
}

#[test]
fn project_case_does_not_make_wrapped_intrinsics_non_idempotent() {
    let target = b"module target\nimplicit none\ncontains\nsubroutine s(x, i, j)\nreal :: x\nreal(ReallyLongKindName) :: LongJMat(size(x%element(i, j)%x), size(x%element(i, j)%x))\nend subroutine s\nend module target\n";
    let project_source = b"module project_names\nreal :: Size\nend module project_names\n";
    let project = analyze_project([
        (Path::new("target.f90"), target.as_slice()),
        (Path::new("project_names.f90"), project_source.as_slice()),
    ])
    .unwrap();
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: crate::config::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        ..FormatConfig::default()
    };
    let once = format_with_context(target, &project, &config)
        .unwrap()
        .bytes;
    let twice = format_with_context(&once, &project, &config).unwrap().bytes;
    assert_eq!(twice, once);
    let output = String::from_utf8(once).unwrap();
    assert!(
        output.contains("LongJMat(size(x%element(i, j)%x), &\n"),
        "{output}"
    );
    assert!(output.contains("size(x%element(i, j)%x))"), "{output}");
}

#[test]
fn a_declined_wrap_keeps_the_whole_statement() {
    let mut source = b"module m\ncontains\nsubroutine s\ncall f(a, '".to_vec();
    source.extend(std::iter::repeat_n(b'x', 150));
    source.extend_from_slice(b"', &\n    b)\nend subroutine s\nend module m\n");
    let once = full(|_| {}, &source);
    let text = String::from_utf8_lossy(&once).into_owned();
    assert!(text.contains("b)\n"), "continuation line dropped:\n{text}");
    let twice = full(|_| {}, &once);
    assert_eq!(text, String::from_utf8_lossy(&twice));
}

#[test]
fn a_continued_format_statement_keeps_its_slash_before_the_paren() {
    let source = b"module m\ncontains\nsubroutine s\n9060 format ('    NXD =', i5, ',  NYD =', i5, ',  NXI =', i5, &\n    ',  NYI =', i5 /)\nend subroutine s\nend module m\n";
    let once = full(|_| {}, source);
    let text = String::from_utf8_lossy(&once).into_owned();
    assert!(
        text.contains("i5 /)"),
        "format descriptor rewritten:\n{text}"
    );
    assert!(
        !text.contains("i5]"),
        "format descriptor rewritten:\n{text}"
    );
}

#[test]
fn normalize_only_mode_leaves_every_column_untouched() {
    let source = b"program p\n        X = 1\nend program p\n";
    let normalized = full(|config| config.mode = FormatMode::NormalizeOnly, source);
    assert_eq!(
        String::from_utf8_lossy(&normalized),
        String::from_utf8_lossy(source)
    );
}

#[test]
fn generated_wrapping_stress_cases_are_fixed_points_and_fit_safe_breaks() {
    let sources = [
        br#"program p
             real :: values(1), weights(2), alpha, beta, gamma, delta
             call compute(alpha, beta, gamma, delta, nested(first_value, second_value, third_value), named=value)
             result_value = alpha + beta + gamma + delta + epsilon + zeta + eta + theta + iota + kappa
             end program p
             "# as &[u8],
        br#"program p
             real :: values(1), weights(2), alpha, beta, &
             & gamma, delta
             call compute(alpha, beta, gamma, delta, nested(first_value, second_value, &
             & third_value), named=value)
             result_value = alpha + beta + gamma + delta + epsilon + zeta + eta + theta + iota + kappa
             end program p
             "# as &[u8],
    ];
    for source in sources {
        for line_length in [60, 80, 100, 120] {
            for align in [false, true] {
                for continuation in [0, 3, 9] {
                    let config = FormatConfig {
                        mode: FormatMode::Full,
                        wrap: crate::config::WrapConfig {
                            enabled: true,
                            line_length,
                        },
                        align_paren: align,
                        align_paren_value: usize::from(align),
                        continuation_indent: continuation,
                        ..FormatConfig::default()
                    };
                    let once = format_with_context(source, &ProjectContext::empty(), &config)
                        .unwrap()
                        .bytes;
                    let twice = format_with_context(&once, &ProjectContext::empty(), &config)
                        .unwrap()
                        .bytes;
                    assert_eq!(
                        once, twice,
                        "not idempotent at {line_length}/{align}/{continuation}"
                    );
                    let mut indent_only = config.clone();
                    indent_only.mode = FormatMode::IndentOnly;
                    let indented = crate::format_source(&once, &indent_only).unwrap().bytes;
                    assert_eq!(
                        once, indented,
                        "I2 failed at {line_length}/{align}/{continuation}"
                    );
                    for line in once.split(|byte| *byte == b'\n') {
                        if line.len() <= line_length || line.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        let text = line.trim_ascii_start();
                        assert!(
                            text.starts_with(b"!") || text.starts_with(b"#"),
                            "generated code line exceeded {line_length}: {:?}",
                            String::from_utf8_lossy(line)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn conditional_declaration_separator_is_visible_to_wrapper_measurement() {
    assert_eq!(
        crate::transform::passes::layout_post::declaration_separator_info(b"!$ real    ::  x"),
        Some((11, 4, 2))
    );
}

#[test]
fn openmp_wrapping_repeats_reserved_sentinels_and_keeps_macro_case() {
    for (authored, expected) in [
        ("!$OMP", b"!$OMP".as_slice()),
        ("!$OMPX", b"!$OMPX".as_slice()),
        ("!$omp", b"!$OMP".as_slice()),
    ] {
        let source = format!(
            "{authored} PARALLEL DO DEFAULT(SHARED), private(worker), SCHEDULE(STATIC), REDUCTION(+:total)\n"
        );
        let mut project = ProjectContext::empty();
        project.define(&[crate::config::MacroDefine {
            name: "private".into(),
            value: None,
        }]);
        let config = FormatConfig {
            mode: FormatMode::Full,
            wrap: crate::config::WrapConfig {
                enabled: true,
                line_length: 42,
            },
            ..FormatConfig::default()
        };
        let output = format_with_context(source.as_bytes(), &project, &config)
            .unwrap()
            .bytes;
        for line in output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            assert!(
                line.starts_with(expected),
                "invalid {authored} sentinel: {line:?}"
            );
            assert!(line.len() <= 42, "overlong OpenMP line: {line:?}");
        }
        assert!(output
            .windows(b"PRIVATE".len())
            .all(|window| window != b"PRIVATE"));
        assert!(output
            .windows(b"private".len())
            .any(|window| window == b"private"));
        let again = format_with_context(&output, &project, &config)
            .unwrap()
            .bytes;
        assert_eq!(again, output, "wrapped {authored} is not a fixed point");
    }
}

#[test]
fn wrapped_openmp_sentinels_follow_the_openmp_case_policy() {
    for (case, openmp_case, expected) in [
        (crate::config::KeywordCase::Lower, true, b"!$OMP".as_slice()),
        (crate::config::KeywordCase::Upper, true, b"!$OMP".as_slice()),
        (
            crate::config::KeywordCase::Preserve,
            true,
            b"!$OMP".as_slice(),
        ),
        (
            crate::config::KeywordCase::Lower,
            false,
            b"!$omp".as_slice(),
        ),
        (
            crate::config::KeywordCase::Upper,
            false,
            b"!$OMP".as_slice(),
        ),
        (
            crate::config::KeywordCase::Preserve,
            false,
            b"!$OmP".as_slice(),
        ),
    ] {
        let source = b"!$OmP PARALLEL DO DEFAULT(SHARED), SCHEDULE(STATIC), REDUCTION(+:total)\n";
        let style = crate::config::StyleConfig {
            keyword_case: case,
            openmp_case,
            ..crate::config::StyleConfig::default()
        };
        let config = FormatConfig {
            mode: FormatMode::Full,
            style,
            wrap: crate::config::WrapConfig {
                enabled: true,
                line_length: 42,
            },
            ..FormatConfig::default()
        };
        let project = ProjectContext::empty();
        let output = format_with_context(source, &project, &config)
            .unwrap()
            .bytes;
        for line in output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            assert!(
                line.starts_with(expected),
                "{case:?}/openmp_case={openmp_case}: {line:?}"
            );
        }
        let again = format_with_context(&output, &project, &config)
            .unwrap()
            .bytes;
        assert_eq!(
            again, output,
            "{case:?}/openmp_case={openmp_case} is not a fixed point"
        );
    }
}
