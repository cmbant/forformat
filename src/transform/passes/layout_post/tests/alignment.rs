use super::*;

#[test]
fn trailing_comments_compress_onto_one_column_across_blank_lines() {
    let source = b"integer(IntKi)  :: i, j, j_ss        ! generic loop counter\ninteger(IntKi)  :: ErrStat           ! Status of error message\n\ninteger(IntKi)  :: n_t_global        ! global-loop time counter\n";
    let output = apply_all_with_comment_alignment(source);
    for expected in [
        b"integer(IntKi) :: i, j, j_ss ! generic loop counter".as_slice(),
        b"integer(IntKi) :: ErrStat    ! Status of error message",
        b"integer(IntKi) :: n_t_global ! global-loop time counter",
    ] {
        assert!(
            output.windows(expected.len()).any(|w| w == expected),
            "missing {}",
            String::from_utf8_lossy(expected)
        );
    }
}

#[test]
fn an_isolated_trailing_comment_keeps_one_space() {
    let output =
        apply_all_with_comment_alignment(b"call sub('IF THEN END')      ! a lone comment\n");
    assert_eq!(output, b"call sub('IF THEN END') ! a lone comment\n");
}

#[test]
fn comment_alignment_is_off_by_default_and_leaves_the_authored_gap() {
    let output = apply_all(b"call sub('IF THEN END')      ! a lone comment\n");
    assert_eq!(output, b"call sub('IF THEN END')      ! a lone comment\n");
}

#[test]
fn declaration_alignment_still_shrinks_by_default_when_comment_alignment_does_not() {
    let source = b"integer(IntKi)      :: i        ! keeps its gap\ninteger(IntKi)      :: errstat  ! shorter gap\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"integer(IntKi) :: i".len())
        .any(|w| w == b"integer(IntKi) :: i"));
    assert!(output
        .windows(b"integer(IntKi) :: errstat".len())
        .any(|w| w == b"integer(IntKi) :: errstat"));
    assert!(output
        .windows(b"i        ! keeps its gap".len())
        .any(|w| w == b"i        ! keeps its gap"));
    assert!(output
        .windows(b"errstat  ! shorter gap".len())
        .any(|w| w == b"errstat  ! shorter gap"));
}

#[test]
fn declaration_alignment_can_be_disabled_to_preserve_authored_spacing() {
    let config = FormatConfig {
        align_declarations: false,
        ..FormatConfig::default()
    };
    let source = b"real      :: first\ninteger   :: second\n";
    let output = apply_all_with(source, &config);
    assert_eq!(output, source.to_vec());
}

#[test]
fn a_trailing_comment_is_not_padded_out_to_a_wider_neighbour() {
    let source = b"integer :: a_very_long_variable_name_here ! first\ninteger :: b                             ! second\n";
    let output = apply_all_with_comment_alignment(source);
    assert!(output
        .windows(b"_here ! first".len())
        .any(|w| w == b"_here ! first"));
    assert!(output
        .windows(b":: b ! second".len())
        .any(|w| w == b":: b ! second"));
}

#[test]
fn a_doc_comment_and_a_directive_keep_their_authored_gap() {
    let output =
        apply_all_with_comment_alignment(b"end if  !! trailing doc\ny = 1  !$omp barrier\n");
    assert!(output
        .windows(b"end if  !! trailing doc".len())
        .any(|w| w == b"end if  !! trailing doc"));
    assert!(output
        .windows(b"y = 1  !$omp barrier".len())
        .any(|w| w == b"y = 1  !$omp barrier"));
}

#[test]
fn declaration_separator_alignment_compresses_blocks_and_is_idempotent() {
    let source = b"module m\nreal :: a\n! between declarations\ninteger, parameter :: b = 1\ncharacter(len=4) :: literal = '::' ! keep\n#define CPP :: body\\\n  continuation :: bytes\nend module m\n";
    let once = apply_all(source);
    assert!(once.windows(2).any(|pair| pair == b"::"));
    assert!(once.windows(4).any(|window| window == b"'::'"));
    assert!(once.windows(9).any(|window| window == b"#define C"));
    assert_eq!(apply_all(&once), once);
}

#[test]
fn declaration_alignment_preserves_the_minimum_separator() {
    let source = b"real      :: first\ninteger   :: second\n\n! comment\n\nlogical   :: third\n\n\nreal   :: unaligned\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"real    :: first".len())
        .any(|w| w == b"real    :: first"));
    assert!(output
        .windows(b"integer :: second".len())
        .any(|w| w == b"integer :: second"));
    assert!(output
        .windows(b"logical :: third".len())
        .any(|w| w == b"logical :: third"));
    assert!(!output
        .windows(b"real   :: first".len())
        .any(|w| w == b"real   :: first"));
}

#[test]
fn declaration_alignment_compresses_through_blank_lines() {
    let source = b"integer(IntKi)                :: ErrStat\n   \nreal(DbKi)                    :: t_global\n\ntype(MAP_InitOutputType)      :: InitOutData_MAP\n";
    let output = apply_all(source);
    for expected in [
        b"integer(IntKi)           :: ErrStat".as_slice(),
        b"real(DbKi)               :: t_global",
        b"type(MAP_InitOutputType) :: InitOutData_MAP",
    ] {
        assert!(
            output.windows(expected.len()).any(|w| w == expected),
            "missing {}",
            String::from_utf8_lossy(expected)
        );
    }
}

