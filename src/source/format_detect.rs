//! Detection of fixed- versus free-form Fortran input.
//!
//! The raw detector is a byte-for-byte port of findent 4.3.7's
//! `determine_fix_or_free` path. Path-aware detection keeps the filename prior
//! but adds a conservative confirmation pass before accepting free form, so a
//! strong fixed-form signature can veto a weak or suffix-only free verdict.

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
/// A modern suffix therefore remains a strong free-form prior, while bare
/// `.f`/`.F` still goes through findent's detector. Before a free verdict is
/// returned, an allocation-free confirmation scan looks for syntax that is
/// unambiguously or very strongly fixed-form. This catches false free results
/// without reintroducing the known `.f90` false-fixed cases.
pub fn detect_path(path: &std::path::Path, source: &[u8]) -> SourceForm {
    let ambiguous_suffix = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("f"));
    let candidate = if ambiguous_suffix {
        detect(source)
    } else {
        SourceForm::Free
    };
    if candidate == SourceForm::Free && has_strong_fixed_form_evidence(source) {
        SourceForm::Fixed
    } else {
        candidate
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

/// Check a provisional free-form verdict for signatures that free form cannot
/// plausibly explain. This is intentionally stricter than `detect`: the raw
/// detector must stay findent-compatible, while this pass is only allowed to
/// veto a free candidate when the evidence is strong.
///
/// The pass is allocation-free and examines the same maximum 4000 physical
/// lines as the raw detector. In the common case it is a handful of byte tests
/// per line and can exit as soon as a fixed signature is seen.
fn has_strong_fixed_form_evidence(source: &[u8]) -> bool {
    let mut previous_code = None;
    let mut previous_free_continuation = false;

    for raw_line in source.split_inclusive(|byte| *byte == b'\n').take(4000) {
        let mut line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        line = line.strip_suffix(b"\r").unwrap_or(line);
        line = rtrim(line);
        if line.is_empty() {
            continue;
        }

        // Directives and comment-only lines may appear between parts of a
        // free-form continuation, so do not let them clear continuation state.
        if preprocessor_kind(line) != PreprocessorKind::Other || line.first() == Some(&b'!') {
            continue;
        }

        if !previous_free_continuation
            && (fixed_comment_signature(line) || fixed_continuation_signature(line, previous_code))
        {
            return true;
        }

        previous_free_continuation = free_line_continues(line);
        previous_code = Some(line);
    }
    false
}

fn fixed_comment_signature(line: &[u8]) -> bool {
    let Some(&first) = line.first() else {
        return false;
    };
    match first {
        // A leading `*` can be a valid free-form continuation token, so the
        // caller only asks this when the preceding source line did not end in
        // a free-form continuation ampersand.
        b'*' => true,
        b'c' | b'C' | b'd' | b'D' => {
            let tail = &line[1..];
            // Fixed-form directive sentinels such as C$OMP/D$OMP are invalid
            // as ordinary free-form source.
            if tail.first() == Some(&b'$') {
                return true;
            }
            let body = trim_left(tail);
            // `C text` / `D text` is fixed-form comment/debug syntax. Keep
            // assignments such as `C = 1` free-form by requiring an
            // alphanumeric first character after the separating whitespace.
            body.len() < tail.len() && body.first().is_some_and(u8::is_ascii_alphanumeric)
        }
        _ => false,
    }
}

fn fixed_continuation_signature(line: &[u8], previous_code: Option<&[u8]>) -> bool {
    if line.len() <= 6
        || !line[..5]
            .iter()
            .all(|byte| *byte == b' ' || byte.is_ascii_digit())
    {
        return false;
    }
    let marker = line[5];
    if matches!(marker, b' ' | b'\t' | b'0') || trim_left(&line[6..]).is_empty() {
        return false;
    }

    // Column-6 `&` without a preceding free-form trailing `&` is the clearest
    // fixed-form continuation signature.
    if marker == b'&' {
        return true;
    }

    if marker.is_ascii_digit() {
        // `     1continue` cannot be a free-form statement label because a
        // label must be separated from its statement. If whitespace follows
        // the digit, only use it when the preceding line is lexically
        // incomplete without continuation; that avoids rejecting valid free
        // lines such as `     1 continue`.
        if line[6].is_ascii_whitespace() {
            return previous_code.is_some_and(line_requires_continuation);
        }
        return true;
    }
    false
}

fn line_requires_continuation(line: &[u8]) -> bool {
    let code = rtrim(free_code_prefix(line));
    if code.last() == Some(&b'&') {
        return false;
    }
    code.last()
        .is_some_and(|byte| matches!(byte, b',' | b'+' | b'-' | b'*' | b'=' | b'(' | b'%'))
}

fn free_line_continues(line: &[u8]) -> bool {
    rtrim(free_code_prefix(line)).last() == Some(&b'&')
}

fn free_code_prefix(line: &[u8]) -> &[u8] {
    let mut quote = None;
    let mut index = 0;
    while index < line.len() {
        let byte = line[index];
        if let Some(delimiter) = quote {
            if byte == delimiter {
                if line.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte == b'!' {
            return &line[..index];
        }
        index += 1;
    }
    line
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
    fn modern_suffixes_default_to_free_when_layout_is_ambiguous() {
        // Reduced from Q-E `Modules/expint.f90`: a comment block, then a
        // statement indented past column six. Nothing in it is decisive, so
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
    fn free_candidate_is_confirmed_against_later_fixed_evidence() {
        // The first line starts with a findent ckey and therefore makes the raw
        // detector return FREE immediately, even though it is a valid fixed-form
        // column-1 comment. The later C-comment supplies corroborating fixed
        // evidence for the path-aware confirmation pass.
        let source = b"CALL THIS ROUTINE ONLY ON UNIX\nC legacy implementation\n      END\n";
        assert_eq!(detect(source), SourceForm::Free);
        assert_eq!(
            detect_path(Path::new("legacy.f"), source),
            SourceForm::Fixed
        );
    }

    #[test]
    fn modern_suffix_can_be_overridden_by_strong_fixed_evidence() {
        assert_eq!(
            detect_path(
                Path::new("legacy.F90"),
                b"C legacy fixed-form comment\n      PROGRAM P\n      END\n"
            ),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_path(
                Path::new("legacy.f90"),
                b"      X = A +\n     &  B\n      END\n"
            ),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_path(
                Path::new("legacy.f90"),
                b"      X = A +\n     1  B\n      END\n"
            ),
            SourceForm::Fixed
        );
    }

    #[test]
    fn confirmation_does_not_reject_valid_free_form_lookalikes() {
        assert_eq!(
            detect_path(Path::new("modern.F90"), b"C = 1\ncall foo()\n"),
            SourceForm::Free
        );
        assert_eq!(
            detect_path(Path::new("modern.F90"), b"x = a &\n* b\n"),
            SourceForm::Free
        );
        assert_eq!(
            detect_path(
                Path::new("modern.F90"),
                b"x = a & ! trailing comment\n     & + b\n"
            ),
            SourceForm::Free
        );
        assert_eq!(
            detect_path(Path::new("modern.F90"), b"     1 continue\nend\n"),
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
