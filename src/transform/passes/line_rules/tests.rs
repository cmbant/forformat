#[cfg(test)]
mod tests {
    use crate::{
        analysis::{analyze_file, ProjectContext, ScopeTree},
        config::FormatConfig,
        format_source,
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
        FormatMode,
    };

    fn normalized(source: &[u8]) -> String {
        let mut document = Document::from_bytes(source);
        let project = ProjectContext::empty();
        let local = analyze_file(source).unwrap();
        let config = FormatConfig::default();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_ne!(
            super::run(&mut document, &context).unwrap(),
            Changed::Structure,
            "the per-line chain must not change the line count"
        );
        String::from_utf8_lossy(&document.to_bytes()).into_owned()
    }

    fn full_pipeline(source: &[u8]) -> String {
        let config = FormatConfig {
            mode: FormatMode::Full,
            apply_indent: false,
            ..FormatConfig::default()
        };
        String::from_utf8(format_source(source, &config).unwrap().bytes).unwrap()
    }

    #[test]
    fn keywords_are_lowercased_and_identifiers_are_not() {
        assert_eq!(
            normalized(
                b"PROGRAM Main\nIF (X > 1) THEN\nCALL DoThing(Arg)\nEND IF\nEND PROGRAM Main\n"
            ),
            "program Main\nif (X > 1) then\ncall DoThing(Arg)\nend if\nend program Main\n"
        );
    }

    #[test]
    fn adjacent_operators_are_padded_once_not_twice() {
        // Regression: `=` and `.not.` each padded their own side, because span
        // edits cannot see what a neighbouring edit already wrote.  The
        // the formatter emits exactly one space between them.
        assert_eq!(normalized(b"a = .not. b\n"), "a = .not. b\n");
        assert_eq!(normalized(b"a=.not.b\n"), "a = .not. b\n");
        assert_eq!(normalized(b"a =.not. b\n"), "a = .not. b\n");
        assert_eq!(normalized(b"a=b.and..not.c\n"), "a = b .and. .not. c\n");
        assert_eq!(normalized(b"if (a) c=.not.d\n"), "if (a) c = .not. d\n");
    }

    #[test]
    fn dotted_words_in_the_intrinsic_table_are_lowercased() {
        assert_eq!(
            normalized(b"x = .TRUE.\ny = .FALSE.\n"),
            "x = .true.\ny = .false.\n"
        );
        assert_eq!(normalized(b"a = .NOT. b\n"), "a = .not. b\n");
        assert_eq!(
            normalized(b"a = b .AND. .NOT. c\n"),
            "a = b .and. .not. c\n"
        );
        assert_eq!(normalized(b"z = a .MYOP. b\n"), "z = a .MYOP. b\n");
        assert_eq!(
            normalized(b"s = '.TRUE.' ! .TRUE.\n"),
            "s = '.TRUE.' ! .TRUE.\n"
        );
    }

    #[test]
    fn only_is_lowercased_even_after_a_double_colon() {
        assert_eq!(
            normalized(b"use, intrinsic :: iso_c_binding, ONLY: A\n"),
            "use, intrinsic :: iso_c_binding, only: A\n"
        );
        assert_eq!(normalized(b"use m, ONLY : A\n"), "use m, only: A\n");
    }

