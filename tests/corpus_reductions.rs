//! Reduced corpus cases from `docs/corpus-reductions.md`.
//!
//! Each test is the smallest source that reproduced a corpus observation, kept
//! next to the fix so the shape cannot regress without a named failure.

use forformat::{
    analyze_project, format_source, format_source_with_context, ConstructIndents, FormatConfig,
};
use std::path::Path;

fn full(source: &[u8]) -> String {
    String::from_utf8(
        format_source(source, &FormatConfig::default())
            .expect("source formats")
            .bytes,
    )
    .expect("output is UTF-8")
}

/// ABINIT `m_polynomial_coeff.F90`: a lowercase use of the type elsewhere in
/// the file made the type name ambiguous, so the first pass left the named END
/// alone and the second pass — reading a file whose use site had been
/// canonicalized — moved it.
#[test]
fn a_named_end_follows_its_own_header_spelling() {
    let source = b"module m\ntype :: t_Name\nend type t_NAME\ntype(t_name) :: v\nend module m\n";
    let output = full(source);

    assert!(output.contains("end type t_Name"), "{output}");
    assert_eq!(full(output.as_bytes()), output);
}

/// ABINIT `m_linalg_interfaces.F90`: the interface header took the project's
/// spelling of an external procedure while its named END kept the authored one,
/// and only the next pass caught up.
#[test]
fn a_named_end_follows_a_project_resolved_header() {
    let definition = b"COMPLEX FUNCTION CDOTC(N, CX, INCX)\nCOMPLEX CX(*)\nCDOTC = (0.0, 0.0)\nEND function CDOTC\n";
    let target =
        b"module m\ninterface\ncomplex function cdotc(n, cx, incx)\ninteger :: n\nend function cdotc\nend interface\nend module m\n";
    let project = analyze_project([
        (Path::new("cdotc.f90"), definition.as_slice()),
        (Path::new("target.f90"), target.as_slice()),
    ])
    .expect("project analyzes");
    let output = String::from_utf8(
        format_source_with_context(target, &project, &FormatConfig::default())
            .expect("target formats")
            .bytes,
    )
    .expect("output is UTF-8");

    assert!(output.contains("complex function CDOTC("), "{output}");
    assert!(output.contains("end function CDOTC"), "{output}");
}

/// WRF `module_diffusion_em_ad.F`: a declaration that carries its `::` on a
/// later physical line is not the old-style declaration its first line looks
/// like, and must not have that line's blanks squeezed. Doing so made the gap
/// before an array-range colon depend on which physical line the wrapper had
/// put it on.
#[test]
fn a_declaration_continued_before_its_separator_keeps_authored_blanks() {
    let source =
        b"subroutine p\nreal, dimension(min0(a, b)  :max0(c, d), &\nmin0(e, f)  :max0(g, h)) :: x\nend subroutine p\n";
    let output = full(source);

    assert_eq!(output.matches("  :max0").count(), 2, "{output}");
    // An old-style declaration — one with no `::` at all — still gets squeezed.
    assert!(
        full(b"subroutine p\nreal  a(3),  b(4)\nend subroutine p\n").contains("real a(3), b(4)")
    );
}

/// The same declaration at WRF's `--indent=8` depth, where the reflow moves the
/// colon onto the statement's first physical line and back.
#[test]
fn a_deeply_indented_continued_declaration_is_a_fixed_point() {
    let source = b"module m\ncontains\nsubroutine p\n   REAL,DIMENSION(min0(1,its):max0(n_nba_mij,min(ite, ide-1)),min0(jms,kts) &\n   :max0(jme,kte-1),min0(kms,jts):max0(kme,min(jte, jde-1)),min0(ims,PARAM_FIRST_SCALAR) &\n   :max0(ime,n_moist)) :: Tmpv500\nend subroutine p\nend module m\n";
    let config = FormatConfig {
        indent: 8,
        construct_indents: ConstructIndents::with_indent(8),
        contains_indent: 8,
        continuation_indent: 8,
        case_indent: 4,
        entry_indent: 4,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config)
        .expect("source formats")
        .bytes;
    let twice = format_source(&once, &config).expect("output formats").bytes;

    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice)
    );
}

