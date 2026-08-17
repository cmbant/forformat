use forformat::{
    format_source, format_to, format_to_owned,
    source::{
        regions::{map_code, regions, LexState},
        LogicalGroup, RegionKind, SourceBuffer,
    },
    FormatConfig, FormatMode, KeywordCase, StyleConfig,
};
use std::{fs, path::PathBuf};

fn indent_only_config() -> FormatConfig {
    FormatConfig {
        mode: FormatMode::IndentOnly,
        ..FormatConfig::default()
    }
}

fn style_config(style: StyleConfig) -> FormatConfig {
    FormatConfig {
        mode: FormatMode::Full,
        apply_indent: false,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        style,
        ..FormatConfig::default()
    }
}

#[test]
fn explicit_default_style_is_the_default_output() {
    let source = b"EnDiF\nx=a*b/c**2+d-e//suffix\ncall f(kind=8,mask=.not.ready)\nx=(/a,b/)\n";
    let ordinary = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let explicit = format_source(source, &style_config(StyleConfig::default()))
        .unwrap()
        .bytes;
    assert_eq!(ordinary, explicit);
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn style_settings_do_not_enter_indent_only() {
    let mut style = StyleConfig::default();
    style.keyword_case = KeywordCase::Upper;
    style.relational_symbols = false;
    style.compact_multiplicative = false;
    style.delimiter_spacing = false;
    style.comment_spacing = false;
    style.continuation_markers = false;
    let config = FormatConfig {
        mode: FormatMode::IndentOnly,
        style,
        ..FormatConfig::default()
    };
    let source = b"PROGRAM P\nIF (X) THEN\nx=a*b\nEND IF\nEND PROGRAM P\n";
    assert_eq!(
        format_source(source, &config).unwrap().bytes,
        format_source(
            source,
            &FormatConfig {
                mode: FormatMode::IndentOnly,
                ..FormatConfig::default()
            }
        )
        .unwrap()
        .bytes
    );
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn style_switches_have_exact_spelling_and_fixed_points() {
    let source = b"EnDiF\nGo   To 10\nx = A*B/C**2 + D - E // suffix\ny = A*-B\nz = A**-2\ncall f(kind=8, mask=.NOT. READY)\np => TARGET\nx = A .EQ. B\na = .TRUE.\nr = 1.0e-3 + 2.0d+4\ncharacter*8 name\nwrite(*, *)\n";

    let mut style = StyleConfig::default();
    style.keyword_case = KeywordCase::Preserve;
    let preserve_case = format_source(source, &style_config(style)).unwrap().bytes;
    assert!(preserve_case
        .windows(b"EnD iF".len())
        .any(|window| window == b"EnD iF"));
    assert!(preserve_case
        .windows(b"GoTo 10".len())
        .any(|window| window == b"GoTo 10"));

    style.keyword_case = KeywordCase::Upper;
    style.relational_symbols = false;
    style.compact_multiplicative = false;
    let upper = format_source(source, &style_config(style)).unwrap().bytes;
    let upper = String::from_utf8(upper).unwrap();
    assert!(upper.contains("END IF"));
    assert!(upper.contains("A * B / C ** 2 + D - E // suffix"));
    assert!(upper.contains("A .EQ. B"));
    assert!(upper.contains("1.0E-3 + 2.0D+4"));
    assert!(upper.contains("CHARACTER*8 name"));
    assert!(upper.contains("WRITE(*, *)"));

    let mut preserve = StyleConfig::default();
    preserve.array_brackets = false;
    preserve.join_goto = false;
    preserve.split_compound_keywords = false;
    preserve.strip_empty_args = false;
    preserve.remove_redundant_parens = false;
    preserve.remove_terminal_return = false;
    preserve.program_unit_spacing = false;
    preserve.max_blank_lines = None;
    preserve.delimiter_spacing = false;
    preserve.comment_spacing = false;
    preserve.continuation_markers = false;
    let source = b"program p\nsubroutine s()\nx=((a))\nreturn\nend subroutine s\nend program p\n";
    let once = format_source(source, &style_config(preserve))
        .unwrap()
        .bytes;
    let twice = format_source(&once, &style_config(preserve)).unwrap().bytes;
    assert_eq!(once, twice);
    let text = String::from_utf8(once).unwrap();
    assert!(text.contains("subroutine s()"));
    assert!(text.contains("x = ((a))"));
    assert!(text.contains("return"));
}

#[test]
fn compact_select_spellings_indent_like_the_two_word_form() {
    // findent's own recognizer accepts `selectcase`/`selecttype`/`selectrank`
    // as SELECT openers (verified against findent 4.3.7), and the compact
    // `end...` spellings are already split by `--split-compound-keywords`;
    // the same must hold for these so the indent-only engine's frame
    // nesting does not diverge from the oracle for every statement inside
    // the construct. Reduced from OpenFAST `modules/nwtc-library/src/NWTC_IO.f90`.
    let compact = format_source(
        b"subroutine s(x)\ninteger :: x\nselectcase(x)\ncase(1)\nx=1\nendselect\nend subroutine s\n",
        &indent_only_config(),
    )
    .unwrap()
    .bytes;
    let text = String::from_utf8(compact).unwrap();
    let indent_of = |needle: &str| {
        text.lines()
            .find(|line| line.trim_start() == needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in\n{text}"))
            .len()
            - needle.len()
    };
    // Two words, unspaced, and preserved through indent-only (which never
    // rewrites spelling): the compact form must still open a deeper frame
    // for CASE and its body, and close it again at ENDSELECT, exactly like
    // the two-word `select case`/`end select` spelling does.
    assert!(indent_of("case(1)") > indent_of("selectcase(x)"));
    assert!(indent_of("x=1") > indent_of("case(1)"));
    assert_eq!(indent_of("endselect"), indent_of("selectcase(x)"));

    let style = StyleConfig {
        keyword_case: KeywordCase::Upper,
        ..StyleConfig::default()
    };
    let full = format_source(b"selectcase(x)\nend select\n", &style_config(style))
        .unwrap()
        .bytes;
    let text = String::from_utf8(full).unwrap();
    assert!(
        text.contains("SELECT CASE(x)"),
        "expected SELECT CASE in\n{text}"
    );
}

#[test]
fn a_then_inside_a_string_literal_does_not_open_an_if_block() {
    // Reduced from OpenFAST `modules/wakedynamics/src/WakeDynamics.f90`: a
    // single-line `if (...) call foo('...then...')` whose message string
    // happens to contain the word "then" must not be misread as an
    // `IF ... THEN` block opener, or every statement for the rest of the
    // file inherits an extra, never-closed indent level.
    let source = b"program p\nif (a) call foo('when x is not set, then y must be set', b)\nx = 1\nend program p\n";
    let text =
        String::from_utf8(format_source(source, &indent_only_config()).unwrap().bytes).unwrap();
    let x_line = text.lines().find(|line| line.contains("x = 1")).unwrap();
    assert_eq!(x_line, "   x = 1", "expected no extra indent in\n{text}");
}

#[test]
fn a_pointer_assignment_to_a_keyword_name_does_not_open_a_construct() {
    // Reduced from MPAS `src/core_atmosphere/dynamics/mpas_atm_boundaries.F`,
    // which walks a linked list through a variable literally named `block`, and
    // from Q-E `PW/src/buffers.f90`, which does the same through `entry`.
    // findent's grammar spells the rule `assignment: lvalue '=' skipnoop /*
    // this includes '=>' */`, so a pointer assignment is an assignment even
    // when its target is spelled like a construct keyword.  Reading either as
    // BLOCK or ENTRY leaves a frame open for the rest of the file: this was 104
    // of MPAS's 106 indent-only oracle mismatches.
    let source = b"subroutine s\ntype(t), pointer :: block, entry, list\nblock => list\nentry%next => list\nx = 1\nend subroutine s\n";
    let text =
        String::from_utf8(format_source(source, &indent_only_config()).unwrap().bytes).unwrap();
    for body in ["block => list", "entry%next => list", "x = 1"] {
        let line = text
            .lines()
            .find(|line| line.trim_start() == body)
            .unwrap_or_else(|| panic!("missing {body:?} in\n{text}"));
        assert_eq!(line, format!("   {body}"), "wrong depth in\n{text}");
    }

    // The guard is the designator shape of the left-hand side, so a
    // pointer-initialised declaration keeps its own classification.
    let declarations = b"module m\ntype :: t\ninteger, pointer :: p => null()\ncontains\nprocedure :: run => t_run\nend type t\nend module m\n";
    let text = String::from_utf8(
        format_source(declarations, &indent_only_config())
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(
        text.contains("      integer, pointer :: p => null()"),
        "declaration lost its depth in\n{text}"
    );
    assert!(
        text.contains("      procedure :: run => t_run"),
        "type-bound binding lost its depth in\n{text}"
    );
}

#[test]
fn a_component_named_function_does_not_open_a_procedure() {
    // Reduced from CP2K `src/colvar_methods.F`.  The recognizer scans a
    // statement's words for FUNCTION/SUBROUTINE, which found the component in
    // `colvar%combine_cvs_param%function` and opened a procedure frame that was
    // never closed.  The keyword only counts at bracket depth zero and when no
    // `%` selects it.
    let source = b"subroutine s\ncall compress(colvar%combine_cvs_param%function, full=.true.)\nx = 1\nend subroutine s\n";
    let text =
        String::from_utf8(format_source(source, &indent_only_config()).unwrap().bytes).unwrap();
    let x_line = text.lines().find(|line| line.contains("x = 1")).unwrap();
    assert_eq!(x_line, "   x = 1", "expected no extra indent in\n{text}");

    // A real heading still opens one.
    let heading = b"module m\ncontains\npure integer function f(a)\ninteger :: a\nf = a\nend function f\nend module m\n";
    let text =
        String::from_utf8(format_source(heading, &indent_only_config()).unwrap().bytes).unwrap();
    assert!(
        text.contains("      f = a"),
        "prefixed function heading stopped opening a frame in\n{text}"
    );

    // And so does one whose prefix contains an ordinary name spelled like the
    // keyword.  Fortran reserves no words, so `function` is a legal name for
    // the named constant here; asking only about the *first* occurrence read
    // the kind parameter, rejected the heading, and left the body — and every
    // later sibling procedure — unindented.  findent 4.3.7 indents both.
    let shadowed = b"module m\ninteger, parameter :: function = 4\ncontains\ninteger(kind=function) function f()\nf = 1\nend function f\nsubroutine s()\nx = 0\nend subroutine s\nend module m\n";
    let text = String::from_utf8(
        format_source(shadowed, &indent_only_config())
            .unwrap()
            .bytes,
    )
    .unwrap();
    for expected in ["      f = 1", "      x = 0"] {
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
    }

    // Declaring a variable by that name is not a heading either.  A
    // `subroutine-stmt` names its procedure right after the keyword, and none
    // of these do; findent opens no frame for any of them.
    for declaration in [
        "integer :: subroutine",
        "integer :: function",
        "integer subroutine",
        "type(t) :: function",
    ] {
        let source = format!("subroutine outer()\n{declaration}\nx = 1\nend subroutine outer\n");
        let text = String::from_utf8(
            format_source(source.as_bytes(), &indent_only_config())
                .unwrap()
                .bytes,
        )
        .unwrap();
        assert!(
            text.lines().any(|line| line == "   x = 1"),
            "{declaration:?} opened a frame in\n{text}"
        );
    }
}

#[test]
fn a_comment_under_an_openmp_sentinel_follows_the_sentinel_body_depth() {
    // Reduced from SPECFEM3D
    // `external_libs/.../MESHER/sorting.f90`.  A `!$ ` line is buffered as Code
    // because of the sentinel, so a sentinel whose entire payload is a comment
    // had no statement to plan and fell to column zero, while the conditional
    // code on either side of it stayed indented.  findent reindents it with the
    // block.
    let source = b"subroutine s\n!$ do i = 1, p\n!$    x = 1\n!$    ! merge splitted arrays\n!$ end do\nend subroutine s\n";
    let text =
        String::from_utf8(format_source(source, &indent_only_config()).unwrap().bytes).unwrap();
    assert!(
        text.contains("!$    ! merge splitted arrays"),
        "sentinel comment lost its depth in\n{text}"
    );
    assert!(
        text.contains("!$    x = 1"),
        "sentinel code lost its depth in\n{text}"
    );
}

#[test]
fn one_line_if_goto_keeps_its_separator() {
    // Reduced from MPAS `src/core_atmosphere/physics/physics_wrf/module_cu_gf.mpas.F`.
    let source =
        b"subroutine p\ninteger :: i\nIF(ierr(i).ne.0)GO TO 62\n62 continue\nend subroutine p\n";
    let once = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let twice = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(once
        .windows(b"if (ierr(i) /= 0) goto 62".len())
        .any(|window| { window == b"if (ierr(i) /= 0) goto 62" }));
    assert_eq!(once, twice);
}

#[test]
fn split_end_do_collapses_the_named_construct_gap() {
    // Reduced from MPAS `src/core_atmosphere/physics/physics_wrf/module_mp_kessler.F`.
    let source =
        b"subroutine p\ninteger :: i\ndo 10 i = 1, 2\n10 ENDDO  loop_k\nend subroutine p\n";
    let once = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let twice = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(once
        .windows(b"10 end do loop_k".len())
        .any(|window| { window == b"10 end do loop_k" }));
    assert_eq!(once, twice);
}

#[test]
fn compact_elseif_removes_redundant_condition_parentheses() {
    // Reduced from Q-E `EPW/src/printing.f90`.
    let source = b"subroutine p\nlogical :: a, b\nIF (a) THEN\nx = 1\nELSEIF ((b)) THEN\nx = 2\nEND IF\nend subroutine p\n";
    let once = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let twice = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(once
        .windows(b"else if (b) then".len())
        .any(|window| { window == b"else if (b) then" }));
    assert_eq!(once, twice);
}

#[test]
fn acc_sentinel_without_a_blank_keeps_clause_identifier_case() {
    // Reduced from Q-E `LR_Modules/ccgsolve_all.f90` line 146.
    let source = b"subroutine p\ninteger :: my_nbnd\n!$ x = omp_get_num_threads()\n!$omp parallel do\n!$acc enter data create(rho(1:MY_NBND), alpha_long_name, beta_long_name, gamma_long_name, delta_long_name, epsilon_long_name, zeta_long_name, eta_long_name, theta_long_name, iota_long_name, kappa_long_name, lambda_long_name, mu_long_name, nu_long_name, xi_long_name, omicron_long_name, pi_long_name, rho_long_name, sigma_long_name)\n!$acc enter data create(rho(1:MY_NBND))\nend subroutine p\n";
    let once = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let twice = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    let text = String::from_utf8(once.clone()).unwrap();
    assert!(text.contains("!$ x = omp_get_num_threads()"));
    assert!(text.contains("!$OMP PARALLEL DO"));
    assert!(text.contains("!$acc enter data create(rho(1:MY_NBND)"));
    assert!(text.contains("!$acc enter data create(rho(1:MY_NBND))"));
    assert_eq!(once, twice);
}

#[test]
fn format_edit_descriptors_keep_authored_slashes() {
    // Reduced from Q-E `atomic/src/compute_phi.f90`.
    let source = b"subroutine s\n130  format (/ /5x, 3hfoo, &\n     f6.3)\nend subroutine s\n";
    let once = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let twice = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(once
        .windows(b"130 format (/ /5x, 3hfoo, &".len())
        .any(|window| { window == b"130 format (/ /5x, 3hfoo, &" }));
    assert_eq!(once, twice);

    // The guard keys on the statement label, because a `format-stmt` cannot be
    // reached without one.  An assignment to an array that happens to be named
    // `format` is an ordinary expression and keeps its operator spacing.
    let assignment =
        b"subroutine s\nreal :: format(10), x\ninteger :: i\nformat(i)=x*2+1\nend subroutine s\n";
    let text = String::from_utf8(
        format_source(assignment, &FormatConfig::default())
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(
        text.contains("format(i) = x*2 + 1"),
        "an array named `format` lost its operator spacing in\n{text}"
    );
}

#[test]
fn commented_relational_operators_are_spaced_in_one_pass() {
    // Reduced from Q-E `CPV/src/qmatrixd.f90`.
    let source = b"subroutine s\n  ! a=<b <= c >= d == e /= f => g\n  x = 1\nend subroutine s\n";
    let once = format_source(source, &FormatConfig::default())
        .unwrap()
        .bytes;
    let twice = format_source(&once, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert!(once
        .windows(b"! a = < b".len())
        .any(|window| window == b"! a = < b"));
    assert!(once
        .windows(b"<= c >= d == e /= f => g".len())
        .any(|window| window == b"<= c >= d == e /= f => g"));
    assert_eq!(once, twice);
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn compact_multiplicative_ignores_a_type_keyword_used_as_an_identifier() {
    // `integer` is both the leading type keyword of the declaration and,
    // legally, one of the declared names. The star in `integer*4` is a
    // length separator, but the star in `n*2` is ordinary multiplication
    // even though the assignment's target spells a type keyword.
    let mut style = StyleConfig::default();
    style.compact_multiplicative = false;
    let source = b"integer :: integer, n\ninteger = n*2\n";
    let formatted = format_source(source, &style_config(style)).unwrap().bytes;
    let text = String::from_utf8(formatted).unwrap();
    assert!(text.contains("integer = n * 2"));

    let declaration_star = b"character*8 name\n";
    let formatted = format_source(declaration_star, &style_config(style))
        .unwrap()
        .bytes;
    assert_eq!(formatted, declaration_star);
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn compound_and_goto_switches_are_independent_and_multiword_spacing_is_unconditional() {
    let source = b"EnDiF\nGo   To 10\nselect     case (x)\n";

    let mut join_off = StyleConfig::default();
    join_off.join_goto = false;
    let join_off = String::from_utf8(
        format_source(source, &style_config(join_off))
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(join_off.contains("end if"));
    assert!(!join_off.contains("goto"));
    assert!(join_off.contains("select case"));

    let mut split_off = StyleConfig::default();
    split_off.split_compound_keywords = false;
    let split_off = String::from_utf8(
        format_source(source, &style_config(split_off))
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(split_off.contains("endif"));
    assert!(split_off.contains("goto 10"));
    assert!(split_off.contains("select case"));

    let mut both_off = StyleConfig::default();
    both_off.join_goto = false;
    both_off.split_compound_keywords = false;
    let both_off = String::from_utf8(
        format_source(source, &style_config(both_off))
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(both_off.contains("endif"));
    assert!(both_off.contains("select case"));
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn delimiter_spacing_can_be_disabled_independently() {
    let mut style = StyleConfig::default();
    style.array_brackets = true;
    style.compact_multiplicative = false;
    style.delimiter_spacing = false;
    let output = format_source(b"x=(/a,b/)\nvalue=a*b\n", &style_config(style))
        .unwrap()
        .bytes;
    assert_eq!(output, b"x = [a,b]\nvalue = a * b\n");
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn lexical_switches_are_independent() {
    let source = b"program p\nx=(/a,b/) !comment\nif (a .and. &\n   & b) then\n!$omp parallel\n!$omp end parallel\nend if\nend program p\n";

    let mut comment_off = StyleConfig::default();
    comment_off.comment_spacing = false;
    let comment_off = String::from_utf8(
        format_source(source, &style_config(comment_off))
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(comment_off.contains("x = [a, b] !comment"));
    assert!(
        comment_off.contains("if (a .and. &\n   b) then"),
        "comment-off output:\n{comment_off}"
    );
    assert!(comment_off.contains("!$OMP PARALLEL"));

    let mut delimiter_off = StyleConfig::default();
    delimiter_off.delimiter_spacing = false;
    let delimiter_off = String::from_utf8(
        format_source(source, &style_config(delimiter_off))
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(delimiter_off.contains("x = [a,b] ! comment"));
    assert!(delimiter_off.contains("if (a .and. &\n   b) then"));
    assert!(delimiter_off.contains("!$OMP PARALLEL"));

    let mut continuation_off = StyleConfig::default();
    continuation_off.continuation_markers = false;
    let continuation_off = String::from_utf8(
        format_source(source, &style_config(continuation_off))
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(continuation_off.contains("x = [a, b] ! comment"));
    assert!(continuation_off.contains("if (a .and. &\n   & b) then"));
    assert!(continuation_off.contains("!$omp parallel"));
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn blank_line_cap_and_program_unit_preserve_are_independent() {
    let source = b"program p\nx=1\n\n\n\nend program p\n\n\n\n";
    let mut style = StyleConfig::default();
    style.program_unit_spacing = false;
    style.max_blank_lines = Some(1);
    let one = format_source(source, &style_config(style)).unwrap().bytes;
    assert_eq!(one, b"program p\nx = 1\n\nend program p\n");

    style.max_blank_lines = Some(0);
    let zero = format_source(source, &style_config(style)).unwrap().bytes;
    assert_eq!(zero, b"program p\nx = 1\nend program p\n");

    style.max_blank_lines = None;
    let unlimited = format_source(source, &style_config(style)).unwrap().bytes;
    assert_eq!(unlimited, b"program p\nx = 1\n\n\n\nend program p\n");
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn style_profiles_are_fixed_points_across_checked_in_fixtures() {
    let mut preserve = StyleConfig::default();
    preserve.keyword_case = KeywordCase::Preserve;
    preserve.relational_symbols = false;
    preserve.array_brackets = false;
    preserve.join_goto = false;
    preserve.split_compound_keywords = false;
    preserve.strip_empty_args = false;
    preserve.remove_redundant_parens = false;
    preserve.remove_terminal_return = false;
    preserve.program_unit_spacing = false;
    preserve.max_blank_lines = None;
    preserve.delimiter_spacing = false;
    preserve.comment_spacing = false;
    preserve.continuation_markers = false;

    let mut upper = StyleConfig::default();
    upper.keyword_case = KeywordCase::Upper;
    upper.compact_multiplicative = false;

    let mut preserved_operators = StyleConfig::default();
    preserved_operators.keyword_case = KeywordCase::Preserve;
    preserved_operators.relational_symbols = false;

    let mut lexical = StyleConfig::default();
    lexical.array_brackets = true;
    lexical.compact_multiplicative = false;
    lexical.delimiter_spacing = false;
    lexical.comment_spacing = false;
    lexical.continuation_markers = false;

    let mut unit_preserve = StyleConfig::default();
    unit_preserve.program_unit_spacing = false;
    unit_preserve.max_blank_lines = Some(0);

    let mut unlimited = StyleConfig::default();
    unlimited.max_blank_lines = None;

    let profiles = [
        StyleConfig::default(),
        preserve,
        upper,
        preserved_operators,
        lexical,
        unit_preserve,
        unlimited,
    ];
    let mut fixtures: Vec<_> = fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "f90"))
        .collect();
    fixtures.sort();
    for style in profiles {
        let config = FormatConfig {
            style,
            ..FormatConfig::default()
        };
        let indent = FormatConfig {
            mode: FormatMode::IndentOnly,
            style,
            ..FormatConfig::default()
        };
        for fixture in &fixtures {
            let source = fs::read(fixture).unwrap();
            let once = format_source(&source, &config)
                .unwrap_or_else(|error| panic!("{}: {error}", fixture.display()))
                .bytes;
            let twice = format_source(&once, &config)
                .unwrap_or_else(|error| panic!("{} second pass: {error}", fixture.display()))
                .bytes;
            assert_eq!(once, twice, "I1 failed for {}", fixture.display());
            assert_eq!(
                format_source(&once, &indent).unwrap().bytes,
                once,
                "I2 failed for {}",
                fixture.display()
            );
        }
    }
}

#[test]
fn default_formatting_is_idempotent_on_malformed_and_lexical_inputs() {
    let inputs: &[&[u8]] = &[
        b"",
        b"program p\nif (x) then\nx = 1\nend if\nend program\n",
        b"program p  \n! caf\xe9\nx = \"!;&\"; 4H;! comment\nend program",
        b"#if X\nif (x) then\n#else\nif (y) then\n#endif\nx=1\nend if\n",
        b"program p\nif (x) then\n",
        &[0, 1, 2, b'\n', 0xff, b'!', b'\n', b')', b'('],
    ];
    for source in inputs {
        let config = FormatConfig {
            mode: FormatMode::IndentOnly,
            ..FormatConfig::default()
        };
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "source was not idempotent: {source:?}");
    }
}

#[test]
fn full_mode_chunk_a_preserves_protected_bytes_and_is_idempotent() {
    let source = b"PROGRAM P\nCALL F('IF  THEN  ', 4Hab  c) ! x=1+2\n#define IF_THING 1\nIF (X) THEN\nEND IF\nEND PROGRAM P\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice, "full mode is not a fixed point");

    let protected = |bytes: &[u8]| {
        let mut literals = Vec::new();
        let mut cpp = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
                cpp.push(line.to_vec());
            }
            for region in regions(line) {
                if matches!(
                    region.kind,
                    RegionKind::StringLiteral | RegionKind::Hollerith
                ) {
                    literals.push(line[region.range].to_vec());
                }
            }
        }
        (literals, cpp)
    };
    assert_eq!(protected(source), protected(&once));
}

#[test]
fn full_mode_fixed_point_and_indent_only_fixed_point_hold_together() {
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    for source in [
        include_bytes!("fixtures/core.f90").as_slice(),
        include_bytes!("fixtures/cpp_continuation.f90").as_slice(),
        include_bytes!("fixtures/array_constructor_multiline.f90").as_slice(),
        b"\n!$ \n".as_slice(),
        b"program p\nif (x) then\ncall f(a, b, c, d, e, f, g, h)\nend if\nend program p\n"
            .as_slice(),
    ] {
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");

        let indent = FormatConfig {
            mode: FormatMode::IndentOnly,
            ..FormatConfig::default()
        };
        assert_eq!(format_source(&once, &indent).unwrap().bytes, once, "I2");
    }
}

#[test]
fn a_declared_name_keeps_its_spelling_on_a_continued_entity_list() {
    // Reduced from CP2K, which really does declare a component called `TYPE`.
    // The rule that protects a declared name looks for the `::` on its own
    // line; once the wrapper moved `TYPE` onto a continuation there was no
    // `::` to find, so it read as the keyword and was lowercased.
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "module m\n   type t\n      integer :: ref_count = -1, &\n         TYPE = -1, other = -1\n   end type t\nend module m\n",
            "         TYPE = -1, other = -1",
        ),
        // An initializer is an expression, not an entity: names in it are
        // resolved as code.
        (
            "module m\n   integer :: a = 1, &\n      b = SIZE(x)\nend module m\n",
            "      b = size(x)",
        ),
        // So is anything inside a group the continuation is still nested in.
        (
            "module m\n   integer :: a = f( &\n      TYPE)\nend module m\n",
            "      type)",
        ),
        // Reduced from CP2K `src/smeagol_matrix_utils.F`: an initializer split
        // across its own `=` leaves neither the `::` nor the `=` on this line,
        // so the whole continuation used to read as entity names.  `INT` kept
        // its authored spelling on the second corpus pass only, because the
        // first pass is what had wrapped it.
        (
            "module m\n   integer, parameter :: k = 8\n   integer, parameter :: y = 4/ &\n      INT(k, kind=k)\nend module m\n",
            "      int(k, kind=k)",
        ),
        // A top-level comma closes that initializer again, so the entity it
        // starts is still a declared name.
        (
            "module m\n   integer :: a = f(1) + 2, &\n      TYPE = 3\nend module m\n",
            "      TYPE = 3",
        ),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn a_wrapped_declaration_does_not_repartition_its_alignment_block() {
    // Reduced from SPECFEM3D at `--line-length=80`. The wrapper measures the
    // laid-out width, which step 17 sets from the alignment block a line is in.
    // A continuation used to end that block, so wrapping the first declaration
    // moved every declaration below it into a different block, with a different
    // column and a different width — and the next run measured that width and
    // made a different wrapping decision. The loop only closes if the partition
    // does not depend on where the wrapper broke.
    let source = "\
program p
    integer,                                intent(in)     :: myrank
    ! local
    integer                                                :: ievent, ireceiver, nsta_slice, irec_local, NSTA, NEVENT, ier
    integer                                                :: ispec_selected, islice_selected, idim
    double precision                                       :: xi_receiver, eta_receiver, gamma_receiver
    double precision                                       :: x_found,  y_found,  z_found
    double precision                                       :: x_to_locate, y_to_locate, z_to_locate
    real(kind=CUSTOM_REAL)                                 :: distance_min_glob,distance_max_glob
    real(kind=CUSTOM_REAL)                                 :: elemsize_min_glob,elemsize_max_glob
    real(kind=CUSTOM_REAL)                                 :: x_min_glob,x_max_glob
    real(kind=CUSTOM_REAL)                                 :: y_min_glob,y_max_glob
    real(kind=CUSTOM_REAL)                                 :: z_min_glob,z_max_glob
    integer,                 dimension(NGNOD)              :: iaddx,iaddy,iaddz
    double precision,        dimension(NGLLX)              :: hxis,hpxis
end program p
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "wrapping one declaration changed the block the others are measured in"
    );
}

#[test]
fn a_dotted_operator_before_a_continuation_leaves_the_next_sign_unary() {
    // Reduced from CP2K at `--indent=8`, where the wrapper breaks after
    // `.or. &`. Deciding "the previous line ended on an operand" from its last
    // byte counted the closing `.` of `.or.`, so the leading `-` of the next
    // line was spaced as if it were binary — one run after the wrapper created
    // the break.
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "program p\nif (a > c .or. &\n   -b > c) then\nx = 1\nend if\nend program p\n",
            "      -b > c) then",
        ),
        // The operand cases this guard has to leave alone: a name, and a
        // decimal point, both of which really do end on an operand.
        ("program p\nx = a &\n   - b\nend program p\n", "      - b"),
        ("program p\nx = 1. &\n   - b\nend program p\n", "      - b"),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn step_17_relayout_keeps_paren_alignment_on_the_width_it_emits() {
    // Reduced from CP2K at `--align-paren`. The engine aligns the continuation
    // under the `[` of the head line; step 17 then compresses that head's `::`
    // and moves the `[` twenty columns left, stranding the continuation where
    // the `[` used to be. The next run reads the compressed head and aligns
    // correctly, so the two runs disagree.
    let source = "\
module m
   character(len=3), DIMENSION(7), &
      PARAMETER, PUBLIC                     :: periodicity_string = [\"  X\", \"  Y\", \"  Z\", &
                                                                     \" XY\", \" XZ\", \" YZ\", &
                                                                     \"XYZ\"]
end module m
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        align_paren: true,
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "step 17 moved a head line the engine had already aligned against"
    );
    // Paren alignment points at something on the line above, so a continuation
    // can never start past the end of the head it is aligned to. Before the
    // re-layout the head was compressed to 71 columns and the continuation
    // stayed at 72 — aligned to a `[` that had moved.
    let text = String::from_utf8_lossy(&once);
    let head = text
        .lines()
        .find(|line| line.contains("periodicity_string"))
        .expect("head line");
    for line in text.lines().filter(|line| line.contains("\" XY\"")) {
        let indent = line.len() - line.trim_start().len();
        assert!(
            indent > 0 && indent <= head.len(),
            "continuation indent {indent} does not point into the {}-column head",
            head.len()
        );
    }
}

#[test]
fn a_declaration_entity_after_an_array_constructor_is_not_a_named_argument() {
    // Reduced from CP2K. The continuation line carries no statement context, so
    // `, b =` after a closing `]` looked like a keyword argument and was
    // compacted to `b=`. Named arguments live in `(...)`; `[...]` is an array
    // constructor, and after its comma comes the next entity of the
    // declaration list.
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "module m\n   real(kind=dp), parameter :: a = [1.0_dp, &\n      2.0_dp], b = &\n      [3.0_dp, 4.0_dp]\nend module m\n",
            "      2.0_dp], b = &",
        ),
        // The rule this guard narrows still has to do its job inside `(...)`.
        (
            "module m\n   real(kind=dp), parameter :: a = f(p=1, &\n      q=2)\nend module m\n",
            "      q=2)",
        ),
        (
            "program p\ncall f(a=1, &\n   b=2, &\n   c=3)\nend program p\n",
            "      b=2, &",
        ),
        // A continuation can close a bracket and land back inside the call, so
        // the decision belongs at the `=`, not to the line as a whole.
        (
            "program p\ncall g(sum([1, &\n   2, 3]), dim=1)\nend program p\n",
            "      2, 3]), dim=1)",
        ),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn a_continued_named_argument_stays_compact_even_with_continuation_markers_off() {
    // Reduced from OpenFAST. `compact_continued_named_argument` used to run
    // only when `--continuation-markers` was on, even though it is fixing a
    // rule-4 spacing decision that has nothing to do with marker
    // normalization. With markers off, a long statement that gets rejoined
    // and rewrapped across a physical continuation boundary picked up spaces
    // around `SHAPE=` on the first pass that this fixup never removed, and
    // the second pass (reading the statement pre-split instead of
    // pre-joined) got the compact spelling right — so the two passes
    // disagreed.
    let mut style = StyleConfig::default();
    style.continuation_markers = false;
    style.delimiter_spacing = false;
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        style,
        ..FormatConfig::default()
    };
    let source = b"module repro\ncontains\nsubroutine s\nreal(DbKi), parameter :: RotGtoL(3,3) = reshape( [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], SHAPE=[3,3] )\nend subroutine s\nend module repro\n";
    let once = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(once.clone()).unwrap();
    assert!(
        text.contains("SHAPE=[3,3]"),
        "expected SHAPE=[3,3] in\n{text}"
    );
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice, "full mode I1 failed for {source:?}");
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn a_named_argument_continued_by_a_marker_alone_keeps_its_gap() {
    // A named argument whose value is entirely on the next physical line
    // (`stat= &` with nothing but the continuation marker after the `=`) is
    // not `name=value` split across a boundary — there is no value on this
    // line to compact against. Reaching past the marker onto its own space
    // produced `stat=&`, corrupting the marker's spacing on every following
    // run of `--continuation-markers=0` corpora that contain this shape.
    let mut style = StyleConfig::default();
    style.continuation_markers = false;
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        style,
        ..FormatConfig::default()
    };
    let source = b"program p\nallocate(a(n), b(n), stat= &\n   ierr)\nend program p\n";
    let once = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(once.clone()).unwrap();
    assert!(
        text.contains("stat= &"),
        "expected the marker's own space to survive in\n{text}"
    );
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice, "full mode I1 failed for {source:?}");
}

#[test]
fn wrapping_measures_the_declaration_separator_step_17_will_emit() {
    // Reduced from SPECFEM3D at `--line-length=80`. The author lined these
    // `::` up in a very wide block; step 17 compresses that block, so the line
    // the wrapper reads is 120 columns and the line it emits is 81. Wrapping
    // the authored spelling found no break that left the head inside the
    // budget and declined, and the next run — reading the compressed 81-column
    // line — wrapped it happily.
    // The authored `::` sits past column 80, so no break — not even one
    // immediately after the `::` — leaves the head inside the budget. Step 17
    // moves it to column 29, where a break after `=` fits.
    let source = "\
module m
   type t
      logical                                                                     :: dump_model_at_each_iteration = .true.
      logical                                                                     :: dump_descent_direction_at_each_iteration = .true.
      !! user-defined taper
      real(kind=CUSTOM_REAL)                                                      :: xmin_taper, xmax_taper
   end type t
end module m
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "wrapping and step 17 disagreed about the emitted width"
    );
    for line in once.split(|byte| *byte == b'\n') {
        assert!(
            line.len() <= 80 || String::from_utf8_lossy(line).contains("::"),
            "line over budget: {}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn wrapping_measures_every_declaration_separator_a_body_carries() {
    // Reduced from a CosmoMC-style likelihood module at `--line-length=80`.
    // The entity has a typed array-constructor initializer, so the body
    // carries a second `::` — the array constructor's own `type ::` — beside
    // the declaration's. `declaration_separator_growth` used to stop at the
    // first `::` it found and return, so it paid step 17's padding for the
    // declaration's `::` but not for the constructor's: once wrapping put
    // `[character(7)::...]` on its own physical line, step 17 treated that
    // `::` as a separator in its own right and padded it too, pushing the
    // line one column over 80. The next run then saw that over-long line and
    // rewrapped it differently, breaking I1.
    let source = "\
module m
contains
   subroutine s
      character(LEN=7), parameter :: spectrum_names(6) = &
          [character(7)::'100x100','143x143','217x217','143x217','TE','EE']
   end subroutine s
end module m
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "wrapping and step 17 disagreed about the emitted width"
    );
    for line in once.split(|byte| *byte == b'\n') {
        assert!(
            line.len() <= 80,
            "line over budget: {}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn end_keyword_spacing_stops_at_the_statement_it_owns() {
    // Both spellings reduced from SPECFEM3D. A compound rewrite (`endif` ->
    // `end if`) hands the next pass two tokens where there was one, so the
    // `end <keyword>` spacing rule saw a line it had not seen before and
    // reached past the keyword: into rule 5's preserved `!!` gap, and into an
    // empty gap in front of `;`, which turned a collapse into an insertion.
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "program p\nif (x) then\ny = 1\nendif  !! trailing doc\nend program p\n",
            "   end if  !! trailing doc",
        ),
        (
            "program p\nif (x) then\ny = 1\nend if  !! trailing doc\nend program p\n",
            "   end if  !! trailing doc",
        ),
        (
            "program p\ndo i = 1, 2\ndo j = 1, 2\ny = 1\nenddo; enddo\nend program p\n",
            "      end do; enddo",
        ),
        (
            "program p\ndo i = 1, 2\ndo j = 1, 2\ny = 1\nend do; enddo\nend program p\n",
            "      end do; enddo",
        ),
        // The collapse this rule does own must survive the narrowing.
        (
            "module m\ncontains\nsubroutine s\ny = 1\nend subroutine    s\nend module m\n",
            "   end subroutine s",
        ),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn end_construct_keyword_wins_over_a_same_spelled_declared_name() {
    // Reduced from SPECFEM3D. `end` is both the block-end keyword and, here, a
    // declared dummy argument and local variable; `do` is also declared. The
    // declared-case engine used to govern every occurrence of those spellings
    // on a line, including the leading keyword of `end do` and `end subroutine` — so
    // under `--keyword-case=upper` the first run emitted `END DO` correctly,
    // and only the *second* run (which starts from a document where `END` and
    // `DO` are already separate tokens) recast the leading `END` down to
    // `end`, because the declared-name lookup ran on that token too.
    let config = FormatConfig {
        mode: FormatMode::Full,
        style: StyleConfig {
            keyword_case: KeywordCase::Upper,
            ..StyleConfig::default()
        },
        ..FormatConfig::default()
    };
    let source = b"subroutine p(end)\nreal end, do\ndo i = 1, 2\nenddo\nend subroutine p\n";
    let once = format_source(source, &config).unwrap().bytes;
    let text = String::from_utf8(once.clone()).unwrap();
    assert!(text.contains("END DO"), "expected END DO in\n{text}");
    assert!(
        text.contains("END SUBROUTINE"),
        "expected END SUBROUTINE in\n{text}"
    );
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice, "full mode I1 failed for {source:?}");
}

#[test]
fn full_mode_protected_spans_are_byte_exact() {
    let source = b"program p\ncharacter(len=20) :: s = 'IF  THEN  ' ! body  x = 1\nx = 4Hab  c\n#if defined(X)\nIF (X) THEN\n#endif\nend program p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let collect = |bytes: &[u8]| {
        let mut strings = Vec::new();
        let mut hollerith = Vec::new();
        let mut cpp = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
                cpp.push(line.to_vec());
            }
            for region in regions(line) {
                match region.kind {
                    RegionKind::StringLiteral => strings.push(line[region.range].to_vec()),
                    RegionKind::Hollerith => hollerith.push(line[region.range].to_vec()),
                    _ => {}
                }
            }
        }
        (strings, hollerith, cpp)
    };
    assert_eq!(collect(source), collect(&once));
}

#[test]
fn default_formatting_preserves_line_bodies_except_trailing_horizontal_space() {
    let source = b"program p  \r\n  x = \"a  b\"  \n! comment  \r\nend program";
    let output = format_source(source, &indent_only_config()).unwrap().bytes;
    assert_eq!(trimmed_line_bodies(source), line_bodies(&output));
}

#[test]
fn default_formatting_allows_only_label_padding_to_change() {
    let source = b"  program p\n10      continue ! keep  \n  end program p\n";
    let output = format_source(source, &indent_only_config()).unwrap().bytes;
    assert_eq!(
        normalized_line_bodies(source),
        normalized_line_bodies(&output)
    );
}

#[test]
fn whitespace_reduction_bypasses_hollerith_payloads() {
    let source = b"program p\nx = 4Ha  b ! comment\nend program p\n";
    let config = FormatConfig {
        ws_remred: true,
        mode: FormatMode::IndentOnly,
        ..FormatConfig::default()
    };
    let output = format_source(source, &config).unwrap().bytes;
    assert!(
        output
            .windows(b"4Ha  b ! comment".len())
            .any(|window| window == b"4Ha  b ! comment"),
        "Hollerith payload was changed: {output:?}"
    );
}

#[test]
fn streaming_api_matches_owned_api() {
    let source = b"program p\nif (x) then\nx = 1\nend if\nend program\n";
    let config = FormatConfig::default();
    let owned = format_source(source, &config).unwrap();
    let mut output = Vec::new();
    let meta = format_to(source, &config, &mut output).unwrap();
    assert_eq!(output, owned.bytes);
    assert_eq!(meta, owned.meta);

    let mut owned_output = Vec::new();
    let owned_meta = format_to_owned(source.to_vec(), &config, &mut owned_output).unwrap();
    assert_eq!(owned_output, owned.bytes);
    assert_eq!(owned_meta, owned.meta);
}

#[test]
fn full_streaming_api_matches_owned_api() {
    let source = b"PROGRAM p\nIF (x) THEN\nCALL f(a, b, c, d)\nEND IF\nEND PROGRAM p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let owned = format_source(source, &config).unwrap();
    let mut output = Vec::new();
    let meta = format_to(source, &config, &mut output).unwrap();
    assert_eq!(output, owned.bytes);
    assert_eq!(meta, owned.meta);

    let mut owned_output = Vec::new();
    let owned_meta = format_to_owned(source.to_vec(), &config, &mut owned_output).unwrap();
    assert_eq!(owned_output, owned.bytes);
    assert_eq!(owned_meta, owned.meta);
}

#[test]
fn unknown_statements_do_not_invent_structural_depth() {
    let source = b"program p\nif (x) then\neditor ???\ncontinue\nend if\nend program\n";
    let output = format_source(source, &indent_only_config()).unwrap().bytes;
    assert_eq!(
        output,
        b"program p\n   if (x) then\n      editor ???\n      continue\n   end if\nend program\n"
    );
}

#[test]
fn full_mode_unknown_statements_still_have_stable_structure() {
    let source = b"PROGRAM p\nIF (x) THEN\neditor ???\nCONTINUE\nEND IF\nEND PROGRAM p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice);
    assert!(once
        .windows(b"editor ???".len())
        .any(|w| w == b"editor ???"));
}

#[test]
fn keyword_case_changes_spelling_but_not_indent_depth() {
    let lower = b"program p\nif (x) then\ncontinue\nelse\ncontinue\nend if\nend program\n";
    let upper = b"PROGRAM p\nIF (x) THEN\ncontinue\nELSE\ncontinue\nEND IF\nEND PROGRAM\n";
    let lower_output = format_source(lower, &FormatConfig::default())
        .unwrap()
        .bytes;
    let upper_output = format_source(upper, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert_eq!(indent_columns(&lower_output), indent_columns(&upper_output));
}

#[test]
fn keyword_case_mutations_preserve_fixture_indent_depth() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("fixtures/core.f90"),
        include_bytes!("fixtures/constructs.f90"),
        include_bytes!("fixtures/advanced_constructs.f90"),
        include_bytes!("fixtures/cpp_nested.f90"),
        include_bytes!("fixtures/legacy_controls.f90"),
    ];
    for source in fixtures {
        let upper: Vec<u8> = source.iter().map(u8::to_ascii_uppercase).collect();
        let original = format_source(source, &FormatConfig::default())
            .expect("original fixture formats")
            .bytes;
        let mutated = format_source(&upper, &FormatConfig::default())
            .expect("case-mutated fixture formats")
            .bytes;
        assert_eq!(indent_columns(&original), indent_columns(&mutated));
    }
}

#[test]
fn case_and_spacing_mutations_of_every_fixture_are_fixed_points() {
    let mut fixtures: Vec<PathBuf> =
        fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "f90"))
            .collect();
    fixtures.sort();

    for fixture in fixtures {
        let source = fs::read(&fixture).unwrap();
        for (name, mutated) in [
            ("case", mutate_fixture(&source, Mutation::Case)),
            ("spacing", mutate_fixture(&source, Mutation::Spacing)),
        ] {
            let once = format_source(&mutated, &FormatConfig::default())
                .unwrap_or_else(|error| panic!("{name} mutation of {}: {error}", fixture.display()))
                .bytes;
            let twice = format_source(&once, &FormatConfig::default())
                .unwrap_or_else(|error| {
                    panic!("second {name} pass of {}: {error}", fixture.display())
                })
                .bytes;
            assert_eq!(
                once,
                twice,
                "I1 failed for {name} mutation of {}",
                fixture.display()
            );
            assert_eq!(
                format_source(&once, &indent_only_config()).unwrap().bytes,
                once,
                "I2 failed for {name} mutation of {}",
                fixture.display()
            );
        }
    }
}

#[derive(Clone, Copy)]
enum Mutation {
    Case,
    Spacing,
}

fn mutate_fixture(source: &[u8], mutation: Mutation) -> Vec<u8> {
    let mut state = LexState::default();
    let mut output = Vec::with_capacity(source.len());
    for line in source.split_inclusive(|byte| *byte == b'\n') {
        let first = line
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .map_or(b'\n', |index| line[index]);
        if first == b'#' {
            output.extend_from_slice(line);
            state = LexState::default();
            continue;
        }
        output.extend(map_code(line, &mut state, |code, output| match mutation {
            Mutation::Case => uppercase_code_identifiers(code, output),
            Mutation::Spacing => remove_code_spacing(code, output),
        }));
    }
    output
}

fn uppercase_code_identifiers(code: &[u8], output: &mut Vec<u8>) {
    let mut index = 0;
    while index < code.len() {
        if code[index].is_ascii_alphabetic() || code[index] == b'_' {
            let start = index;
            index += 1;
            while index < code.len() && (code[index].is_ascii_alphanumeric() || code[index] == b'_')
            {
                index += 1;
            }
            output.extend(code[start..index].iter().map(u8::to_ascii_uppercase));
        } else {
            output.push(code[index]);
            index += 1;
        }
    }
}

fn remove_code_spacing(code: &[u8], output: &mut Vec<u8>) {
    let is_spacing_target =
        |byte: u8| matches!(byte, b'-' | b'+' | b'*' | b'/' | b'=' | b'<' | b'>' | b',');
    for (index, &byte) in code.iter().enumerate() {
        if (byte == b' ' || byte == b'\t')
            && (index > 0 && is_spacing_target(code[index - 1])
                || code
                    .get(index + 1)
                    .is_some_and(|next| is_spacing_target(*next)))
        {
            continue;
        }
        output.push(byte);
    }
}

#[test]
fn arbitrary_byte_inputs_are_total_without_utf8_assumptions() {
    for seed in 0u8..128 {
        let mut source = Vec::with_capacity(384);
        for index in 0..384u16 {
            let byte = seed
                .wrapping_mul(31)
                .wrapping_add(index as u8)
                .rotate_left((index % 8) as u32);
            source.push(if index % 29 == 0 { b'\n' } else { byte });
        }
        format_source(&source, &FormatConfig::default()).expect("arbitrary bytes are total");
    }
}

#[test]
fn arbitrary_non_ascii_bytes_in_comments_and_strings_are_transparent() {
    for value in 0x80u8..=0xff {
        let mut source = b"program p\n! comment ".to_vec();
        source.push(value);
        source.extend_from_slice(b"\nx = \"");
        source.push(value);
        source.extend_from_slice(b"\"  \nend program\n");
        let output = format_source(&source, &indent_only_config())
            .expect("non-UTF-8 source remains formatable")
            .bytes;
        assert_eq!(
            trimmed_line_bodies(&source),
            line_bodies(&output),
            "byte {value:#x}"
        );
    }
}

#[test]
fn source_and_logical_group_spans_stay_inside_the_input() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("fixtures/core.f90"),
        include_bytes!("fixtures/lexical.f90"),
        include_bytes!("fixtures/align_nested.f90"),
        include_bytes!("fixtures/align_legacy_full.f90"),
        include_bytes!("fixtures/cpp_continuation.f90"),
        include_bytes!("fixtures/malformed_end.f90"),
        include_bytes!("fixtures/malformed_end_matrix.f90"),
        include_bytes!("fixtures/labeled_cpp_do.f90"),
        include_bytes!("fixtures/legacy_free_matrix.f90"),
    ];
    for source in fixtures {
        assert_valid_spans(source);
        for end in 0..=source.len() {
            assert_valid_spans(&source[..end]);
        }
    }
}

#[test]
fn fixture_prefixes_are_total_and_idempotent() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("fixtures/core.f90"),
        include_bytes!("fixtures/lexical.f90"),
        include_bytes!("fixtures/align.f90"),
        include_bytes!("fixtures/align_nested.f90"),
        include_bytes!("fixtures/constructs.f90"),
        include_bytes!("fixtures/construct_options.f90"),
        include_bytes!("fixtures/advanced_constructs.f90"),
        include_bytes!("fixtures/benchmark.f90"),
        include_bytes!("fixtures/benchmark_continuation.f90"),
        include_bytes!("fixtures/benchmark_preprocessor.f90"),
        include_bytes!("fixtures/cli_layout.f90"),
        include_bytes!("fixtures/cpp_continuation.f90"),
        include_bytes!("fixtures/cpp_continuation_indent.f90"),
        include_bytes!("fixtures/cpp_nested.f90"),
        include_bytes!("fixtures/engine_options.f90"),
        include_bytes!("fixtures/fortran2023.f90"),
        include_bytes!("fixtures/label_matrix.f90"),
        include_bytes!("fixtures/legacy_controls.f90"),
        include_bytes!("fixtures/malformed_end.f90"),
        include_bytes!("fixtures/legacy_recovery.f90"),
        include_bytes!("fixtures/labeled_cpp_do.f90"),
        include_bytes!("fixtures/legacy_free_matrix.f90"),
        include_bytes!("fixtures/openmp_continuation.f90"),
        include_bytes!("fixtures/procedure_decl.f90"),
        include_bytes!("fixtures/procedure_matrix.f90"),
        include_bytes!("fixtures/refactor.f90"),
        include_bytes!("fixtures/query.f90"),
        include_bytes!("fixtures/structures.f90"),
        include_bytes!("fixtures/ws_full.f90"),
        include_bytes!("fixtures/ws_remred.f90"),
    ];
    for source in fixtures {
        let cuts = [
            0,
            source.len() / 3,
            source.len() / 2,
            source.len().saturating_sub(1),
            source.len(),
        ];
        for cut in cuts {
            let prefix = &source[..cut];
            let buffer = SourceBuffer::new(prefix).expect("prefix is within the byte-size limit");
            for line in &buffer.lines {
                assert!(line.span.start <= line.span.end);
                assert!(line.code_span.start <= line.code_span.end);
                assert!(line.span.end as usize <= prefix.len());
                assert!(line.code_span.end as usize <= prefix.len());
                if let Some(comment) = &line.comment_span {
                    assert!(comment.start <= comment.end);
                    assert!(comment.end as usize <= prefix.len());
                }
            }
            for group in LogicalGroup::assemble(&buffer) {
                assert!(group.lines.start <= group.lines.end);
                assert!(group.lines.end <= buffer.lines.len());
            }
            let once = format_source(prefix, &indent_only_config()).expect("formatter is total");
            let twice = format_source(&once.bytes, &indent_only_config())
                .expect("formatted prefix remains total");
            assert_eq!(
                once.bytes, twice.bytes,
                "prefix at {cut} was not idempotent"
            );
        }
    }
}

