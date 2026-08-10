pub fn leading_ampersand(line: &[u8]) -> bool {
    let mut s = line;
    while s.first().is_some_and(|c| c.is_ascii_whitespace()) {
        s = &s[1..];
    }
    s.first() == Some(&b'&')
}

pub fn trailing_ampersand(line: &[u8]) -> bool {
    let mut s = line;
    while s.last().is_some_and(|c| c.is_ascii_whitespace()) {
        s = &s[..s.len() - 1];
    }
    s.last() == Some(&b'&')
}

pub fn paren_alignment(line: &[u8]) -> Option<usize> {
    let mut quote = 0u8;
    let mut stack = Vec::new();
    let mut last_sig = None;
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if quote != 0 {
            if c == quote {
                if line.get(i + 1) == Some(&quote) {
                    i += 2;
                    continue;
                }
                quote = 0
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quote = c;
            i += 1;
            continue;
        }
        if c == b'!' {
            break;
        }
        if c == b'(' || c == b'[' {
            stack.push(i);
            last_sig = None;
        } else if c == b')' || c == b']' {
            stack.pop();
            last_sig = Some(i);
        } else if !c.is_ascii_whitespace() {
            last_sig = Some(i);
        }
        if stack.len() == 1 && last_sig == Some(i) && c != b'(' && c != b'[' {
            return stack.first().copied().map(|p| p + 1);
        }
        i += 1;
    }
    None
}

#[derive(Debug, Default)]
pub struct ParenAlignmentState {
    stack: Vec<OpenParen>,
    quote: u8,
    prev_indent: usize,
    start_indent: Option<usize>,
}

#[derive(Debug)]
struct OpenParen {
    alignment: usize,
    first_item_seen: bool,
}

impl ParenAlignmentState {
    pub fn current(&self) -> Option<usize> {
        self.stack.last().map(|open| open.alignment)
    }

    /// Scan one physical code line. The returned alignment is the value that
    /// was active before this line; newly opened parentheses affect the next
    /// continued line. `line_target` is the column at which the code body of
    /// this physical line is emitted.
    pub fn scan(&mut self, line: &[u8], line_target: usize) -> Option<usize> {
        let active = self.current();
        if self.start_indent.is_none() {
            self.start_indent = Some(line_target);
        }
        // When an active alignment exists, the oracle scans the trimmed
        // continuation body and adds its column to the previous target.  A
        // line without an active delimiter is scanned with its emitted
        // leading indentation still present.  `line_target` plus the source
        // index models both cases after paren_scan_line has trimmed input.
        let position = |index: usize| {
            self.prev_indent
                + if active.is_some() {
                    index
                } else {
                    line_target + index
                }
        };
        let mut indent = self.prev_indent;
        let mut i = 0;
        while i < line.len() {
            let byte = line[i];
            if self.quote != 0 {
                if byte == self.quote {
                    if line.get(i + 1) == Some(&self.quote) {
                        i += 2;
                        continue;
                    }
                    self.quote = 0;
                }
                i += 1;
                continue;
            }
            if byte == b'\'' || byte == b'"' {
                if let Some(open) = self.stack.last_mut() {
                    if !open.first_item_seen {
                        open.alignment = position(i);
                        open.first_item_seen = true;
                        indent = open.alignment;
                    }
                }
                self.quote = byte;
                i += 1;
                continue;
            }
            if byte == b'!' {
                break;
            }
            match byte {
                b'(' | b'[' => {
                    if let Some(parent) = self.stack.last_mut() {
                        // A nested delimiter is already the first item of
                        // its parent.  The oracle keeps the parent's target
                        // at the parent delimiter in this case.
                        parent.first_item_seen = true;
                    }
                    let alignment = position(i);
                    self.stack.push(OpenParen {
                        alignment,
                        first_item_seen: false,
                    });
                    indent = alignment;
                }
                b')' | b']' => {
                    let was_empty = self.stack.pop().is_some_and(|open| !open.first_item_seen);
                    if was_empty {
                        if let Some(parent) = self.stack.last_mut() {
                            // An empty nested call leaves the oracle in its
                            // lparen state, so the next separator can become
                            // the active target for the parent delimiter.
                            parent.first_item_seen = false;
                        }
                    }
                    indent = self
                        .stack
                        .last()
                        .map(|open| open.alignment)
                        .or(self.start_indent)
                        .unwrap_or(indent);
                }
                // findent's get_paren_align ignores slash separators while
                // looking for the first item inside a delimiter.  Once that
                // item is found, it aligns to its actual column, rather than
                // to the column immediately following the opening delimiter.
                _ if !byte.is_ascii_whitespace() && byte != b'/' => {
                    if let Some(open) = self.stack.last_mut() {
                        if !open.first_item_seen {
                            open.alignment = position(i);
                            open.first_item_seen = true;
                            indent = open.alignment;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        self.prev_indent = indent;
        active
    }
}

#[cfg(test)]
mod tests {
    use super::ParenAlignmentState;

    #[test]
    fn tracks_nested_parentheses_across_physical_lines() {
        let mut state = ParenAlignmentState::default();
        assert_eq!(state.scan(b"call sub(a, &", 11), None);
        assert_eq!(state.current(), Some(20));
        assert_eq!(state.scan(b"fun(3.0, &", 20), Some(20));
        assert_eq!(state.current(), Some(24));
        assert_eq!(state.scan(b"4.0), &", 24), Some(24));
        assert_eq!(state.current(), Some(20));
        assert_eq!(state.scan(b"5.0)", 20), Some(20));
        assert_eq!(state.current(), None);
    }

    #[test]
    fn ignores_parentheses_inside_strings_and_comments() {
        let mut state = ParenAlignmentState::default();
        assert_eq!(state.scan(b"print *, '(not code)' ! (comment)", 3), None);
        assert_eq!(state.current(), None);
    }

    #[test]
    fn aligns_after_delimiter_whitespace_to_first_item() {
        let mut state = ParenAlignmentState::default();
        assert_eq!(state.scan(b"x = [ a, &", 3), None);
        assert_eq!(state.current(), Some(9));
        assert_eq!(state.scan(b"/ a, &", 9), Some(9));
        assert_eq!(state.current(), Some(9));
    }
}