/// WRF `phys/module_surface_driver.F`: an attribute is an attribute because a
/// `::` closes the attribute half of the declaration, and that `::` need not be
/// on the physical line the attribute sits on.
#[test]
fn attributes_are_keywords_before_a_separator_on_a_later_line() {
    let source = b"module m\ncontains\nsubroutine p(a, b)\nREAL, OPTIONAL, DIMENSION(ims:ime), &\nINTENT(IN) :: a\nREAL, ALLOCATABLE, TARGET, &\nSAVE :: b(:)\nend subroutine p\nend module m\n";
    let output = full(source);

    assert!(
        output.contains("real, optional, dimension(ims:ime), &"),
        "{output}"
    );
    assert!(output.contains("intent(in) :: a"), "{output}");
    assert!(output.contains("real, allocatable, target, &"), "{output}");
    assert!(output.contains("save :: b(:)"), "{output}");
    assert_eq!(full(output.as_bytes()), output);
}

/// A physical line carries one owning scope — the one in force before its
/// statements were read — so the second of two `;`-separated nested ENDs sees
/// the scope the first one closed. Reported against the named-END pass above:
/// the outer END matched no scope of its kind and was left to the case tables,
/// which is exactly what that pass exists to stop.
#[test]
fn a_named_end_after_a_separator_still_follows_its_own_header() {
    let source = b"module m\ninterface Blas_Dot\ncomplex function cdotc(n)\ninteger :: n\nend function cdotc; end interface BLAS_DOT\ncontains\nsubroutine s(n)\ninteger :: n\nwrite(*,*) blas_dot(n)\nend subroutine s\nend module m\n";
    let output = full(source);

    assert!(
        output.contains("end function cdotc; end interface Blas_Dot"),
        "{output}"
    );
    assert_eq!(full(output.as_bytes()), output);
}

/// CP2K `xc_perdew_wang.F`: wrapping an OpenMP continuation at a narrow line
/// length must not make the local dummy `order` look like the OpenMP keyword
/// `ORDER` on the next pass.
#[test]
fn a_wrapped_openmp_continuation_keeps_local_order_case() {
    let source = include_bytes!("fixtures/corpus_openmp_order_case.f90");
    let config = FormatConfig {
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..Default::default()
        },
        ..FormatConfig::default()
    };
    let once = format_source(source, &config)
        .expect("source formats")
        .bytes;
    let twice = format_source(&once, &config)
        .expect("first output formats")
        .bytes;
    assert_eq!(once, twice, "{0}", String::from_utf8_lossy(&twice));
    assert!(
        String::from_utf8_lossy(&once).contains("order)"),
        "{}",
        String::from_utf8_lossy(&once)
    );
}

/// Style profile at the eight-column indentation the CP2K and WRF reports use.
fn deep_indent(style: forformat::StyleConfig) -> FormatConfig {
    FormatConfig {
        style,
        indent: 8,
        construct_indents: ConstructIndents::with_indent(8),
        contains_indent: 8,
        continuation_indent: 8,
        case_indent: 4,
        entry_indent: 4,
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..Default::default()
        },
        ..FormatConfig::default()
    }
}

fn format(source: &[u8], config: &FormatConfig) -> String {
    String::from_utf8(format_source(source, config).expect("source formats").bytes)
        .expect("output is UTF-8")
}

fn fixed_point(source: &[u8], config: &FormatConfig) -> String {
    let once = format(source, config);
    assert_eq!(format(once.as_bytes(), config), once, "{once}");
    once
}

fn upper() -> FormatConfig {
    FormatConfig {
        style: forformat::StyleConfig {
            keyword_case: forformat::KeywordCase::Upper,
            ..forformat::StyleConfig::default()
        },
        ..FormatConfig::default()
    }
}

/// specfem3d `numerical_recipes.f90`: a procedure defined at file level has no
/// enclosing module to register its name, so only its header — the one
/// occurrence followed by `(` — was read as the intrinsic it spells. The header
/// went uppercase while the body and the END kept the declared spelling, and
/// the next pass propagated the header's.
#[test]
fn an_external_procedure_name_is_not_the_intrinsic_it_spells() {
    let output = fixed_point(
        b"real function erf(x)\nreal :: x\nerf = x\nend function erf\n",
        &upper(),
    );

    assert!(output.contains("function erf(x)"), "{output}");
    assert!(output.contains("erf = x"), "{output}");
    assert!(output.contains("END FUNCTION erf"), "{output}");
}

/// ABINIT `m_xg.F90`: `=>` ends a declaration's entity name and opens an
/// ordinary expression, so `null()` there is the intrinsic. The lexer spells
/// `=>` as one token, which the entity-name scan's byte test never matched, so
/// the initializer was read as part of the name and kept its authored case —
/// but only on the physical lines where the scan reached that far.
#[test]
fn a_pointer_initialization_is_cased_as_an_expression() {
    let output = fixed_point(
        b"module m\ntype :: t\ninteger, pointer :: p => NULL()\ninteger, pointer :: q => Null()\nend type t\nend module m\n",
        &FormatConfig::default(),
    );

    assert_eq!(output.matches("=> null()").count(), 2, "{output}");
}

