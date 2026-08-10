use crate::classify::StatementKind;
pub fn end_text(kind: StatementKind, name: Option<&[u8]>, uppercase: bool) -> Vec<u8> {
    let mut s = match kind {
        StatementKind::Program => b"end program".to_vec(),
        StatementKind::Module => b"end module".to_vec(),
        StatementKind::Submodule => b"end submodule".to_vec(),
        StatementKind::Subroutine => b"end subroutine".to_vec(),
        StatementKind::Function => b"end function".to_vec(),
        StatementKind::Interface | StatementKind::AbstractInterface => b"end interface".to_vec(),
        StatementKind::Type => b"end type".to_vec(),
        StatementKind::BlockData => b"end block data".to_vec(),
        StatementKind::Procedure => b"end procedure".to_vec(),
        _ => b"end".to_vec(),
    };
    if uppercase {
        s.make_ascii_uppercase();
    }
    if let Some(n) = name {
        if !n.is_empty() {
            s.push(b' ');
            s.extend_from_slice(n)
        }
    }
    s
}
