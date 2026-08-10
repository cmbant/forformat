use crate::source::buffer::comment_start;

pub fn payload(line: &[u8]) -> Option<&[u8]> {
    let mut i = comment_start(line)?;
    i += 1;
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    if !line[i..].to_ascii_lowercase().starts_with(b"findentfix:") {
        return None;
    }
    i += 11;
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    if line[i..].eq_ignore_ascii_case(b"p-on") || line[i..].eq_ignore_ascii_case(b"p-off") {
        None
    } else {
        Some(&line[i..])
    }
}