/// CP2K `qs_kinetic.F`: the emitter writes `!$ ` in the indent's first columns,
/// so measuring the joined form against the raw budget charged them twice. The
/// call below fits in 78 columns; the wrapper broke it anyway and produced a
/// continuation of 81, which the next pass joined again.
#[test]
fn a_conditional_sentinel_is_not_charged_to_the_indent_twice() {
    let source = b"MODULE m\nCONTAINS\nSUBROUTINE p(n, hash, locks)\nINTEGER :: n, hash, locks(*), i, j, k, l\nDO i = 1, n\nDO j = 1, n\nDO k = 1, n\nDO l = 1, n\n!$ CALL omp_set_lock(locks(hash))\nEND DO\nEND DO\nEND DO\nEND DO\nEND SUBROUTINE p\nEND MODULE m\n";
    let output = fixed_point(source, &deep_indent(forformat::StyleConfig::default()));

    assert!(
        output.contains("call omp_set_lock(locks(hash))"),
        "{output}"
    );
    let sentinel = output
        .lines()
        .find(|line| line.starts_with("!$"))
        .expect("a sentinel line");
    assert!(!sentinel.ends_with('&'), "{output}");
    assert!(sentinel.len() <= 80, "{} columns: {output}", sentinel.len());
}

/// Wannier90 `berry.F90` declares an `INTEGER :: if`, so the declared-case pass
/// settles every free-standing `if` in it as `if`. Casing the whole `ELSE IF`
/// replacement wrote `ELSE IF` and left that pass to rewrite it next run.
#[test]
fn a_compound_keyword_split_follows_a_declared_name() {
    let source = b"subroutine p(a, b)\nlogical :: a, b\ninteger :: if\ndo if = 1, 3\nif (a) then\nwrite(*,*) if\nelseif (b) then\nwrite(*,*) 2\nend if\nend do\nend subroutine p\n";
    let output = fixed_point(source, &upper());

    assert!(output.contains("ELSE if (b) THEN"), "{output}");
    // A file that declares nothing by that name still gets the keyword.
    let plain = fixed_point(
        b"subroutine p(a)\nlogical :: a\nif (a) then\nelseif (a) then\nend if\nend subroutine p\n",
        &upper(),
    );
    assert!(plain.contains("ELSE IF (a) THEN"), "{plain}");
}

/// WRF `module_sf_ruclsm.F`: `IF (...) PRINT *, …` is one statement, and the
/// wrapper breaks it inside the condition. The continuation then opens
/// mid-condition with no `PRINT` head in sight, so its `*` read as a
/// multiplication and was compacted on the pass after the one that spaced it.
#[test]
fn an_io_unit_star_survives_a_break_in_its_if_condition() {
    let source = b"MODULE m\nCONTAINS\nSUBROUTINE p(i, j, ktau, z0, znt)\nINTEGER :: i, j, ktau\nREAL :: z0(:, :), znt(:, :)\nDO ii = 1, 3\nDO jj = 1, 3\nIF (wrf_at_debug_level(lvl)) THEN\nif(ktau.eq.1 .and.(i.eq.358.and.j.eq.260)) &\nprint *,'before soilvegin - z0,znt(195,254)',z0(i,j),znt(i,j)\nENDIF\nEND DO\nEND DO\nEND SUBROUTINE p\nEND MODULE m\n";
    // Parenthesis alignment is what pushes the break into the condition here,
    // exactly as `full-layout-edge` does on the corpus file.
    let mut config = deep_indent(forformat::StyleConfig::default());
    config.align_paren = true;
    config.align_paren_value = 8;
    let output = fixed_point(source, &config);

    assert!(output.contains("print *"), "{output}");
    assert!(!output.contains("print*"), "{output}");
}

/// WRF `module_HYDRO_io.F90`: a blank line between a `&` and the line it
/// continues belongs to that statement. `--max-blank-lines=0` dropped it after
/// the wrapper had already declined the group, so the next run saw a statement
/// it could wrap and did.
#[test]
fn a_blank_line_inside_a_continued_statement_is_not_a_separator() {
    let source = b"subroutine p(did, ncid)\ninteger :: did, ncid\nif( nlst(did)%channel_only .eq. 0 .and. &\nnlst(did)%channelBucket_only .eq. 0 ) &\n\ncall w_rst_rt_nc2(ncid,rt_domain(did)%ixrt,rt_domain(did)%jxrt,rt_domain(did)%overland%streams_and_lakes%surface_water_to_lake,\"lake_inflort\")\nend subroutine p\n";
    let config = FormatConfig {
        style: forformat::StyleConfig {
            max_blank_lines: Some(0),
            ..forformat::StyleConfig::default()
        },
        ..FormatConfig::default()
    };
    let output = fixed_point(source, &config);

    // The blank stays because it is inside the statement, not between two.
    assert!(output.contains("== 0) &\n\n"), "{output}");
}