fn line_bodies(source: &[u8]) -> Vec<Vec<u8>> {
    let mut bodies = Vec::new();
    let mut start = 0;
    for (index, &byte) in source.iter().enumerate() {
        if byte == b'\n' {
            let mut end = index;
            if end > start && source[end - 1] == b'\r' {
                end -= 1;
            }
            bodies.push(body(&source[start..end]));
            start = index + 1;
        }
    }
    if start < source.len() {
        bodies.push(body(&source[start..]));
    }
    bodies
}

fn trimmed_line_bodies(source: &[u8]) -> Vec<Vec<u8>> {
    line_bodies(source)
        .into_iter()
        .map(|line| {
            let end = line
                .iter()
                .rposition(|byte| *byte != b' ' && *byte != b'\t')
                .map_or(0, |index| index + 1);
            line[..end].to_vec()
        })
        .collect()
}

fn assert_valid_spans(source: &[u8]) {
    let buffer = SourceBuffer::new(source).expect("source buffer accepts byte input");
    for line in &buffer.lines {
        assert!(line.span.end as usize <= source.len());
        assert!(line.code_span.start >= line.span.start);
        assert!(line.code_span.end <= line.span.end);
        if let Some(comment) = &line.comment_span {
            assert!(comment.start >= line.code_span.start);
            assert!(comment.end <= line.span.end);
        }
        let _ = buffer.line_bytes(line);
        let _ = buffer.code_bytes(line);
    }
    for group in LogicalGroup::assemble(&buffer) {
        assert!(group.lines.start <= group.lines.end);
        assert!(group.lines.end <= buffer.lines.len());
        for statement in group.statements {
            assert!(!statement.text.is_empty());
        }
    }
}

