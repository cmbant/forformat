//! Detection of fixed- versus free-form Fortran input.
//!
//! This is a byte-for-byte port of findent 4.3.7's `determine_fix_or_free`
//! path. Keeping the detector independent of the formatter lets the file
//! workflow decline fixed-form sources before free-form parsing can alter them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceForm {
    Free,
    Fixed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreprocessorKind {
    Cpp,
    Coco,
    Fypp,
    Other,
}

/// Determine the source form of a named file.
///
/// findent reads standard input, so its detector has only the bytes to go on
/// and answers FIXED whenever no line is decisive — which is exactly what a
/// free-form file that opens with a comment block and indents its first
/// statements past column six looks like. Across the five corpus checkouts
/// that misfires on 24 `.f90` sources (Q-E's `Modules/expint.f90` among them):
/// findent itself reports `fixed` for every one.
///
/// A filename settles it. `.f90` and its successors were introduced with free
/// form and are not used for fixed form in practice, so only the bare `.f`
/// spelling is genuinely ambiguous and worth asking the detector about.
pub fn detect_path(path: &std::path::Path, source: &[u8]) -> SourceForm {
    let ambiguous_suffix = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("f"));
    if ambiguous_suffix {
        detect(source)
    } else {
        SourceForm::Free
    }
}

/// Determine the source form using findent's fixed/free format detector.
///
/// Prefer [`detect_path`] whenever a filename is available; this is the raw
/// port, for input that arrives without one.
pub fn detect(source: &[u8]) -> SourceForm {
    let mut preprocessor = PreprocessorKind::Other;
    let mut p_more = false;
    let mut skip = false;

    // findent examines at most 4000 physical lines and defaults to fixed at
    // EOF. `split_inclusive` avoids inventing a final empty getline after a
    // source that ends in a newline.
    for raw_line in source.split_inclusive(|byte| *byte == b'\n').take(4000) {
        let mut line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        // findent's input reader removes CR from DOS line endings before the
        // detector sees the line; its rtrim only removes spaces and tabs.
        line = line.strip_suffix(b"\r").unwrap_or(line);

        if !p_more {
            preprocessor = preprocessor_kind(line);
        }
        p_more = match preprocessor {
            PreprocessorKind::Coco | PreprocessorKind::Fypp => line.last() == Some(&b'&'),
            PreprocessorKind::Cpp | PreprocessorKind::Other => line.last() == Some(&b'\\'),
        };
        if p_more {
            skip = true;
            continue;
        }
        if skip {
            skip = false;
            continue;
        }

        // The original guard runs before ltab2sp and treats a leading control
        // byte (except a tab) as UNSURE. It therefore cannot decide the form.
        if line
            .first()
            .is_some_and(|byte| *byte != b'\t' && *byte < 32)
        {
            continue;
        }
        let normalized = ltab2sp(line);
        match classify_line(&normalized) {
            Classification::Free => return SourceForm::Free,
            Classification::Fixed => return SourceForm::Fixed,
            Classification::Unsure => {}
        }
    }
    SourceForm::Fixed
}

fn preprocessor_kind(line: &[u8]) -> PreprocessorKind {
    let line = trim_left(line);
    if line.starts_with(b"??") {
        PreprocessorKind::Coco
    } else if line.starts_with(b"#:") {
        PreprocessorKind::Fypp
    } else if line.first() == Some(&b'#') {
        PreprocessorKind::Cpp
    } else {
        PreprocessorKind::Other
    }
}

fn ltab2sp(line: &[u8]) -> Vec<u8> {
    let Some(tab) = line.iter().take(6).position(|byte| *byte == b'\t') else {
        return rtrim(line).to_vec();
    };
    if !line[..tab]
        .iter()
        .all(|byte| *byte == b' ' || byte.is_ascii_digit())
    {
        return rtrim(line).to_vec();
    }
    if tab + 1 == line.len() {
        return rtrim(&line[..tab]).to_vec();
    }
    let continuation = matches!(line[tab + 1], b'1'..=b'9');
    let spaces = if continuation { 5 - tab } else { 6 - tab };
    let mut converted = Vec::with_capacity(line.len() + spaces - 1);
    converted.extend_from_slice(&line[..tab]);
    converted.extend(std::iter::repeat_n(b' ', spaces));
    converted.extend_from_slice(&line[tab + 1..]);
    rtrim(&converted).to_vec()
}

fn rtrim(mut line: &[u8]) -> &[u8] {
    while line
        .last()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        line = &line[..line.len() - 1];
    }
    line
}

fn trim_left(mut line: &[u8]) -> &[u8] {
    while line
        .first()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        line = &line[1..];
    }
    line
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    Free,
    Fixed,
    Unsure,
}