/// ABINIT `m_bessel.F90`: a `DATA` statement's slashes delimit its value lists,
/// and a data-stmt-constant is a literal rather than an expression. Reading
/// them as divisions made the spacing depend on what the wrapper had left
/// beside each slash, so a reflow changed it.
#[test]
fn a_data_statement_slash_is_not_a_division() {
    let output = fixed_point(
        b"subroutine p\ndouble precision eps, lda\ndata eps / -1.d0 /\ndata lda/2/\nend subroutine p\n",
        &FormatConfig::default(),
    );

    assert!(output.contains("data eps / -1.d0 /"), "{output}");
    assert!(output.contains("data lda/2/"), "{output}");
    // A real division is still spaced by the active policy.
    assert!(fixed_point(
        b"subroutine p\nx = a/b\nend subroutine p\n",
        &FormatConfig::default()
    )
    .contains("x = a/b"));
}

/// A main program's name is global, not a name declared in its own scope, so
/// registering it suppressed casing that should have applied: under
/// `program data`, the `DATA` keyword of a data statement stayed lowercase.
/// A procedure's name still registers — that is what
/// [`an_external_procedure_name_is_not_the_intrinsic_it_spells`] needs.
#[test]
fn a_main_program_name_does_not_shadow_the_keyword_it_spells() {
    let output = fixed_point(
        b"program data\ninteger :: eight\ndata eight /8/\nprint *, eight\nend program data\n",
        &upper(),
    );

    assert!(output.contains("DATA eight /8/"), "{output}");

    // The same for an intrinsic a main program happens to be named after.
    let output = fixed_point(
        b"program erf\nreal :: x, y\nx = 1.0\ny = erf(x)\nend program erf\n",
        &upper(),
    );

    assert!(output.contains("y = ERF(x)"), "{output}");
}

/// Fortran keywords are not reserved, so `data` at the head of a statement is
/// not proof of a `DATA` statement: `data = a/b` assigns to a variable, and
/// its slash is an ordinary division that the active policy still spaces.
#[test]
fn a_variable_named_data_is_not_a_data_statement() {
    let output = fixed_point(
        b"subroutine p(a, b, data, i)\nreal :: a, b, data(10)\ninteger :: i\ndata = a / b\ndata(i) = a / b\ndata(1) = a / b + data(2) / a\nend subroutine p\n",
        &FormatConfig::default(),
    );

    assert!(output.contains("data = a/b"), "{output}");
    assert!(output.contains("data(i) = a/b"), "{output}");
    assert!(output.contains("data(1) = a/b + data(2)/a"), "{output}");

    // An implied-do control's `=` sits inside the implied-do, so it does not
    // make a real `DATA` statement look like an assignment.
    let output = fixed_point(
        b"subroutine p\ninteger :: k(3), i\ndata (k(i), i=1,3) /1, 2, 3/\nend subroutine p\n",
        &FormatConfig::default(),
    );

    assert!(output.contains("/1, 2, 3/"), "{output}");
}

/// A comment between a `&` and the line it continues leaves the statement as
/// open as it found it. Recomputing the continuation flag from the comment
/// cleared it, so the blank after it was capped away — the same statement, and
/// the same mistake, as
/// [`a_blank_line_inside_a_continued_statement_is_not_a_separator`].
#[test]
fn a_comment_inside_a_continued_statement_does_not_close_it() {
    let config = FormatConfig {
        style: forformat::StyleConfig {
            max_blank_lines: Some(0),
            ..forformat::StyleConfig::default()
        },
        ..FormatConfig::default()
    };
    let output = fixed_point(
        b"subroutine p(alpha, beta, gamma, delta)\nreal :: alpha, beta, gamma, delta\nalpha = beta + &\n! why this term is here\n\ngamma + delta\nend subroutine p\n",
        &config,
    );

    assert!(output.contains("here\n\n"), "{output}");

    // A blank between two statements is still a separator, and still capped.
    let output = fixed_point(
        b"subroutine p(a, b)\nreal :: a, b\na = 1.0\n! an unrelated remark\n\nb = 2.0\nend subroutine p\n",
        &config,
    );

    assert!(output.contains("remark\n   b = 2.0"), "{output}");
}
