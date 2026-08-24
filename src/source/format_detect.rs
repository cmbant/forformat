//! Detection of fixed- versus free-form Fortran input.
//!
//! The automatic detector accumulates positive source-form evidence instead of
//! returning on the first suggestive line. Named sources also keep the filename
//! prior that avoids false-fixed results on ordinary modern `.f90` sources.
//! The original findent `determine_fix_or_free` port remains below as a
//! compatibility baseline for focused regression tests.

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

#[derive(Debug, Default, Clone, Copy)]
struct FormEvidence {
    strong_free: bool,
    strong_fixed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CppActivity {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
struct CppFrame {
    parent: CppActivity,
    previous_taken: Option<bool>,
    condition: Option<bool>,
}

impl CppFrame {
    fn activity(self) -> CppActivity {
        let branch = match (self.previous_taken, self.condition) {
            (Some(true), _) | (_, Some(false)) => CppActivity::Inactive,
            (Some(false), Some(true)) => CppActivity::Active,
            _ => CppActivity::Unknown,
        };
        match (self.parent, branch) {
            (CppActivity::Inactive, _) | (_, CppActivity::Inactive) => CppActivity::Inactive,
            (CppActivity::Active, activity) => activity,
            (CppActivity::Unknown, _) => CppActivity::Unknown,
        }
    }

    fn advance(&mut self, condition: Option<bool>) {
        self.previous_taken = match (self.previous_taken, self.condition) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
        self.condition = condition;
    }
}

/// Determine the source form of a named file.
///
/// Modern suffixes remain a strong free-form prior because content alone is
/// genuinely ambiguous for many valid free-form files whose statements begin
/// in column seven. Strong contradictory fixed-form evidence always wins.
pub fn detect_path(path: &std::path::Path, source: &[u8]) -> SourceForm {
    let evidence = collect_evidence(source);
    if evidence.strong_fixed {
        return SourceForm::Fixed;
    }
    if has_modern_free_suffix(path) || evidence.strong_free {
        SourceForm::Free
    } else {
        SourceForm::Fixed
    }
}

/// Determine the source form using source bytes only.
///
/// Anonymous input is formatted when the bytes contain positive free-form
/// evidence and no strong fixed-form evidence. Genuinely ambiguous input still
/// defaults to fixed so automatic mode cannot silently reinterpret it.
pub fn detect(source: &[u8]) -> SourceForm {
    let evidence = collect_evidence(source);
    if evidence.strong_fixed {
        SourceForm::Fixed
    } else if evidence.strong_free {
        SourceForm::Free
    } else {
        SourceForm::Fixed
    }
}

fn has_modern_free_suffix(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| !extension.eq_ignore_ascii_case("f"))
}

fn collect_evidence(source: &[u8]) -> FormEvidence {
    let mut evidence = FormEvidence::default();
    let mut tentative_evidence = FormEvidence::default();
    let mut previous_code = None;
    let mut previous_free_continuation = false;
    let mut tentative_previous_code = None;
    let mut tentative_previous_free_continuation = false;
    let mut continued_directive = None;
    let mut cpp_stack = Vec::new();

    for raw_line in source.split_inclusive(|byte| *byte == b'\n') {
        let mut line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        line = line.strip_suffix(b"\r").unwrap_or(line);
        line = rtrim(line);

        if let Some(kind) = continued_directive {
            continued_directive = preprocessor_line_continues(kind, line).then_some(kind);
            continue;
        }

        let preprocessor = preprocessor_kind(line);
        let is_preprocessor_line = match preprocessor {
            PreprocessorKind::Cpp => is_cpp_directive_line(line),
            PreprocessorKind::Coco | PreprocessorKind::Fypp => true,
            PreprocessorKind::Other => false,
        };

        // A source line that is already lexically incomplete without
        // continuation makes any legal nonblank/nonzero column-6 marker strong
        // fixed-form evidence. Actual preprocessor directives are excluded:
        // they disappear before Fortran parsing and may be indented through
        // column six without becoming source continuation lines.
        if !is_preprocessor_line
            && cpp_activity(&cpp_stack) == CppActivity::Active
            && !previous_free_continuation
            && previous_code.is_some_and(line_requires_continuation)
            && fixed_continuation_signature(line, previous_code)
        {
            evidence.strong_fixed = true;
        }

        if is_preprocessor_line {
            if preprocessor == PreprocessorKind::Cpp {
                update_cpp_activity(line, &mut cpp_stack);
            }
            continued_directive =
                preprocessor_line_continues(preprocessor, line).then_some(preprocessor);
            continue;
        }

        let (line_evidence, line_previous_code, line_previous_free_continuation) =
            match cpp_activity(&cpp_stack) {
                CppActivity::Active => (
                    &mut evidence,
                    &mut previous_code,
                    &mut previous_free_continuation,
                ),
                CppActivity::Unknown => (
                    &mut tentative_evidence,
                    &mut tentative_previous_code,
                    &mut tentative_previous_free_continuation,
                ),
                CppActivity::Inactive => continue,
            };

        if line.is_empty() {
            continue;
        }

        let free_comment = trim_left(line).first() == Some(&b'!');
        if !*line_previous_free_continuation
            && (fixed_comment_signature(line)
                || fixed_continuation_signature(line, *line_previous_code))
        {
            line_evidence.strong_fixed = true;
        }
        if !free_comment && strong_free_form_signature(line) {
            line_evidence.strong_free = true;
        }

        // Comments may appear between physical lines of a free continuation;
        // they do not terminate or replace the surrounding continuation state.
        if free_comment {
            continue;
        }

        *line_previous_free_continuation = free_line_continues(line);
        *line_previous_code = Some(line);
    }

    if tentative_evidence.strong_free && !tentative_evidence.strong_fixed {
        evidence.strong_free = true;
    }
    evidence
}

fn cpp_activity(stack: &[CppFrame]) -> CppActivity {
    stack
        .last()
        .copied()
        .map(CppFrame::activity)
        .unwrap_or(CppActivity::Active)
}

fn update_cpp_activity(line: &[u8], stack: &mut Vec<CppFrame>) {
    let Some((keyword, rest)) = cpp_directive(line) else {
        return;
    };
    match keyword {
        b"if" => {
            let parent = cpp_activity(stack);
            let condition = if preprocessor_line_continues(PreprocessorKind::Cpp, line) {
                None
            } else {
                literal_cpp_condition(rest)
            };
            stack.push(CppFrame {
                parent,
                previous_taken: Some(false),
                condition,
            });
        }
        b"ifdef" | b"ifndef" => {
            let parent = cpp_activity(stack);
            stack.push(CppFrame {
                parent,
                previous_taken: Some(false),
                condition: None,
            });
        }
        b"else" => {
            if let Some(frame) = stack.last_mut() {
                frame.advance(Some(true));
            }
        }
        b"elif" => {
            let condition = if preprocessor_line_continues(PreprocessorKind::Cpp, line) {
                None
            } else {
                literal_cpp_condition(rest)
            };
            if let Some(frame) = stack.last_mut() {
                frame.advance(condition);
            }
        }
        b"elifdef" | b"elifndef" => {
            if let Some(frame) = stack.last_mut() {
                frame.advance(None);
            }
        }
        b"endif" => {
            let _ = stack.pop();
        }
        _ => {}
    }
}

fn cpp_directive(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let line = trim_left(line);
    let rest = line.strip_prefix(b"#")?;
    let rest = trim_left(rest);
    let keyword_len = rest
        .iter()
        .position(|byte| !byte.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    Some((&rest[..keyword_len], trim_left(&rest[keyword_len..])))
}

fn is_cpp_directive_line(line: &[u8]) -> bool {
    let trimmed = trim_left(line);
    let Some(rest) = trimmed.strip_prefix(b"#") else {
        return false;
    };

    // Away from the fixed-form continuation column, preserve the detector's
    // historical broad `#...` treatment. Exactly in column six, however, `#`
    // is also a legal fixed continuation marker, so only actual CPP directive
    // spellings may bypass the fixed-form evidence scan.
    if line.len() - trimmed.len() != 5 {
        return true;
    }

    let rest = trim_left(rest);
    if rest.is_empty() || rest.first().is_some_and(u8::is_ascii_digit) {
        return true;
    }
    let keyword_len = rest
        .iter()
        .position(|byte| !byte.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    matches!(
        &rest[..keyword_len],
        b"if"
            | b"ifdef"
            | b"ifndef"
            | b"elif"
            | b"elifdef"
            | b"elifndef"
            | b"else"
            | b"endif"
            | b"include"
            | b"define"
            | b"undef"
            | b"line"
            | b"error"
            | b"warning"
            | b"pragma"
            | b"import"
            | b"embed"
            | b"assert"
            | b"unassert"
            | b"ident"
            | b"sccs"
            | b"region"
            | b"endregion"
    )
}

fn literal_cpp_condition(rest: &[u8]) -> Option<bool> {
    let token_len = rest
        .iter()
        .position(|byte| byte.is_ascii_whitespace())
        .unwrap_or(rest.len());
    match &rest[..token_len] {
        b"0" => Some(false),
        b"1" => Some(true),
        _ => None,
    }
}

fn preprocessor_line_continues(kind: PreprocessorKind, line: &[u8]) -> bool {
    match kind {
        PreprocessorKind::Cpp => line.last() == Some(&b'\\'),
        PreprocessorKind::Coco | PreprocessorKind::Fypp => line.last() == Some(&b'&'),
        PreprocessorKind::Other => false,
    }
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
            if tail.first() == Some(&b'$') {
                return true;
            }
            let body = trim_left(tail);
            if body.len() < tail.len() && body.first().is_some_and(u8::is_ascii_alphanumeric) {
                return true;
            }
            // Decorative legacy comments such as C-----, C==== and D***** are
            // common and cannot be valid free-form statements. Require at
            // least two punctuation characters so assignments such as C=-1
            // and pointer assignments such as C=>D remain free lookalikes.
            body.len() >= 2 && body.iter().all(u8::is_ascii_punctuation)
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

    if marker == b'&' {
        return true;
    }

    if marker.is_ascii_digit() {
        if line[6].is_ascii_whitespace() {
            return previous_code.is_some_and(|previous| {
                line_requires_continuation(previous)
                    || fixed_list_directed_continuation(previous, &line[6..])
            });
        }
        if free_statement_label_extends_past_column_six(line) {
            return previous_code.is_some_and(line_requires_continuation);
        }
        return true;
    }

    // An alphabetic marker is indistinguishable from an ordinary free-form
    // statement indented exactly five spaces ("     print ...") unless the
    // preceding line proves continuation is required. A comma in the statement
    // field can also visibly extend a complete list-directed PRINT/READ.
    if marker.is_ascii_alphabetic() {
        return previous_code.is_some_and(|previous| {
            line_requires_continuation(previous)
                || fixed_list_directed_continuation(previous, &line[6..])
        });
    }

    // `!` in column six is also an indented free-form comment, so keep its
    // stronger requirement that the previous line itself need continuation.
    if marker == b'!' {
        return previous_code.is_some_and(line_requires_continuation);
    }

    // Other nonblank/nonzero characters cannot start a valid standalone
    // free-form Fortran statement in column six, so they are strong fixed
    // continuation evidence when followed by a statement field.
    true
}

fn fixed_list_directed_continuation(previous_code: &[u8], statement_field: &[u8]) -> bool {
    terminal_star_is_list_directed_io(rtrim(free_code_prefix(previous_code)))
        && trim_left(statement_field).first() == Some(&b',')
}

fn strong_free_form_signature(line: &[u8]) -> bool {
    // Tabs in the fixed label/continuation field have compiler-specific legacy
    // meanings, so do not use byte-column evidence on those lines.
    if line.iter().take(6).any(|byte| *byte == b'\t') {
        return false;
    }

    for index in 0..=4 {
        if line.len() <= index {
            break;
        }
        if !line[..index]
            .iter()
            .all(|byte| *byte == b' ' || byte.is_ascii_digit())
        {
            continue;
        }
        let byte = line[index];
        if (byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'%'))
            && (index > 0 || !matches!(byte, b'c' | b'C' | b'd' | b'D'))
        {
            return true;
        }
        if byte == b'&' {
            return true;
        }
    }

    let code = rtrim(free_code_prefix(line));
    if code.last() == Some(&b'&') {
        let index = code.len() - 1;
        if index != 5 {
            return true;
        }
    }
    false
}

fn free_statement_label_extends_past_column_six(line: &[u8]) -> bool {
    let label = trim_left(line);
    let leading = line.len() - label.len();
    let digits = label
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    (1..=5).contains(&digits)
        && leading <= 5
        && leading + digits > 6
        && label.get(digits).is_some_and(u8::is_ascii_whitespace)
        && !trim_left(&label[digits..]).is_empty()
}

fn line_requires_continuation(line: &[u8]) -> bool {
    let code = rtrim(free_code_prefix(line));
    let Some(&last) = code.last() else {
        return false;
    };
    match last {
        b'&' => false,
        b'*' => terminal_star_requires_operand(code),
        b',' | b'+' | b'-' | b'=' | b'(' | b'%' => true,
        _ => false,
    }
}

fn terminal_star_requires_operand(code: &[u8]) -> bool {
    !terminal_star_is_list_directed_io(code)
}

fn terminal_star_is_list_directed_io(code: &[u8]) -> bool {
    let statement_start = super::scanner::split_statement_ranges(code)
        .last()
        .map_or(0, |range| range.start);

    let mut tokens = super::scanner::iter_tokens(&code[statement_start..]);
    let Some(mut token) = tokens.next() else {
        return false;
    };

    if token.text.len() <= 5 && token.text.iter().all(u8::is_ascii_digit) {
        let Some(next) = tokens.next() else {
            return false;
        };
        token = next;
    }

    if token.text.eq_ignore_ascii_case(b"if") {
        let Some(open) = tokens.next() else {
            return false;
        };
        if open.text != b"(" {
            return false;
        }

        let mut depth = 1usize;
        for condition_token in tokens.by_ref() {
            if condition_token.text == b"(" {
                depth += 1;
            } else if condition_token.text == b")" {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
        if depth != 0 {
            return false;
        }

        let Some(action) = tokens.next() else {
            return false;
        };
        token = action;
    }

    if ![b"print".as_slice(), b"read"]
        .iter()
        .any(|keyword| token.text.eq_ignore_ascii_case(keyword))
    {
        return false;
    }

    let Some(star) = tokens.next() else {
        return false;
    };
    star.text == b"*" && tokens.next().is_none()
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

/// Exact findent `determine_fix_or_free` compatibility baseline.
///
/// Automatic policy deliberately does not use its first-decisive-line/EOF
/// result directly; retaining it makes the historical behavior independently
/// testable.
#[cfg(test)]
fn detect_findent_compatible(source: &[u8]) -> SourceForm {
    let mut preprocessor = PreprocessorKind::Other;
    let mut p_more = false;
    let mut skip = false;

    for raw_line in source.split_inclusive(|byte| *byte == b'\n').take(4000) {
        let mut line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
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

#[cfg(test)]
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

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    Free,
    Fixed,
    Unsure,
}

#[cfg(test)]
fn classify_line(line: &[u8]) -> Classification {
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

#[cfg(test)]
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

#[cfg(test)]
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
    use super::{detect, detect_findent_compatible, detect_path, SourceForm};
    use std::path::Path;

    #[test]
    fn modern_suffixes_supply_free_evidence_when_content_is_ambiguous() {
        let ambiguous = b"!\n! Copyright\n!\n      FUNCTION EXPINT(n, x)\n      END FUNCTION\n";
        assert_eq!(detect(ambiguous), SourceForm::Fixed);
        assert_eq!(detect_findent_compatible(ambiguous), SourceForm::Fixed);
        assert_eq!(
            detect_path(Path::new("expint.f90"), ambiguous),
            SourceForm::Free
        );
        assert_eq!(
            detect_path(Path::new("expint.F90"), ambiguous),
            SourceForm::Free
        );
        assert_eq!(
            detect_path(Path::new("dlasrt2.f"), ambiguous),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_path(Path::new("legacy.F"), ambiguous),
            SourceForm::Fixed
        );
    }

    #[test]
    fn stdin_uses_positive_free_evidence_instead_of_findent_first_match() {
        let source = b"! leading comment\nmodule m\ncontains\nsubroutine s\nend subroutine s\nend module m\n";
        assert_eq!(detect(source), SourceForm::Free);

        let ambiguous = b"! leading comment\n      function f()\n      end function f\n";
        assert_eq!(detect(ambiguous), SourceForm::Fixed);
    }

    #[test]
    fn fixed_evidence_overrides_free_evidence_and_suffixes() {
        let source = b"CALL THIS ROUTINE ONLY ON UNIX\nC legacy implementation\n      END\n";
        assert_eq!(detect_findent_compatible(source), SourceForm::Free);
        assert_eq!(detect(source), SourceForm::Fixed);
        assert_eq!(
            detect_path(Path::new("legacy.f90"), source),
            SourceForm::Fixed
        );
    }

    #[test]
    fn modern_suffix_can_be_overridden_by_fixed_continuations() {
        for source in [
            b"      X = A +\n     &  B\n      END\n".as_slice(),
            b"      X = A +\n     1  B\n      END\n".as_slice(),
            b"      x = 1 +\n     12 * y\n      END\n".as_slice(),
            b"      x = 1\n     + + y\n      END\n".as_slice(),
            b"      x = a *\n     a b\n      END\n".as_slice(),
            b"      x = 7H;print  *\n     a y\n      END\n".as_slice(),
            b"      PRINT *\n     x, value\n      END\n".as_slice(),
        ] {
            assert_eq!(
                detect_path(Path::new("legacy.f90"), source),
                SourceForm::Fixed
            );
        }
    }

    #[test]
    fn column_six_hash_is_fixed_unless_it_starts_a_cpp_directive() {
        assert_eq!(
            detect_path(
                Path::new("legacy.f90"),
                b"      x = 1\n     #+ 2\n      end\n"
            ),
            SourceForm::Fixed
        );

        for source in [
            b"     #define VALUE 1\nmodule m\nend module m\n".as_slice(),
            b"     #if 1\nmodule m\n     #endif\nend module m\n".as_slice(),
            b"     # 1 \"source.f90\"\nmodule m\nend module m\n".as_slice(),
        ] {
            assert_eq!(
                detect_path(Path::new("modern.f90"), source),
                SourceForm::Free
            );
        }
    }

    #[test]
    fn every_printable_nonblank_nonzero_column_six_marker_is_detected_when_required() {
        for marker in 0x21u8..=0x7e {
            if marker == b'0' {
                continue;
            }
            let mut source = b"      x = 1 +\n     ".to_vec();
            source.push(marker);
            source.extend_from_slice(b" y\n      end\n");
            assert_eq!(
                detect_path(Path::new("legacy.f90"), &source),
                SourceForm::Fixed,
                "marker {:?}",
                marker as char
            );
        }
    }

    #[test]
    fn decorative_fixed_comments_are_strong_evidence() {
        for banner in [
            b"C----------------\n".as_slice(),
            b"C================\n".as_slice(),
            b"D****************\n".as_slice(),
        ] {
            let mut source = banner.to_vec();
            source.extend_from_slice(b"      PROGRAM P\n      END\n");
            assert_eq!(
                detect_path(Path::new("legacy.F90"), &source),
                SourceForm::Fixed
            );
        }
        assert_eq!(detect(b"C=-1\nend\n"), SourceForm::Free);
        assert_eq!(detect(b"C=>D\nend\n"), SourceForm::Free);
    }

    #[test]
    fn free_form_lookalikes_remain_free() {
        assert_eq!(
            detect_path(Path::new("modern.F90"), b"C = 1\ncall foo()\nend\n"),
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
        assert_eq!(detect(b"     print *, 1\nend\n"), SourceForm::Free);
    }

    #[test]
    fn terminal_list_directed_star_does_not_trigger_fixed_continuation() {
        for source in [
            b"program p\n  if (.true.) then\n     print *\n     write(*,*) \"x\"\n  end if\nend program p\n".as_slice(),
            b"program p\n  if (.true.) print *\n     write(*,*) \"x\"\nend program p\n".as_slice(),
            b"program p\n  x = 1; print *\n     write(*,*) \"x\"\nend program p\n".as_slice(),
            b"program p\n     PRINT*\n     write(*,*) \"x\"\nend program p\n".as_slice(),
            b"program p\n  10 read *\n     print *, \"done\"\nend program p\n".as_slice(),
        ] {
            assert_eq!(detect(source), SourceForm::Free);
            assert_eq!(detect_path(Path::new("p.f"), source), SourceForm::Free);
        }
    }

    #[test]
    fn terminal_multiplication_star_still_requires_an_operand() {
        let source = b"program p\n  x = print *\n     y z\nend program p\n";
        assert_eq!(detect(source), SourceForm::Fixed);
    }

    #[test]
    fn free_labels_that_cross_column_six_are_not_fixed_continuations() {
        for source in [
            b"program p\n    111  call foo()\nend program p\n".as_slice(),
            b"program p\n   1011  format(i0)\nend program p\n".as_slice(),
            b"program p\n  98765  continue\nend program p\n".as_slice(),
        ] {
            assert_eq!(detect(source), SourceForm::Free);
        }
    }

    #[test]
    fn cpp_literal_conditions_contribute_only_active_branch_evidence() {
        let inactive =
            b"#if 0\nC legacy fixed-form text in disabled branch\n#endif\nmodule m\nend module m\n";
        assert_eq!(detect(inactive), SourceForm::Free);

        let active =
            b"#if 1\nC legacy fixed-form text in active branch\n#endif\nmodule m\nend module m\n";
        assert_eq!(detect(active), SourceForm::Fixed);

        let active_else = b"#if 0\nC ignored\n#else\nmodule m\n#endif\n";
        assert_eq!(detect(active_else), SourceForm::Free);
    }

    #[test]
    fn cpp_unknown_free_only_evidence_is_tentatively_promoted() {
        let source = b"#ifdef FEATURE\nmodule m\nend module m\n#endif\n";
        assert_eq!(detect(source), SourceForm::Free);
    }

    #[test]
    fn cpp_unknown_fixed_only_evidence_stays_fixed() {
        let source = b"#ifdef FEATURE\nC legacy fixed-form text\n      END\n#endif\n";
        assert_eq!(detect(source), SourceForm::Fixed);
    }

    #[test]
    fn cpp_unknown_mixed_alternatives_stay_fixed() {
        let source = b"#ifdef FEATURE\nmodule m\n#else\nC legacy text\n#endif\n";
        assert_eq!(detect(source), SourceForm::Fixed);
    }

    #[test]
    fn cpp_unknown_conditions_are_conservative_and_nest() {
        let nested = b"#if 1\n#ifdef MAYBE\nC ignored unknown branch\n#endif\nmodule m\n#endif\n";
        assert_eq!(detect(nested), SourceForm::Free);

        let nested_inactive =
            b"#ifdef MAYBE\n#if 0\nC disabled fixed text\n#endif\nmodule m\n#endif\n";
        assert_eq!(detect(nested_inactive), SourceForm::Free);

        let nested_else =
            b"#ifdef MAYBE\n#if 0\nC disabled fixed text\n#else\nmodule m\n#endif\n#endif\n";
        assert_eq!(detect(nested_else), SourceForm::Free);

        let nested_mixed =
            b"#ifdef MAYBE\n#ifdef INNER\nmodule m\n#else\nC legacy text\n#endif\n#endif\n";
        assert_eq!(detect(nested_mixed), SourceForm::Fixed);
    }

    #[test]
    fn cpp_elif_after_taken_literal_branch_stays_inactive() {
        for directive in ["#elif FEATURE", "#elifdef FEATURE", "#elifndef FEATURE"] {
            let source = format!("#if 1\n      END\n{directive}\nmodule m\n#endif\n");
            assert_eq!(detect(source.as_bytes()), SourceForm::Fixed, "{directive}");
        }
    }

    #[test]
    fn cpp_elif_after_untaken_literal_branch_can_be_unknown() {
        for directive in ["#elif FEATURE", "#elifdef FEATURE", "#elifndef FEATURE"] {
            let source = format!("#if 0\nC disabled fixed text\n{directive}\nmodule m\n#endif\n");
            assert_eq!(detect(source.as_bytes()), SourceForm::Free, "{directive}");
        }
    }

    #[test]
    fn cpp_true_elif_makes_following_else_inactive() {
        let source =
            b"#ifdef FEATURE\n! ambiguous\n#elif 1\nmodule m\n#else\nC unreachable fixed\n#endif\n";
        assert_eq!(detect(source), SourceForm::Free);
    }

    #[test]
    fn continued_preprocessor_directives_are_skipped() {
        let source =
            b"#define CONT \\\n     &\nprogram p\ninteger :: x\nx = 1 CONT\n+ 2\nend program p\n";
        assert_eq!(detect(source), SourceForm::Free);
    }

    #[test]
    fn evidence_scan_is_not_limited_to_findents_first_4000_lines() {
        let mut source = Vec::new();
        for _ in 0..4001 {
            source.extend_from_slice(b"! comment\n");
        }
        source.extend_from_slice(b"module m\nend module m\n");
        assert_eq!(detect_findent_compatible(&source), SourceForm::Fixed);
        assert_eq!(detect(&source), SourceForm::Free);
    }

    #[test]
    fn ambiguous_and_fixed_only_input_stays_fixed() {
        assert_eq!(detect(b"* comment\n"), SourceForm::Fixed);
        assert_eq!(detect(b"C comment\n"), SourceForm::Fixed);
        assert_eq!(detect(b"      x = 1\n"), SourceForm::Fixed);
        assert_eq!(detect(b"! comment\n\n"), SourceForm::Fixed);
        assert_eq!(detect(b"call foo()\n"), SourceForm::Fixed);
    }

    #[test]
    fn findent_baseline_is_preserved_for_oracle_coverage() {
        assert_eq!(
            detect_findent_compatible(b"program p\nx = 1 &\n & y\nend\n"),
            SourceForm::Free
        );
        assert_eq!(
            detect_findent_compatible(b"MODULE m\nend module m\n"),
            SourceForm::Free
        );
        assert_eq!(
            detect_findent_compatible(b"\x0c\nMODULE m\n"),
            SourceForm::Free
        );
        assert_eq!(
            detect_findent_compatible(b"\tprogram p\n"),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_findent_compatible(b"#if defined(X)\\\nMODULE m\n#endif\n"),
            SourceForm::Fixed
        );
        assert_eq!(
            detect_findent_compatible(b"#if defined(X)\nMODULE m\n#endif\n"),
            SourceForm::Free
        );
    }
}
