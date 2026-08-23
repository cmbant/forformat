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