fn classify_line(line: &[u8]) -> Classification {
    // Every flex rule here has a trailing `\\n`; because `.*` cannot cross
    // that newline, every successful rule consumes the entire physical line.
    // Thus all matches tie on length and first-rule order is the flex result.
    if line.starts_with(b"??") {
        return Classification::Unsure;
    }
    if line.first().is_some_and(|byte| {
        !matches!(
            byte,
            b'd' | b'D' | b'c' | b'C' | b'#' | b'!' | b'*' | b' ' | b'0'..=b'9'
        )
    }) {
        return Classification::Free;
    }
    for index in 1..=4 {
        if line.len() > index
            && line[..index]
                .iter()
                .all(|byte| *byte == b' ' || byte.is_ascii_digit())
            && (line[index].is_ascii_alphabetic() || matches!(line[index], b'_' | b'%'))
        {
            return Classification::Free;
        }
    }
    if let Some(bang) = line.iter().position(|byte| *byte == b'!') {
        if line[..bang].iter().all(|byte| *byte != b'!') {
            return Classification::Unsure;
        }
    }
    if line.starts_with(b"     &") && !line[6..].contains(&b'&') {
        return Classification::Unsure;
    }
    if ampersand_with_trailing_blanks(line, 0, 4) {
        return Classification::Free;
    }
    if ampersand_with_trailing_blanks(line, 6, usize::MAX) {
        return Classification::Free;
    }
    if let Some(ampersand) = line.iter().position(|byte| *byte == b'&') {
        if !line[..ampersand].contains(&b'&')
            && line[ampersand + 1..]
                .iter()
                .all(|byte| *byte == b' ' || *byte == b'\t')
        {
            return Classification::Free;
        }
    }
    let leading_spaces = line.iter().take_while(|byte| **byte == b' ').count();
    if leading_spaces <= 4 && line.get(leading_spaces) == Some(&b'&') {
        return Classification::Free;
    }
    if leading_spaces >= 6 && line.get(leading_spaces) == Some(&b'&') {
        return Classification::Free;
    }
    if ckey(line) {
        return Classification::Free;
    }
    if line
        .first()
        .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'c'))
    {
        let mut index = 1;
        while line
            .get(index)
            .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
        {
            index += 1;
        }
        if index > 1 && line.get(index).is_some_and(u8::is_ascii_alphanumeric) {
            return Classification::Fixed;
        }
    }
    if line.len() >= 6
        && line[..6]
            .iter()
            .all(|byte| *byte == b' ' || byte.is_ascii_digit())
    {
        return Classification::Unsure;
    }
    Classification::Unsure
}

fn ampersand_with_trailing_blanks(line: &[u8], first: usize, last: usize) -> bool {
    line.iter().enumerate().any(|(index, byte)| {
        index >= first
            && index <= last
            && *byte == b'&'
            && line[index + 1..]
                .iter()
                .all(|tail| *tail == b' ' || *tail == b'\t')
    })
}

fn ckey(line: &[u8]) -> bool {
    [
        b"call".as_slice(),
        b"close",
        b"common",
        b"continue",
        b"case",
        b"contains",
        b"cycle",
        b"class",
        b"codimension",
        b"contiguous",
        b"critical",
        b"complex",
        b"changeteam",
    ]
    .iter()
    .any(|keyword| {
        line.len() >= keyword.len()
            && line[..keyword.len()]
                .iter()
                .zip(keyword.iter())
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    })
}

#[cfg(test)]
mod tests {
    use super::{detect, detect_path, SourceForm};
    use std::path::Path;

    #[test]
    fn only_the_bare_f_suffix_is_asked_about() {
        // Reduced from Q-E `Modules/expint.f90`: a comment block, then a
        // statement indented past column six.  Nothing in it is decisive, so
        // findent's EOF default calls it fixed — `findent -q` agrees — and 24
        // free-form corpus sources would be skipped if the suffix were ignored.
        let ambiguous = b"!\n! Copyright\n!\n      FUNCTION EXPINT(n, x)\n      END FUNCTION\n";
        assert_eq!(detect(ambiguous), SourceForm::Fixed);
        assert_eq!(
            detect_path(Path::new("expint.f90"), ambiguous),
            SourceForm::Free
        );
        assert_eq!(
            detect_path(Path::new("expint.F90"), ambiguous),
            SourceForm::Free
        );
        // `.f`/`.F` really can be either, so those still go to the detector.
        assert_eq!(
            detect_path(Path::new("dlasrt2.f"), ambiguous),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_path(Path::new("legacy.F"), ambiguous),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_path(Path::new("modern.f"), b"module m\nend module m\n"),
            SourceForm::Free
        );
    }

    #[test]
    fn fixed_comments_and_unsure_input_default_to_fixed() {
        assert_eq!(detect(b"* comment\n"), SourceForm::Fixed);
        assert_eq!(detect(b"C comment\n"), SourceForm::Fixed);
        assert_eq!(detect(b"      x = 1\n"), SourceForm::Fixed);
        assert_eq!(detect(b"! comment\n\n"), SourceForm::Fixed);
    }

    #[test]
    fn free_form_signals_include_continuations_and_module_headers() {
        assert_eq!(detect(b"program p\nx = 1 &\n & y\nend\n"), SourceForm::Free);
        assert_eq!(detect(b"MODULE m\nend module m\n"), SourceForm::Free);
    }

    #[test]
    fn control_characters_are_unsure_and_do_not_block_later_evidence() {
        assert_eq!(detect(b"\x0c\nMODULE m\n"), SourceForm::Free);
    }

    #[test]
    fn tab_indented_code_is_handled_by_ltab2sp() {
        assert_eq!(detect(b"\tprogram p\n"), SourceForm::Fixed);
    }

    #[test]
    fn preprocessor_continuations_are_skipped() {
        assert_eq!(
            detect(b"#if defined(X)\\\nMODULE m\n#endif\n"),
            SourceForm::Fixed
        );
        assert_eq!(
            detect(b"#if defined(X)\nMODULE m\n#endif\n"),
            SourceForm::Free
        );
    }
}
