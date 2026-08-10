pub fn reduce(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    reduce_into(s, |byte| out.push(byte));
    out
}

/// Apply redundant-whitespace reduction without allocating an output buffer.
/// Quotes, doubled quotes, and comments are copied byte-for-byte.
pub fn reduce_into<F: FnMut(u8)>(s: &[u8], mut put: F) {
    let mut quote = 0u8;
    reduce_line_into(s, &mut quote, &mut put);
}

/// Reduce one physical line while carrying a quote state across free-form
/// continuation lines. Fortran strings may span physical lines, so a
/// reducer that starts each line in the unquoted state would rewrite spaces
/// inside the continued literal.
pub fn reduce_line_into<F: FnMut(u8)>(s: &[u8], quote: &mut u8, put: &mut F) {
    let mut space = false;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if *quote != 0 {
            put(c);
            if c == *quote {
                if s.get(i + 1) == Some(&*quote) {
                    put(s[i + 1]);
                    i += 1;
                } else {
                    *quote = 0;
                }
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            if space {
                put(b' ');
                space = false;
            }
            *quote = c;
            put(c);
        } else if c == b'!' {
            if space {
                put(b' ');
            }
            for &byte in &s[i..] {
                put(byte);
            }
            break;
        } else if c == b' ' || c == b'\t' {
            space = true;
        } else {
            if space {
                put(b' ');
                space = false;
            }
            put(c);
        }
        i += 1;
    }
}

pub fn reduce_to<W: std::io::Write>(s: &[u8], out: &mut W) -> std::io::Result<()> {
    let mut quote = 0u8;
    reduce_to_with_quote(s, &mut quote, out)
}

pub fn reduce_to_with_quote<W: std::io::Write>(
    s: &[u8],
    quote: &mut u8,
    out: &mut W,
) -> std::io::Result<()> {
    let mut error = None;
    reduce_line_into(s, quote, &mut |byte| {
        if error.is_none() {
            if let Err(e) = out.write_all(&[byte]) {
                error = Some(e);
            }
        }
    });
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{reduce, reduce_line_into};

    #[test]
    fn continued_strings_keep_their_internal_spaces() {
        let mut output = Vec::new();
        let mut quote = 0;
        reduce_line_into(
            b"call sub(\"hello              &",
            &mut quote,
            &mut |byte| output.push(byte),
        );
        reduce_line_into(b"world  \" ,  &", &mut quote, &mut |byte| output.push(byte));
        assert_eq!(
            output,
            b"call sub(\"hello              &world  \" , &".to_vec()
        );
        assert_eq!(quote, 0);
    }

    #[test]
    fn standalone_reduction_remains_quote_aware() {
        assert_eq!(reduce(b"x   =   \"a  b\"   +  y"), b"x = \"a  b\" + y");
    }
}
