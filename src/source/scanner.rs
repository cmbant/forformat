use super::buffer::{comment_start, SourceBuffer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub text: &'a [u8],
    pub start: usize,
    pub end: usize,
}

pub fn tokens<'a>(s: &'a [u8]) -> Vec<Token<'a>> {
    let mut out = Vec::new();
    let mut i = 0;
    let end = comment_start(s).unwrap_or(s.len());
    while i < end {
        while i < end && s[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }
        let start = i;
        let c = s[i];
        if c == b'\'' || c == b'"' {
            let q = c;
            i += 1;
            while i < end {
                if s[i] == q {
                    if s.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if c.is_ascii_alphanumeric() || c == b'_' {
            i += 1;
            while i < end && (s[i].is_ascii_alphanumeric() || s[i] == b'_') {
                i += 1;
            }
        } else if c == b'.' && s.get(i + 1).is_some_and(|x| x.is_ascii_alphabetic()) {
            i += 1;
            while i < end && s[i] != b'.' {
                i += 1;
            }
            if i < end {
                i += 1;
            }
        } else {
            i += 1;
        }
        out.push(Token {
            text: &s[start..i],
            start,
            end: i,
        });
    }
    out
}

pub fn split_statements(s: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quote = 0u8;
    let mut hollerith = 0usize;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if hollerith > 0 {
            hollerith -= 1;
            i += 1;
            continue;
        }
        if quote != 0 {
            if c == quote {
                if s.get(i + 1) == Some(&quote) {
                    i += 2;
                    continue;
                }
                quote = 0;
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
        if c == b';' {
            out.push(&s[start..i]);
            start = i + 1;
        }
        if c.is_ascii_digit() && (i == 0 || (!s[i - 1].is_ascii_alphanumeric() && s[i - 1] != b'_'))
        {
            let mut j = i;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            if s.get(j).is_some_and(|x| *x == b'h' || *x == b'H') {
                hollerith = std::str::from_utf8(&s[i..j])
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

pub fn normalized_statement(buf: &SourceBuffer, lines: std::ops::Range<usize>) -> Vec<u8> {
    let mut result = Vec::new();
    for (n, i) in lines.enumerate() {
        let line = &buf.lines[i];
        let mut s = buf.code_bytes(line);
        let mut j = 0;
        while j < s.len() && (s[j] == b' ' || s[j] == b'\t') {
            j += 1;
        }
        s = &s[j..];
        if n > 0 && s.first() == Some(&b'&') {
            s = &s[1..];
        }
        while s.last().is_some_and(|x| x.is_ascii_whitespace()) {
            s = &s[..s.len() - 1];
        }
        if s.last() == Some(&b'&') {
            s = &s[..s.len() - 1];
        }
        result.extend_from_slice(s);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{split_statements, tokens};

    #[test]
    fn statement_splitting_respects_strings_and_hollerith() {
        let parts = split_statements(b"a='x;y'; b=3H;!;; c=1");
        assert_eq!(
            parts,
            [b"a='x;y'".as_slice(), b" b=3H;!;".as_slice(), b" c=1"]
        );
    }

    #[test]
    fn tokens_keep_dot_operators_as_one_opaque_token() {
        let values: Vec<&[u8]> = tokens(b"if (x .EQ. y) then")
            .into_iter()
            .map(|token| token.text)
            .collect();
        assert!(values
            .iter()
            .any(|token| token.eq_ignore_ascii_case(b".EQ.")));
    }
}
