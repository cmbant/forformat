use super::stack::IndentStack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreprocessorEvent {
    None,
    If,
    Elif,
    Else,
    EndIf,
}

#[derive(Debug, Clone, Default)]
pub struct PreprocessorState {
    pub stack: Vec<(IndentStack, bool)>,
}

pub fn event(line: &[u8]) -> PreprocessorEvent {
    let mut s = line;
    while s.first().is_some_and(|c| c.is_ascii_whitespace()) {
        s = &s[1..];
    }
    if s.starts_with(b"??") || s.starts_with(b"#:") {
        s = &s[2..];
    } else if s.starts_with(b"#") {
        s = &s[1..];
    } else {
        return PreprocessorEvent::None;
    }
    while s.first().is_some_and(|c| c.is_ascii_whitespace()) {
        s = &s[1..];
    }
    let end = s
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(s.len());
    let word = &s[..end];
    if word.eq_ignore_ascii_case(b"if")
        || word
            .strip_suffix(b"(")
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"if"))
        || (word.len() >= 3 && word[..3].eq_ignore_ascii_case(b"if("))
        || word.eq_ignore_ascii_case(b"ifdef")
        || word.eq_ignore_ascii_case(b"ifndef")
    {
        PreprocessorEvent::If
    } else if word.eq_ignore_ascii_case(b"elif") {
        PreprocessorEvent::Elif
    } else if word.eq_ignore_ascii_case(b"else") {
        PreprocessorEvent::Else
    } else if word.eq_ignore_ascii_case(b"endif") {
        PreprocessorEvent::EndIf
    } else {
        PreprocessorEvent::None
    }
}

impl PreprocessorState {
    pub fn apply(&mut self, event: PreprocessorEvent, state: &mut IndentStack) {
        match event {
            PreprocessorEvent::If => self.stack.push((state.snapshot(), false)),
            PreprocessorEvent::Elif => {
                if let Some((s, _)) = self.stack.last() {
                    *state = s.clone()
                }
            }
            PreprocessorEvent::Else => {
                if let Some((s, seen)) = self.stack.last_mut() {
                    *state = s.clone();
                    *seen = true
                }
            }
            PreprocessorEvent::EndIf => {
                if let Some((_, seen)) = self.stack.pop() {
                    if !seen { /* keep current branch state */ }
                }
            }
            PreprocessorEvent::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{event, PreprocessorEvent};

    #[test]
    fn cpp_and_deferred_dialect_directives_change_branch_state() {
        assert_eq!(event(b"#ifdef X"), PreprocessorEvent::If);
        assert_eq!(event(b"#else"), PreprocessorEvent::Else);
        assert_eq!(event(b"#endif"), PreprocessorEvent::EndIf);
        assert_eq!(event(b"??if(foo)"), PreprocessorEvent::If);
        assert_eq!(event(b"??if("), PreprocessorEvent::If);
        assert_eq!(event(b"??else"), PreprocessorEvent::Else);
        assert_eq!(event(b"#:endif"), PreprocessorEvent::EndIf);
    }
}
