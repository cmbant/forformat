use forformat::{
    analysis::scope::ScopeTree,
    classify::{classify, StatementClass, StatementKind},
    format_source,
    transform::{document::Document, vocab, vocab_2023},
    FormatConfig, FormatMode,
};

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
fn fortran_2023_words_specifiers_and_intrinsics_normalize() {
    let source = b"program p\n\
real :: x, a(2)\n\
integer :: n\n\
logical :: flag\n\
TYPEOF(x) :: y\n\
CLASSOF(x) :: z\n\
INQUIRE (IOLENGTH = n) x\n\
INQUIRE (UNIT = 10, NAMED = flag)\n\
x = SUM(a) + COUNT(a > 0) + ACOSPI(x)\n\
n = SELECTED_LOGICAL_KIND(8)\n\
CALL RANDOM_SEED\n\
IF (flag) CALL RANDOM_SEED\n\
x = .NIL.\n\
ENUMERATION     TYPE colour\n\
END     ENUMERATION      TYPE colour\n\
NOTIFY      WAIT(flag)\n\
end program p\n";

    let once = normalize(source);
    let twice = normalize(&once);
    assert_eq!(twice, once);

    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("typeof(x) :: y"));
    assert!(output.contains("classof(x) :: z"));
    assert!(output.contains("inquire(iolength=n) x"));
    assert!(output.contains("inquire(unit=10, named=flag)"));
    assert!(output.contains("x = sum(a) + count(a > 0) + acospi(x)"));
    assert!(output.contains("n = selected_logical_kind(8)"));
    assert!(output.contains("call random_seed"));
    assert!(output.contains("if (flag) call random_seed"));
    assert!(output.contains("x = .nil."));
    assert!(output.contains("enumeration type colour"));
    assert!(output.contains("end enumeration type colour"));
    assert!(output.contains("notify wait(flag)"));
}

#[test]
fn enumeration_type_is_its_own_structural_frame() {
    let open = classify(b"ENUMERATION TYPE :: colour");
    assert_eq!(open.kind, StatementKind::Enum);
    assert_eq!(open.class, StatementClass::Definition);

    let close = classify(b"END ENUMERATION TYPE colour");
    assert_eq!(close.kind, StatementKind::EndEnum);
    assert_eq!(close.class, StatementClass::Neutral);

    // These two optional-blank forms also need structural meanings that differ
    // from the generic ELSE / END fallbacks.
    assert_eq!(classify(b"ELSE WHERE").kind, StatementKind::ElseWhere);
    assert_eq!(classify(b"END FILE").class, StatementClass::Neutral);

    // The wrapper must not steal ordinary assignments whose operands happen
    // to spell the same words.
    assert_eq!(
        classify(b"enumeration = type").kind,
        StatementKind::Unknown
    );
    assert_eq!(classify(b"end = file").class, StatementClass::Executable);

    let source = b"module m\n\
enumeration type :: colour\n\
enumerator :: red, green\n\
end enumeration type colour\n\
contains\n\
subroutine s\n\
end subroutine s\n\
end module m\n";

    let analysis = Document::from_bytes(source).analyze().unwrap();
    let tree = ScopeTree::build(&analysis);
    assert_eq!(
        tree.program_unit_of_line(4).unwrap().name.as_deref(),
        Some(b"m".as_slice())
    );
    assert_eq!(
        tree.program_unit_of_line(5).unwrap().name.as_deref(),
        Some(b"s".as_slice())
    );

    let indented = String::from_utf8(
        format_source(
            source,
            &FormatConfig {
                mode: FormatMode::IndentOnly,
                ..FormatConfig::default()
            },
        )
        .unwrap()
        .bytes,
    )
    .unwrap();
    assert!(indented.contains("\n   enumeration type :: colour\n"));
    assert!(indented.contains("\n      enumerator :: red, green\n"));
    assert!(indented.contains("\n   end enumeration type colour\n"));
    assert!(indented.contains("\n   subroutine s\n"));
}

#[test]
fn optional_blank_compound_table_is_exhaustively_covered() {
    // Fortran 2023 Table 6.2. GO TO has its dedicated join policy and IN OUT
    // is an INTENT specifier; every other joined spelling is a statement-head
    // compound and must be present in either the legacy or supplemental table.
    let forms = [
        ("blockdata", "block data"),
        ("doubleprecision", "double precision"),
        ("elseif", "else if"),
        ("elsewhere", "else where"),
        ("endassociate", "end associate"),
        ("endblock", "end block"),
        ("endblockdata", "end block data"),
        ("endcritical", "end critical"),
        ("enddo", "end do"),
        ("endenum", "end enum"),
        ("endfile", "end file"),
        ("endforall", "end forall"),
        ("endfunction", "end function"),
        ("endif", "end if"),
        ("endinterface", "end interface"),
        ("endmodule", "end module"),
        ("endprocedure", "end procedure"),
        ("endprogram", "end program"),
        ("endselect", "end select"),
        ("endsubmodule", "end submodule"),
        ("endsubroutine", "end subroutine"),
        ("endteam", "end team"),
        ("endtype", "end type"),
        ("endwhere", "end where"),
        ("selectcase", "select case"),
        ("selecttype", "select type"),
    ];
    for (joined, separated) in forms {
        let canonical = vocab::lookup_pair(vocab::COMPOUND_KEYWORDS, joined.as_bytes())
            .or_else(|| vocab::lookup_pair(vocab_2023::COMPOUND_KEYWORDS, joined.as_bytes()));
        assert_eq!(canonical, Some(separated), "missing {joined}");
    }
    assert!(vocab::contains(vocab::FORTRAN_KEYWORDS, b"goto"));
    assert!(vocab::contains(vocab::FORTRAN_KEYWORDS, b"inout"));
}

#[test]
fn supplemental_compounds_normalize_and_remain_fixed_points() {
    let source = b"program p\n\
DOUBLEPRECISION :: x\n\
DOUBLEPRECISION SUM\n\
WHERE (x > 0)\n\
ELSEWHERE\n\
ENDWHERE\n\
CRITICAL\n\
ENDCRITICAL\n\
END PROGRAM p\n";
    let once = normalize(source);
    assert_eq!(normalize(&once), once);
    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("double precision :: x"));
    assert!(output.contains("double precision SUM"));
    assert!(output.contains("else where"));
    assert!(output.contains("end critical"));
}

#[test]
fn complex_parts_follow_keyword_case_but_declared_components_win() {
    let source = b"module m\n\
type :: Parts\n\
real :: rE\n\
real :: iM\n\
end type Parts\n\
contains\n\
subroutine s(z, item)\n\
complex :: z\n\
type(Parts) :: item\n\
x = z%RE + z%IM\n\
x = item%RE + item%IM\n\
end subroutine s\n\
end module m\n";

    let once = normalize(source);
    let twice = normalize(&once);
    assert_eq!(twice, once);

    let output = String::from_utf8(once).unwrap();
    assert!(output.contains("x = z%re + z%im"));
    assert!(output.contains("x = item%rE + item%iM"));
}

#[test]
fn intrinsic_procedure_names_do_not_reclassify_defined_dotted_operators() {
    let source = b"program p\nx = a .SUM. b\ny = .NIL.\nend program p\n";
    let output = String::from_utf8(normalize(source)).unwrap();
    assert!(output.contains("x = a .SUM. b"));
    assert!(output.contains("y = .nil."));
}
