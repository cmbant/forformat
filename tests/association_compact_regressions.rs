use forformat::{
    analysis::ProjectContext, format_source, format_source_with_context, FormatConfig, FormatMode,
};

fn full_without_wrap() -> FormatConfig {
    FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    }
}

#[test]
fn compact_endassociate_closes_member_type_scope() {
    let source = b"program p\n  type :: T\n    integer :: FieldName\n  end type T\n  type(T) :: x\n  associate(Alias => x)\n    print *, alias%fieldname\n  endassociate\n  print *, alias%fieldname\nend program p\n";
    let output = format_source(source, &full_without_wrap()).unwrap().bytes;
    let text = String::from_utf8(output).unwrap();

    assert!(
        text.contains("Alias%FieldName"),
        "association did not resolve inside scope:\n{text}"
    );
    let after = text
        .split_once("end associate")
        .map(|(_, after)| after)
        .expect("formatter should spell compact ENDASSOCIATE canonically");
    assert!(
        after.contains("alias%fieldname") && !after.contains("Alias%FieldName"),
        "compact ENDASSOCIATE leaked alias/type evidence past the construct:\n{text}"
    );
}

#[test]
fn compact_endselect_closes_member_type_scope() {
    let source = b"program p\n  type :: T\n    integer :: FieldName\n  end type T\n  class(T), allocatable :: x\n  select type (Alias => x)\n  type is (T)\n    print *, alias%fieldname\n  endselect\n  print *, alias%fieldname\nend program p\n";
    let output = format_source(source, &full_without_wrap()).unwrap().bytes;
    let text = String::from_utf8(output).unwrap();

    assert!(
        text.contains("Alias%FieldName"),
        "SELECT TYPE alias did not resolve inside scope:\n{text}"
    );
    let after = text
        .split_once("end select")
        .map(|(_, after)| after)
        .expect("formatter should spell compact ENDSELECT canonically");
    assert!(
        after.contains("alias%fieldname") && !after.contains("Alias%FieldName"),
        "compact ENDSELECT leaked alias/type evidence past the construct:\n{text}"
    );
}

#[test]
fn compact_selecttype_and_selectrank_create_association_scopes() {
    for source in [
        b"program p\n  type :: T\n    integer :: FieldName\n  end type T\n  class(T), allocatable :: x\n  selecttype(Alias => x)\n  type is (T)\n    print *, alias%fieldname\n  endselect\nend program p\n".as_slice(),
        b"program p\n  type :: T\n    integer :: FieldName\n  end type T\n  type(T), allocatable :: x(:)\n  selectrank(Alias => x)\n  rank(1)\n    print *, alias%fieldname\n  endselect\nend program p\n".as_slice(),
    ] {
        let output = format_source(source, &full_without_wrap()).unwrap().bytes;
        let text = String::from_utf8(output).unwrap();
        assert!(
            text.contains("Alias%FieldName"),
            "compact SELECT opener did not establish association provenance:\n{text}"
        );
    }
}

#[test]
fn wrapper_is_total_for_separator_only_reproducer() {
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 50,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let result = format_source_with_context(b"\n;", &ProjectContext::empty(), &config);
    assert!(
        result.is_ok(),
        "separator-only wrapper reproducer returned {result:?}"
    );
}