fn normalized_line_bodies(source: &[u8]) -> Vec<Vec<u8>> {
    trimmed_line_bodies(source)
        .into_iter()
        .map(|line| normalize_label_padding(&line))
        .collect()
}

fn body(line: &[u8]) -> Vec<u8> {
    let start = line
        .iter()
        .position(|byte| *byte != b' ' && *byte != b'\t')
        .unwrap_or(line.len());
    line[start..].to_vec()
}

fn indent_columns(source: &[u8]) -> Vec<usize> {
    let mut result = Vec::new();
    let mut start = 0;
    for (index, byte) in source.iter().enumerate() {
        if *byte == b'\n' {
            let end = if index > start && source[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            let line = &source[start..end];
            result.push(
                line.iter()
                    .take_while(|byte| **byte == b' ' || **byte == b'\t')
                    .count(),
            );
            start = index + 1;
        }
    }
    if start < source.len() {
        result.push(
            source[start..]
                .iter()
                .take_while(|byte| **byte == b' ' || **byte == b'\t')
                .count(),
        );
    }
    result
}

fn normalize_label_padding(line: &[u8]) -> Vec<u8> {
    let digits = line.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 || digits == line.len() || !line[digits].is_ascii_whitespace() {
        return line.to_vec();
    }
    let after_label = line[digits..]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|offset| digits + offset)
        .unwrap_or(line.len());
    let mut normalized = line[..digits].to_vec();
    normalized.extend_from_slice(&line[after_label..]);
    normalized
}

