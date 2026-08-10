use super::{buffer::comment_start, scanner, PhysicalLineKind, SourceBuffer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalStatement {
    pub text: Vec<u8>,
    pub is_fix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalGroup {
    pub lines: std::ops::Range<usize>,
    pub statements: Vec<LogicalStatement>,
}

impl LogicalGroup {
    pub fn assemble(buf: &SourceBuffer) -> Vec<Self> {
        let mut groups = Vec::new();
        Self::visit(buf, |group| {
            groups.push(group);
            Ok::<(), ()>(())
        })
        .expect("logical-group collection cannot fail");
        groups
    }

    pub fn visit<E, F>(buf: &SourceBuffer, mut visit: F) -> Result<(), E>
    where
        F: FnMut(Self) -> Result<(), E>,
    {
        let mut i = 0;
        while i < buf.lines.len() {
            let start = i;
            let first = &buf.lines[i];
            if matches!(
                first.kind,
                PhysicalLineKind::Blank | PhysicalLineKind::Comment
            ) {
                visit(Self {
                    lines: i..i + 1,
                    statements: Vec::new(),
                })?;
                i += 1;
                continue;
            }
            if first.kind == PhysicalLineKind::Preprocessor {
                let mut j = i + 1;
                let mut cur = buf.code_bytes(first);
                let continuation = if cur.trim_ascii_start().starts_with(b"??")
                    || cur.trim_ascii_start().starts_with(b"#:")
                {
                    b'&'
                } else {
                    b'\\'
                };
                let mut more = trailing_directive(cur, continuation);
                while more && j < buf.lines.len() {
                    let l = &buf.lines[j];
                    cur = buf.code_bytes(l);
                    more = trailing_directive(cur, continuation);
                    j += 1;
                }
                visit(Self {
                    lines: i..j,
                    statements: Vec::new(),
                })?;
                i = j;
                continue;
            }
            let mut j = i + 1;
            let mut joined = if first.kind == PhysicalLineKind::FindentFix {
                buf.line_bytes(first).to_vec()
            } else {
                normalized_fragment(buf.code_bytes(first), false)
            };
            let mut more = trailing_amp(buf.code_bytes(first));
            while more && j < buf.lines.len() {
                let l = &buf.lines[j];
                // A comment, blank, or CPP directive within a continued
                // statement is emitted as its own physical line but does not
                // terminate the surrounding Fortran statement.
                if matches!(
                    l.kind,
                    PhysicalLineKind::Preprocessor
                        | PhysicalLineKind::Comment
                        | PhysicalLineKind::Blank
                ) {
                    j += 1;
                    continue;
                }
                let s = buf.code_bytes(l);
                joined.extend_from_slice(&normalized_fragment(s, true));
                more = trailing_amp(s);
                j += 1;
            }
            let mut statements = Vec::new();
            if first.kind == PhysicalLineKind::FindentFix {
                let t = fix_payload(&joined);
                if !t.is_empty() {
                    statements.push(LogicalStatement {
                        text: t,
                        is_fix: true,
                    });
                }
            } else {
                for s in scanner::split_statements(&joined) {
                    if !s.iter().all(|x| x.is_ascii_whitespace()) {
                        statements.push(LogicalStatement {
                            text: s.to_vec(),
                            is_fix: false,
                        });
                    }
                }
            }
            visit(Self {
                lines: start..j,
                statements,
            })?;
            i = j;
        }
        Ok(())
    }
}

fn trailing_amp(s: &[u8]) -> bool {
    let mut t = s;
    while t.last().is_some_and(|x| x.is_ascii_whitespace()) {
        t = &t[..t.len() - 1];
    }
    t.last() == Some(&b'&')
}

fn trailing_directive(s: &[u8], continuation: u8) -> bool {
    let mut t = s;
    while t.last().is_some_and(|x| x.is_ascii_whitespace()) {
        t = &t[..t.len() - 1];
    }
    t.last() == Some(&continuation)
}

/// Produce the structural view of one continued physical line.  The original
/// bytes remain in `SourceBuffer` for emission; this only removes syntax that
/// joins physical lines so recognizers see the same statement a Fortran reader
/// sees.
fn normalized_fragment(s: &[u8], continuation_line: bool) -> Vec<u8> {
    let mut start = 0;
    while start < s.len() && (s[start] == b' ' || s[start] == b'\t') {
        start += 1;
    }
    if continuation_line && s.get(start) == Some(&b'&') {
        start += 1;
    }
    let mut end = s.len();
    while end > start && s[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end > start && s[end - 1] == b'&' {
        end -= 1;
    }
    s[start..end].to_vec()
}
fn fix_payload(s: &[u8]) -> Vec<u8> {
    let Some(mut i) = comment_start(s) else {
        return Vec::new();
    };
    i += 1;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    if s[i..].len() < b"findentfix:".len()
        || !s[i..i + b"findentfix:".len()].eq_ignore_ascii_case(b"findentfix:")
    {
        return Vec::new();
    }
    i += 11;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    s[i..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::LogicalGroup;
    use crate::source::SourceBuffer;

    #[test]
    fn directive_continuation_does_not_classify_following_source() {
        let buffer = SourceBuffer::new(
            br#"#define VALUE \
program p
x = 1
"#,
        )
        .unwrap();
        let groups = LogicalGroup::assemble(&buffer);
        assert_eq!(groups[0].lines, 0..2);
        assert!(groups[0].statements.is_empty());
        assert_eq!(groups[1].statements.len(), 1);
    }

    #[test]
    fn embedded_comments_and_cpp_lines_remain_in_a_fortran_group() {
        let buffer = SourceBuffer::new(
            b"x = a &\n! keep this comment\n#ifdef X\n & b &\n#endif\n & c\nend\n",
        )
        .unwrap();
        let groups = LogicalGroup::assemble(&buffer);
        assert_eq!(groups[0].lines, 0..6);
        assert_eq!(groups[0].statements.len(), 1);
        assert_eq!(groups[0].statements[0].text, b"x = a  b  c");
    }

    #[test]
    fn continued_source_accepts_blank_lines_and_leading_ampersands() {
        let buffer = SourceBuffer::new(b"x = a &\n\n & b &\n & c\n").unwrap();
        let groups = LogicalGroup::assemble(&buffer);
        assert_eq!(groups[0].lines, 0..4);
        assert_eq!(groups[0].statements[0].text, b"x = a  b  c");
    }

    #[test]
    fn directive_and_source_boundaries_survive_truncated_editor_buffers() {
        for source in [
            b"#define X \\\n+".as_slice(),
            b"x = a &\n! comment\n#ifdef X\n & b".as_slice(),
            b"! findentfix: do\n".as_slice(),
        ] {
            let buffer = SourceBuffer::new(source).unwrap();
            for group in LogicalGroup::assemble(&buffer) {
                assert!(group.lines.start <= group.lines.end);
                assert!(group.lines.end <= buffer.lines.len());
                for statement in group.statements {
                    assert!(!statement.text.is_empty());
                }
            }
        }
    }

    #[test]
    fn directive_continuation_does_not_skip_a_later_cpp_event() {
        let buffer = SourceBuffer::new(b"#define X \\\n+body\n#if X\nprogram p\n#endif\n").unwrap();
        let groups = LogicalGroup::assemble(&buffer);
        assert_eq!(groups[0].lines, 0..2);
        assert_eq!(groups[1].lines, 2..3);
        assert_eq!(groups[2].lines, 3..4);
        assert_eq!(groups[3].lines, 4..5);
    }

    #[test]
    fn visitor_matches_collected_groups() {
        let buffer =
            SourceBuffer::new(b"program p\nif (x) then\n! comment\n#endif\nend if\nend program\n")
                .unwrap();
        let collected = LogicalGroup::assemble(&buffer);
        let mut visited = Vec::new();
        LogicalGroup::visit(&buffer, |group| {
            visited.push(group);
            Ok::<(), ()>(())
        })
        .unwrap();
        assert_eq!(visited, collected);
    }
}