    #[test]
    fn adjacent_operator_padding_is_idempotent() {
        let once = normalized(b"a=.not.b\nx=y.and..not.z\n");
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn spaced_power_operator_stays_spaced_after_a_continuation() {
        let source = b"x = value &\n  ** 2\n";
        let mut config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            apply_indent: false,
            ..FormatConfig::default()
        };
        config.style.compact_multiplicative = false;
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice);
        assert!(once.windows(4).any(|window| window == b"** 2"));
    }

    #[test]
    fn spaced_relational_operator_stays_spaced_after_a_continuation() {
        let source = b"if (DefaultFalse(filename) .and. &\n  value /= '') value = replace(value)\n";
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            apply_indent: false,
            ..FormatConfig::default()
        };
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice);
        assert!(once.windows(5).any(|window| window == b"/= ''"));
    }

    #[test]
    fn a_named_argument_keeps_a_dotted_operator_against_its_equals() {
        assert_eq!(
            normalized(b"call f(a, append=.not. new_chains)\n"),
            "call f(a, append=.not. new_chains)\n"
        );
        assert_eq!(
            normalized(b"call f(a, append= .not. new_chains)\n"),
            "call f(a, append=.not. new_chains)\n"
        );
        assert_eq!(
            normalized(b"append=.not.new_chains\n"),
            "append = .not. new_chains\n"
        );
    }

    #[test]
    fn an_undeclared_name_that_is_a_keyword_is_lowercased() {
        assert_eq!(normalized(b"CALL sub(x)\nSTOP\n"), "call sub(x)\nstop\n");
    }

    #[test]
    fn context_sensitive_keywords_are_only_keywords_in_their_own_shape() {
        assert_eq!(
            normalized(b"real(dl) function f(a) BIND(C, name='exported')\n"),
            "real(dl) function f(a) bind(c, name='exported')\n"
        );
        assert_eq!(
            normalized(b"subroutine s() BIND(C)\n"),
            "subroutine s bind(c)\n"
        );
        assert_eq!(normalized(b"USE m, ONLY: x\n"), "use m, only: x\n");
        assert_eq!(normalized(b"x = ONLY + 1\n"), "x = ONLY + 1\n");
        assert_eq!(
            normalized(b"DOUBLE PRECISION :: y\n"),
            "double precision :: y\n"
        );
        assert_eq!(normalized(b"z = PRECISION\n"), "z = PRECISION\n");
        assert_eq!(
            normalized(b"integer(KIND=4) :: n\n"),
            "integer(kind=4) :: n\n"
        );
        assert_eq!(
            normalized(b"integer, POINTER :: p\n"),
            "integer, pointer :: p\n"
        );
        assert_eq!(normalized(b"call sub(POINTER)\n"), "call sub(POINTER)\n");
    }

    #[test]
    fn bind_c_marker_is_syntax_even_when_c_is_a_local_name() {
        assert_eq!(
            normalized(
                b"subroutine s(C) BIND(C, NAME='entry')\ninteger :: C\nprint *, C\nend subroutine s\n"
            ),
            "subroutine s(C) bind(c, name='entry')\ninteger :: C\nprint *, C\nend subroutine s\n"
        );
        assert_eq!(
            normalized(
                b"subroutine t(C)\ninteger :: C\nexternal BIND\ncall BIND(C)\nend subroutine t\n"
            ),
            "subroutine t(C)\ninteger :: C\nexternal BIND\ncall BIND(C)\nend subroutine t\n"
        );
    }

    #[test]
    fn a_declared_name_that_collides_with_a_keyword_is_left_alone() {
        let source = b"module M\ntype :: Data\nend type Data\nend module M\n";
        assert_eq!(
            normalized(source),
            "module M\ntype :: Data\nend type Data\nend module M\n"
        );
    }

    #[test]
    fn string_literals_and_comments_keep_their_case() {
        assert_eq!(
            normalized(b"CALL sub('IF THEN END')  ! IF THEN END\n"),
            "call sub('IF THEN END')  ! IF THEN END\n"
        );
    }

    #[test]
    fn a_component_after_percent_is_not_a_keyword() {
        assert_eq!(normalized(b"X = State%Data\n"), "X = State%Data\n");
    }

    #[test]
    fn preprocessor_lines_are_preserved_byte_for_byte() {
        assert_eq!(
            normalized(b"#define IF_THING 1\n#if defined(IF_THING)\nCALL X\n#endif\n"),
            "#define IF_THING 1\n#if defined(IF_THING)\ncall X\n#endif\n"
        );
    }

    #[test]
    fn a_literal_continued_across_lines_is_not_reinterpreted_as_code() {
        assert_eq!(
            normalized(b"x = 'THEN END &\n  IF' // Y\n"),
            "x = 'THEN END &\n  IF' // Y\n"
        );
    }

    #[test]
    fn keyword_and_delimiter_rules_match_expected_shapes() {
        let source = b"ENDIF\n\
ELSEIF  ( X )\n\
BLOCKDATA\n\
GO   TO 10\n\
DOUBLE   PRECISION :: X\n\
IF( X )THEN\n\
SELECT   TYPE   IS   ( X )\n\
DO    WHILE( X )\n\
COMMON / blk / x\n\
SUBROUTINE s( )\n\
x = (/ 1 , 2 /)\n\
FORMAT((/ 1, 2 /))\n\
WRITE( UNIT = 1 , FMT = 2 )'x'\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "end if\nelse if (X)\nblock data\ngoto 10\ndouble precision :: X\nif (X) then\nselect type is (X)\ndo while (X)\ncommon /blk/ x\nsubroutine s\nx = [1, 2]\nformat((/1, 2 /))\nwrite(unit=1, fmt=2) 'x'\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn keyword_to_name_gaps_collapse_to_one_space() {
        let source = b"module   mymod\n\
use   mymod\n\
call   foo(x)\n\
subroutine   do_thing\n\
end subroutine   do_thing\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "module mymod\nuse mymod\ncall foo(x)\nsubroutine do_thing\nend subroutine do_thing\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn concatenation_spacing_survives_a_continuation_line() {
        let source = b"call MpiStop('SP(k) cannot be combined with HMCode_A_baryon/' &\n\
    // 'HMCode_eta_baryon baryonic corrections in HMCode 2015/2016')\n";
        assert_eq!(normalized(source), String::from_utf8_lossy(source));
    }

    #[test]
    fn go_to_is_compacted_after_a_continuation_join() {
        let source = b"GO &\n  TO 10\n";
        let document = Document::from_bytes(source);
        let project = ProjectContext::empty();
        let local = analyze_file(source).unwrap();
        let config = FormatConfig::default();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        let declared_names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        assert_eq!(
            super::respace_joined(b"GO TO 10", &context, &declared_names, 0),
            b"goto 10"
        );
    }

    #[test]
    fn post_f2008_keywords_are_lowercased_and_spaced() {
        let source = b"IMPURE  ELEMENTAL FUNCTION f(x)\n\
PURE   ELEMENTAL SUBROUTINE s\n\
CONTIGUOUS :: x\n\
CRITICAL(STAT = istat)\n\
CHANGE   TEAM(newteam)\n\
SELECT  RANK(a)\n\
RANK  DEFAULT\n\
FORM  TEAM(n, team, STAT=istat)\n\
SYNC  ALL(STAT=istat)\n\
SYNC   TEAM(team)\n\
EVENT  POST(event)\n\
EVENT WAIT(event, UNTIL_COUNT =n)\n\
FAIL  IMAGE\n\
LOCK(lockvar, ACQUIRED_LOCK = acquired)\n\
UNLOCK(lockvar)\n\
DO  CONCURRENT(i=1:n) LOCAL_INIT(x) SHARED(y) REDUCE(+:z)\n";
        assert_eq!(
            normalized(source),
            "impure elemental function f(x)\n\
pure elemental subroutine s\n\
contiguous :: x\n\
critical(stat=istat)\n\
change team (newteam)\n\
select rank (a)\n\
rank default\n\
form team (n, team, stat=istat)\n\
sync all(stat=istat)\n\
sync team (team)\n\
event post(event)\n\
event wait(event, until_count=n)\n\
fail image\n\
lock(lockvar, acquired_lock=acquired)\n\
unlock(lockvar)\n\
do concurrent(i=1:n) local_init(x) shared(y) reduce(+:z)\n"
        );
    }

    #[test]
    fn chunk_a_operators_exponents_and_comments_are_narrow() {
        let source =
            b"x=1.0E-3+Y.eq.-1\n! x%value= a.eq.b + 2\nCALL sub('IF ( X )', A , B ) !keep\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "x = 1.0e-3 + Y == -1\n! x%value = a == b + 2\ncall sub('IF ( X )', A, B) ! keep\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn declaration_spacing_and_intrinsic_case_respect_names() {
        assert_eq!(
            normalized(b"INTEGER, OPTIONAL, INTENT(IN) :: X\n"),
            "integer, intent(in), optional:: X\n"
        );
        assert_eq!(
            normalized(b"REAL(KIND=8)X,Y\nX = SIZE + SQRT(Y)\n"),
            "real(kind=8) X, Y\nX = size + sqrt(Y)\n"
        );
        assert_eq!(
            normalized(b"SUBROUTINE s(Write)\nX = Write ( 1 )\nEND SUBROUTINE s\n"),
            "subroutine s(Write)\nX = Write (1)\nend subroutine s\n"
        );
    }

    #[test]
    fn dimension_and_write_output_spacing_matches_expected_shape() {
        let source = b"integer, dimension (:) :: values\nwrite(*, *)'Warning...'\nwrite(unit, '(1I6,4E15.6)')il, value\nwrite(unit, '(1I6,4E15.6)')\nwrite(unit, '(1I6,4E15.6)') &\nwrite(unit, '(1I6,4E15.6)' ) ! no output item\nprint *, \"write(*)'literal'\"\n! write(*)'comment'\n";
        assert_eq!(
            normalized(source),
            "integer, dimension(:) :: values\nwrite(*, *) 'Warning...'\nwrite(unit, '(1I6,4E15.6)') il, value\nwrite(unit, '(1I6,4E15.6)')\nwrite(unit, '(1I6,4E15.6)') &\nwrite(unit, '(1I6,4E15.6)' ) ! no output item\nprint *, \"write(*)'literal'\"\n! write(*)'comment'\n"
        );
    }

    #[test]
    fn parenthesized_statements_lowercase_unless_locally_shadowed() {
        let source = b"WRITE (*, *) value\nREAD (unit, *) value\nOPEN (newunit=unit, file=name)\nBACKSPACE (unit)\nALLOCATED (value)\nC%Write (*, *) value\nsubroutine s\nprocedure :: Write\ncall WRITE()\nend subroutine s\n";
        assert_eq!(
            full_pipeline(source),
            "write(*, *) value\nread(unit, *) value\nopen(newunit=unit, file=name)\nbackspace(unit)\nallocated(value)\nC%Write(*, *) value\nsubroutine s\nprocedure :: Write\ncall Write()\n\nend subroutine s\n"
        );
    }

    #[test]
    fn old_style_declarations_normalize_spacing_and_optional_order() {
        let source = b"    real(dl)  x\n    real(dl)kh, PK\n    real(dp), optional, intent(out) :: sin_k\n    real(dp), intent(in), optional :: cos_k\n";
        assert_eq!(
            full_pipeline(source),
            "    real(dl) x\n    real(dl) kh, PK\n    real(dp), intent(out), optional :: sin_k\n    real(dp), intent(in), optional :: cos_k\n"
        );
    }

    #[test]
    fn a_local_intrinsic_name_is_scoped_to_its_own_procedure() {
        let source = b"SUBROUTINE first()\nINTEGER :: SIZE\nX = SIZE\nEND SUBROUTINE first\n\
SUBROUTINE second()\nX = SIZE\nEND SUBROUTINE second\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "subroutine first\ninteger :: SIZE\nX = SIZE\nend subroutine first\n\
subroutine second\nX = size\nend subroutine second\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn module_declared_names_are_visible_inside_contained_procedures_only() {
        let source = b"MODULE m\nINTEGER :: STATUS\nCONTAINS\nSUBROUTINE s()\nX = STATUS\nEND SUBROUTINE s\nEND MODULE m\n\
X = STATUS\n";
        assert_eq!(
            normalized(source),
            "module m\ninteger :: STATUS\ncontains\nsubroutine s\nX = STATUS\nend subroutine s\nend module m\n\
X = status\n"
        );
    }

    #[test]
    fn a_procedure_name_from_one_module_does_not_shadow_an_intrinsic_in_another() {
        let source =
            b"MODULE a\nCONTAINS\nFUNCTION SIZE()\nSIZE = 1\nEND FUNCTION SIZE\nEND MODULE a\n\
MODULE b\nX = SIZE(1)\nEND MODULE b\n";
        assert_eq!(
            normalized(source),
            "module a\ncontains\nfunction SIZE()\nSIZE = 1\nend function SIZE\nend module a\n\
module b\nX = size(1)\nend module b\n"
        );
    }

    #[test]
    fn local_and_file_names_have_different_keyword_argument_rules() {
        let local =
            normalized(b"SUBROUTINE s(STATUS)\nCALL f(x, STATUS=STATUS)\nEND SUBROUTINE s\n");
        assert_eq!(
            local,
            "subroutine s(STATUS)\ncall f(x, STATUS=STATUS)\nend subroutine s\n"
        );

        let file = normalized(
            b"MODULE m\nINTEGER :: STATUS\nCONTAINS\nSUBROUTINE s()\nCALL f(x, STATUS=STATUS)\nEND SUBROUTINE s\nEND MODULE m\n",
        );
        assert_eq!(
            file,
            "module m\ninteger :: STATUS\ncontains\nsubroutine s\ncall f(x, status=STATUS)\nend subroutine s\nend module m\n"
        );
    }

    #[test]
    fn dollar_sentinel_clause_bodies_follow_fortran_normalization() {
        assert_eq!(normalized(b"!$ USE OMP_LIB\n"), "!$ use OMP_LIB\n");
        assert_eq!(
            normalized(b"!$ IF(X.EQ.1) CALL F( A , B )\n"),
            "!$ if (X == 1) call F(A, B)\n"
        );
    }

    #[test]
    fn dollar_sentinel_boundaries_and_protected_text_are_preserved() {
        let source = b"! USE OMP_LIB\n!$OMP IF(X.EQ.1) CALL F( A , B )\n!$\n  !$ USE OMP_LIB\n!$ CALL F('IF THEN', A)\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "! USE OMP_LIB\n!$OMP IF(X.EQ.1) CALL F( A , B )\n!$\n  !$ use OMP_LIB\n!$ call F('IF THEN', A)\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn contextual_declaration_names_reset_after_top_level_initializers() {
        assert_eq!(
            normalized(b"INTEGER :: A = 1, SIZE\n"),
            "integer :: A = 1, SIZE\n"
        );
    }

    #[test]
    fn contextual_declaration_initializer_scan_sees_nested_equals() {
        assert_eq!(
            normalized(b"REAL :: X(F(N=1) + SIZE)\n"),
            "real :: X(F(N=1) + size)\n"
        );
    }

    #[test]
    fn uppercase_single_l_is_opt_in_and_protected_bytes_are_untouched() {
        let source = b"x = l + 'l' ! l\n#define L 1\n";
        let mut document = Document::from_bytes(source);
        let project = ProjectContext::empty();
        let local = analyze_file(source).unwrap();
        let config = FormatConfig {
            uppercase_single_l: true,
            ..FormatConfig::default()
        };
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        super::run(&mut document, &context).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&document.to_bytes()),
            "x = L + 'l' ! l\n#define L 1\n"
        );
    }

    #[test]
    fn joined_named_arguments_keep_compact_equals() {
        assert_eq!(
            super::compact_joined_named_arguments(
                b"call compute(alpha, nested(first, second), named = value)"
            ),
            b"call compute(alpha, nested(first, second), named=value)"
        );
    }
}