/// A wrapping profile deep enough that a hand-aligned declaration block sits
/// near the budget, which is where step 16 and step 17 disagree.
fn deep_wrapping_config() -> FormatConfig {
    FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        indent: 8,
        // The engine reads the per-construct indents, not the scalar, so
        // setting only `indent` leaves every construct at the default 3 and
        // the block never reaches the columns where the two passes disagree.
        construct_indents: forformat::ConstructIndents::with_indent(8),
        contains_indent: 8,
        continuation_indent: 8,
        align_paren: true,
        align_paren_value: 4,
        ..FormatConfig::default()
    }
}

fn full_twice(source: &str, config: &FormatConfig) -> (String, String) {
    let once = format_source(source.as_bytes(), config).unwrap().bytes;
    let twice = format_source(&once, config).unwrap().bytes;
    (
        String::from_utf8(once).unwrap(),
        String::from_utf8(twice).unwrap(),
    )
}

#[test]
fn wrapping_a_block_mate_does_not_change_the_separator_a_declaration_was_measured_against() {
    // Reduced from CP2K `src/rpa_grad.F`.  `TYPE(...)` is aligned into a block
    // with the `REAL` declarations, so step 17 holds its `::` out at the
    // block's shared column and the wrapper measures it there — but wrapping
    // those neighbours moves their separators onto continuation lines, the
    // block breaks up, and the `::` is emitted at one space instead.  The
    // statement was left at 82 columns with `NoSafeBreak` and the next run,
    // reading the compressed spelling, wrapped it cleanly.
    let source = "\
MODULE m
CONTAINS
   SUBROUTINE s(ispin)
      INTEGER :: ispin
      IF (ispin > 1) THEN
         IF (ispin > 2) THEN
            IF (ispin > 3) THEN
               INTEGER :: send_a_start, send_a_end, send_a_size, &
                          recv_a_start, recv_a_end, recv_a_size, proc_shift
               REAL(KIND=dp), DIMENSION(:), ALLOCATABLE, TARGET :: buffer_send_1D
               REAL(KIND=dp), DIMENSION(:, :), POINTER :: buffer_send
               REAL(KIND=dp), DIMENSION(:, :), ALLOCATABLE :: buffer_recv
               TYPE(group_dist_d1_type)                           :: gd_virtual_sub
            END IF
         END IF
      END IF
   END SUBROUTINE s
END MODULE m
";
    let (once, twice) = full_twice(source, &deep_wrapping_config());
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
    for line in once.lines() {
        assert!(
            line.len() <= 80,
            "emitted {} columns, over the budget it was measured against: {line}",
            line.len()
        );
    }
}