#[test]
fn alignment_only_ever_removes_whitespace() {
    for source in [
        b"integer(IntKi)                :: a  ! one\n\ninteger(IntKi)                :: bb ! two\n\ntype(a_long_type_name_here)   :: c  ! three\n".as_slice(),
        b"real      :: first\ninteger   :: second\n\n! comment\n\nlogical   :: third\n\n\nreal   :: unaligned\n",
        b"integer :: a\n\ntype(a_very_long_derived_type_name) :: b\n",
    ] {
        let output = apply_all_with_comment_alignment(source);
        let gaps = |text: &[u8], marker: &[u8]| -> Vec<usize> {
            text.split(|byte| *byte == b'\n')
                .filter_map(|line| {
                    let at = line
                        .windows(marker.len())
                        .position(|window| window == marker)?;
                    Some(line[..at].len() - line[..at].trim_ascii_end().len())
                })
                .collect()
        };
        for marker in [b"::".as_slice(), b"!"] {
            for (before, after) in gaps(source, marker).iter().zip(gaps(&output, marker)) {
                assert!(
                    after <= *before || *before == 0,
                    "{marker:?} gap grew from {before} to {after} in {}",
                    String::from_utf8_lossy(source)
                );
            }
        }
    }
}

#[test]
fn declaration_alignment_reduces_procedure_generic_and_attribute_blocks() {
    let source = b"procedure, private  :: WriteSizedArray1\nprocedure, private  :: WriteSizedArray2\ngeneric  :: LoadTxt => LoadTxt_2D, LoadTxt_1D\ninteger, intent(in)   :: md\nreal(GI), intent(in)    :: xd(nxd)\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"procedure, private :: WriteSizedArray1".len())
        .any(|w| w == b"procedure, private :: WriteSizedArray1"));
    assert!(output
        .windows(b"generic :: LoadTxt =>".len())
        .any(|w| w == b"generic :: LoadTxt =>"));
    assert!(output
        .windows(b"integer, intent(in)  :: md".len())
        .any(|w| w == b"integer, intent(in)  :: md"));
    assert!(output
        .windows(b"real(GI), intent(in) :: xd(nxd)".len())
        .any(|w| w == b"real(GI), intent(in) :: xd(nxd)"));
}

#[test]
fn declaration_alignment_compresses_through_comment_lines() {
    let source = b"real(dl), intent(in)              :: ax\nreal(dl), intent(in)              :: bx\n!! of the final result\nreal(dl), intent(out)             :: xzero\ninteger, intent(out)              :: iflag\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"real(dl), intent(in)  :: ax".len())
        .any(|w| w == b"real(dl), intent(in)  :: ax"));
    assert!(output
        .windows(b"real(dl), intent(out) :: xzero".len())
        .any(|w| w == b"real(dl), intent(out) :: xzero"));
    assert!(output
        .windows(b"integer, intent(out)  :: iflag".len())
        .any(|w| w == b"integer, intent(out)  :: iflag"));
}

#[test]
fn declaration_alignment_keeps_a_compressible_subblock_before_an_unaligned_line() {
    let source = b"real(dl), intent(in)              :: ax\nreal(dl), intent(in)              :: bx\nreal(dl), intent(in), optional     :: fax\nreal(dl), parameter :: one = 1._dl\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"real(dl), intent(in)           :: ax".len())
        .any(|w| w == b"real(dl), intent(in)           :: ax"));
    assert!(output
        .windows(b"real(dl), intent(in), optional :: fax".len())
        .any(|w| w == b"real(dl), intent(in), optional :: fax"));
    assert!(output
        .windows(b"real(dl), parameter :: one".len())
        .any(|w| w == b"real(dl), parameter :: one"));
}

#[test]
fn declaration_alignment_never_adds_padding_to_short_lines() {
    let source = b"type(c_ptr) :: cptr\ntype(ModelParams), pointer :: PType\nclass(InterfaceClass), pointer :: P\n\nclass(ModelParams), target :: this\ntype(ModelParams), pointer :: p\n";
    let output = apply_all(source);
    assert!(output
        .windows(b"type(c_ptr) :: cptr".len())
        .any(|w| w == b"type(c_ptr) :: cptr"));
    assert!(output
        .windows(b"type(ModelParams), pointer :: PType".len())
        .any(|w| w == b"type(ModelParams), pointer :: PType"));
    assert!(!output
        .windows(b"type(c_ptr)     :: cptr".len())
        .any(|w| w == b"type(c_ptr)     :: cptr"));
}

#[test]
fn a_comment_after_a_unit_end_opens_the_next_unit() {
    let source = b"function f\nend function f\n! trailing note\nsubroutine s\nend subroutine s\n";
    let expected =
        b"function f\n\nend function f\n\n! trailing note\nsubroutine s\n\nend subroutine s\n";
    let once = apply_all(source);
    assert_eq!(once, expected);
    assert_eq!(apply_all(&once), once);
}

#[test]
fn post_layout_passes_never_lengthen_retained_lines() {
    let source = b"module m\nreal(dl), intent(in) :: x\n! comment\ninteger :: y\ncontains\nsubroutine s\nend subroutine s\nend module m\n";
    let mut document = Document::from_bytes(source);
    let before = document.lines.clone();
    let config = FormatConfig::default();
    declaration_separator_alignment(&mut document, &config).unwrap();
    assert!(before
        .iter()
        .zip(&document.lines)
        .all(|(old, new)| new.len() <= old.len()));

    let before = document.lines.clone();
    program_unit_spacing(&mut document, &config).unwrap();
    assert_retained_lines_do_not_grow(&before, &document.lines);

    let before = document.lines.clone();
    limit_blank_lines(&mut document, &config).unwrap();
    assert_retained_lines_do_not_grow(&before, &document.lines);

    let before = document.lines.clone();
    output_whitespace(&mut document, &config).unwrap();
    assert!(before
        .iter()
        .zip(&document.lines)
        .all(|(old, new)| new.len() <= old.len()));
}