#[test]
fn a_separator_on_a_continuation_line_is_still_the_statement_s_own() {
    // Reduced from CP2K `src/subsys/cell_types.F`.  The author broke this
    // declaration before its attributes, so the `::` is on the continuation
    // and the group's *head* line carries none.  Measuring only the head line
    // found no separator at all, so the wrapper sized and broke the authored
    // run while step 17 emitted the aligned one, and the two passes chose
    // different break points.
    let source = "\
MODULE m
   CHARACTER(LEN=3), DIMENSION(7), &
      PARAMETER, PUBLIC                     :: periodicity_string = [\"  X\", \"  Y\", \"  Z\", &
                                                                     \" XY\", \" XZ\", \" YZ\", &
                                                                     \"XYZ\"]
END MODULE m
";
    let (once, twice) = full_twice(source, &deep_wrapping_config());
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
}

#[test]
fn wrapping_measures_internal_whitespace_after_remred_before_searching_for_a_break() {
    let source = "\
module NoahmpIOVarInitMod
contains
  subroutine NoahmpIOVarInitDefault(NoahmpIO)
    associate(its => NoahmpIO%its, ite => NoahmpIO%ite)
    if (NoahmpIO%sf_urban_physics > 0) then
    if ( .not. allocated (NoahmpIO%trb_urb4d)  ) allocate ( NoahmpIO%trb_urb4d   (its:ite,NoahmpIO%urban_map_zrd) )
    endif
    end associate
  end subroutine NoahmpIOVarInitDefault
end module NoahmpIOVarInitMod
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        indent: 8,
        start_indent: 2,
        indent_continuation: true,
        continuation_indent: 6,
        indent_ampersand: true,
        align_paren: true,
        align_paren_value: 4,
        ws_remred: true,
        ws_remred_value: 1,
        align_declarations: true,
        align_comments: true,
        contains_restart: true,
        openmp: true,
        max_indent: 32,
        construct_indents: forformat::ConstructIndents::with_indent(8),
        contains_indent: 8,
        wrap: forformat::WrapConfig {
            line_length: 120,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let (once, twice) = full_twice(source, &config);
    assert!(
        once.lines()
            .any(|line| line.contains("trb_urb4d (its:ite, &")),
        "the remred-width guard was not exercised:\n{once}"
    );
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
}

#[test]
fn a_redundant_pair_is_still_redundant_when_the_author_broke_the_line_between_them() {
    // Reduced from Q-E `PP/src/write_hamiltonians.f90`.  The author closed this
    // condition on a continuation, so the inner `)` is followed by `&`, a
    // newline and only then the outer `)`.  Step 7 scanned raw bytes and looked
    // for the matching `)` across *whitespace* alone, so the continuation
    // marker hid the pair and the parentheses survived.  Wrapping then rejoined
    // the statement and broke it elsewhere, and the next run — now seeing the
    // two `)` adjacent — removed what the first had kept.
    let source = "\
subroutine p
   integer i
   IF( ((wan_in(i,1)%iatom .ne. wan_in(i+1,1)%iatom) .OR. wan_in(i,1)%ing(1)%l .ne. wan_in(i+1,1)%ing(1)%l) &
       ) THEN
      x = 1
   END IF
end subroutine p
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        ..FormatConfig::default()
    };
    let (once, twice) = full_twice(source, &config);
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
    assert!(
        !once.contains("if ((("),
        "the redundant pair survived the first pass:\n{once}"
    );
}

#[test]
fn continuation_transparency_does_not_widen_what_paren_removal_is_allowed_to_touch() {
    // Looking through a continuation marker also lets an assignment's `=` and a
    // condition's keyword reach parentheses opened on a later physical line.
    // The guards that make step 7 safe are the argument-list and
    // `ASSOCIATE`-target tests, and neither may be weakened by that: an extra
    // pair around an actual argument can carry intent, so it stays.
    let source = "\
subroutine p
  x = 1
  call foo(a=1, &
       ((b)) )
  y = 2; call bar(((c)) &
       )
  z = merge(((u)), ((v)), &
       ((w)) )
  associate (q => (a), &
       r => ((b)) )
    n = ((m) &
        )
  end associate
  write (*, *) ((k))
end subroutine p
";
    let (once, twice) = full_twice(source, &FormatConfig::default());
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
    for kept in ["((b))", "((c))", "((u))", "((v))", "((w))", "((k))"] {
        assert!(
            once.contains(kept),
            "a protected pair {kept} was removed:\n{once}"
        );
    }
    assert!(
        once.contains("n = (m"),
        "the eligible right-hand side kept its redundant pair:\n{once}"
    );
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn a_binary_minus_stays_spaced_across_a_conditional_sentinel_continuation() {
    // The `!$` sentinel is a layout prefix, not the end of the expression.
    // The wrapper moves the minus to the next sentinel line; the next pass
    // must still see the operand on the preceding physical line and retain
    // binary spacing.
    let mut style = StyleConfig::default();
    style.keyword_case = KeywordCase::Preserve;
    style.relational_symbols = false;
    style.array_brackets = false;
    style.compact_multiplicative = false;
    style.delimiter_spacing = false;
    style.comment_spacing = false;
    style.continuation_markers = false;
    let config = FormatConfig {
        mode: FormatMode::Full,
        indent: 8,
        construct_indents: forformat::ConstructIndents::with_indent(8),
        align_paren: true,
        align_paren_value: 4,
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        style,
        ..FormatConfig::default()
    };
    let source = "\
subroutine b1
  integer :: iatom, nkind, jatom, nlock, hash, natom
  if (.true.) then
    if (.true.) then
      if (.true.) then
        if (.true.) then
          if (.true.) then
            if (.true.) then
              if (.true.) then
                !$ hash = mod((iatom - 1)*natom + jatom, nlock) + 1
              end if
            end if
          end if
        end if
      end if
    end if
  end if
end subroutine b1
";
    let (once, twice) = full_twice(source, &config);
    assert!(
        once.contains("- 1)"),
        "the reduced break was not exercised:\n{once}"
    );
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn an_io_format_star_stays_spaced_when_wrapping_moves_it_before_a_marker() {
    // `PRINT *` uses `*` as its output unit.  Once wrapping puts the marker
    // after that star, a per-line multiplication test must not compact it.
    let mut style = StyleConfig::default();
    style.relational_symbols = true;
    style.array_brackets = true;
    style.compact_multiplicative = true;
    style.join_goto = true;
    style.split_compound_keywords = true;
    style.strip_empty_args = true;
    style.remove_redundant_parens = true;
    style.remove_terminal_return = true;
    style.program_unit_spacing = true;
    style.max_blank_lines = Some(2);
    style.delimiter_spacing = true;
    style.comment_spacing = true;
    style.continuation_markers = true;
    let config = FormatConfig {
        mode: FormatMode::Full,
        indent: 8,
        construct_indents: forformat::ConstructIndents::with_indent(8),
        continuation_indent: 6,
        align_paren: true,
        align_paren_value: 8,
        align_comments: true,
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        style,
        ..FormatConfig::default()
    };
    let source = "\
subroutine c1(ex)
  logical, optional :: ex
  if (.true.) then
    if (.true.) then
      if (.true.) then
        if (.true.) then
          if (.true.) then
            if ( .not.present(ex) ) &
              print *, 'error: closed tag that was not open'
          end if
        end if
      end if
    end if
  end if
end subroutine c1
";
    let (once, twice) = full_twice(source, &config);
    assert!(
        once.contains("print * &"),
        "the reduced break was not exercised:\n{once}"
    );
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn an_io_format_star_is_recognized_whatever_case_the_keyword_is_written_in() {
    // Fortran reserves no case, so `PRINT` under `--keyword-case=preserve` is
    // the same statement as `print` under `=lower`.  Matching the keyword's raw
    // bytes rather than comparing case-insensitively left the specifier rule
    // switched off for every source that spells its I/O keywords in upper case.
    let mut style = StyleConfig::default();
    style.keyword_case = KeywordCase::Preserve;
    style.compact_multiplicative = true;
    style.delimiter_spacing = true;
    style.continuation_markers = true;
    let config = FormatConfig {
        mode: FormatMode::Full,
        indent: 8,
        construct_indents: forformat::ConstructIndents::with_indent(8),
        continuation_indent: 6,
        align_paren: true,
        align_paren_value: 8,
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 80,
        },
        style,
        ..FormatConfig::default()
    };
    let source = "\
subroutine c1(ex)
  logical, optional :: ex
  if (.true.) then
    if (.true.) then
      if (.true.) then
        if (.true.) then
          if (.true.) then
            IF ( .not.present(ex) ) &
              PRINT *, 'error: closed tag that was not open'
          end if
        end if
      end if
    end if
  end if
end subroutine c1
";
    let (once, twice) = full_twice(source, &config);
    assert!(
        once.contains("PRINT * &"),
        "the reduced break was not exercised:\n{once}"
    );
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn a_bind_keyword_argument_stays_compact_when_its_call_head_wraps_away() {
    // `BIND(C,` opens the argument list on the preceding physical line.  The
    // `name=` token remains a keyword argument after the wrapper moves it to a
    // continuation, so it must not be respaced as an ordinary assignment.
    let config = FormatConfig {
        mode: FormatMode::Full,
        indent: 8,
        start_indent: 2,
        max_indent: 32,
        continuation_indent: 6,
        indent_ampersand: true,
        align_paren: true,
        align_paren_value: 4,
        ws_remred: true,
        ws_remred_value: 1,
        align_comments: true,
        contains_restart: true,
        construct_indents: forformat::ConstructIndents::with_indent(8),
        wrap: forformat::WrapConfig {
            enabled: true,
            line_length: 120,
        },
        ..FormatConfig::default()
    };
    let source = "\
module c2
  use iso_c_binding
  interface
    integer(C_INT) FUNCTION libvori_pushAtoms(n, pord, pchg, posx, posy, posz) BIND(C, NAME='libvori_pushAtoms')
      use iso_c_binding, only: c_int
      integer(c_int), value :: n
      integer(c_int) :: pord(*), pchg(*), posx(*), posy(*), posz(*)
    end function libvori_pushAtoms
  end interface
end module c2
";
    let (once, twice) = full_twice(source, &config);
    assert!(
        once.contains("name= &"),
        "the reduced break was not exercised:\n{once}"
    );
    assert_eq!(once, twice, "first pass:\n{once}\nsecond pass:\n{twice}");
}
