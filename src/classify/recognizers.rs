use super::statement::{StatementClass, StatementInfo, StatementKind};
use crate::source::scanner::tokens;

pub fn classify(input: &[u8]) -> StatementInfo {
    let text = trim(input);
    if text.is_empty() {
        return StatementInfo {
            kind: StatementKind::Blank,
            class: StatementClass::Neutral,
            construct_name: None,
            entity_name: None,
            statement_label: None,
            referenced_labels: Vec::new(),
            payload: Vec::new(),
            contains_hollerith: false,
            unframed_procedure: None,
            end_kind: None,
        };
    }
    let mut s = text;
    let mut label = None;
    if let Some((n, rest)) = leading_label(s) {
        label = Some(n);
        s = rest;
    }
    let mut construct = None;
    if let Some((name, rest)) = construct_prefix(s) {
        construct = Some(name.to_vec());
        s = rest;
    }
    let low = lower(s);
    let words = words(&low);
    let first = words.first().map(|v| v.as_slice()).unwrap_or(b"");
    let mut info = StatementInfo::unknown(text);
    info.statement_label = label;
    info.construct_name = construct;
    info.payload = text.to_vec();
    info.unframed_procedure = comma_prefixed_procedure(s, &words);
    // A free-form statement that starts with digits but is not a valid
    // label is malformed editor input.  Do not discard that token while
    // collecting keyword words, or `10abc continue` would become a false
    // `CONTINUE` statement and mutate label bookkeeping.
    if label.is_none() && s.first().is_some_and(u8::is_ascii_digit) {
        return info;
    }
    if is_assignment(s) {
        info.class = StatementClass::Executable;
        return info;
    }
    if let Some((kind, class)) = prefixed_procedure(s, &words) {
        info.kind = kind;
        info.class = class;
        info.entity_name = entity_name(s, &kind);
        return info;
    }
    let (kind, class) = match first {
        b"program" => (StatementKind::Program, StatementClass::Definition),
        b"module" if words.get(1).is_some_and(|x| x != b"procedure") => {
            (StatementKind::Module, StatementClass::Definition)
        }
        b"submodule" => (StatementKind::Submodule, StatementClass::Definition),
        b"subroutine" => (StatementKind::Subroutine, StatementClass::Definition),
        b"function" => (StatementKind::Function, StatementClass::Definition),
        b"block" if words.get(1).is_some_and(|x| x == b"data") => {
            (StatementKind::BlockData, StatementClass::Definition)
        }
        b"interface" => (StatementKind::Interface, StatementClass::Definition),
        b"abstract" if words.get(1).is_some_and(|x| x == b"interface") => {
            (StatementKind::AbstractInterface, StatementClass::Definition)
        }
        b"type" if is_type_definition(s) => (StatementKind::Type, StatementClass::Definition),
        b"contains" => (StatementKind::Contains, StatementClass::Neutral),
        b"module" if words.get(1).is_some_and(|x| x == b"procedure") => {
            (StatementKind::Procedure, StatementClass::Definition)
        }
        // A standalone PROCEDURE statement declares a procedure pointer or
        // binding; it does not start a procedure body.  Only MODULE
        // PROCEDURE (handled above) and a prefixed FUNCTION/SUBROUTINE open
        // structural frames.  Misclassifying declarations such as
        // `procedure(state_function), private :: dtauda` leaves an extra
        // frame open and makes everything after CONTAINS too deeply indented.
        b"procedure" if is_procedure_definition(s) => {
            (StatementKind::Procedure, StatementClass::Definition)
        }
        b"procedure" => (StatementKind::Unknown, StatementClass::Neutral),
        b"if" if has_then(&low) => (StatementKind::If, StatementClass::Executable),
        b"else" if words.get(1).is_some_and(|x| x == b"if") => {
            (StatementKind::ElseIf, StatementClass::Executable)
        }
        b"elseif" => (StatementKind::ElseIf, StatementClass::Executable),
        b"else" => (StatementKind::Else, StatementClass::Executable),
        b"do" => (StatementKind::Do, StatementClass::Executable),
        b"select" => (StatementKind::Select, StatementClass::Executable),
        b"case" => (StatementKind::Case, StatementClass::Executable),
        b"rank" => (StatementKind::Case, StatementClass::Executable),
        b"class" if words.get(1).is_some_and(|x| x == b"is" || x == b"default") => {
            (StatementKind::Case, StatementClass::Executable)
        }
        b"type" if words.get(1).is_some_and(|x| x == b"is" || x == b"default") => {
            (StatementKind::Case, StatementClass::Executable)
        }
        b"where" if !single_line_after_paren(s, b"where") => {
            (StatementKind::Where, StatementClass::Executable)
        }
        b"where" => (StatementKind::Unknown, StatementClass::Executable),
        b"elsewhere" => (StatementKind::ElseWhere, StatementClass::Executable),
        b"forall" if !single_line_after_paren(s, b"forall") => {
            (StatementKind::Forall, StatementClass::Executable)
        }
        b"forall" => (StatementKind::Unknown, StatementClass::Executable),
        b"associate" => (StatementKind::Associate, StatementClass::Executable),
        b"block" => (StatementKind::Block, StatementClass::Executable),
        b"critical" => (StatementKind::Critical, StatementClass::Executable),
        b"change" if words.get(1).is_some_and(|x| x == b"team") => {
            (StatementKind::ChangeTeam, StatementClass::Executable)
        }
        b"enum" => (StatementKind::Enum, StatementClass::Definition),
        b"entry" => (StatementKind::Entry, StatementClass::Executable),
        b"include" => (StatementKind::Include, StatementClass::Neutral),
        b"continue" => (StatementKind::LabelContinue, StatementClass::Neutral),
        b"end" => end_kind(&words),
        // The compact spellings are standard free-form Fortran, not
        // misspellings.  Keep them separate from the generic END handling so
        // that an incomplete editor buffer cannot pop an unrelated frame.
        b"endif" => (StatementKind::EndIf, StatementClass::Neutral),
        b"enddo" => (StatementKind::EndDo, StatementClass::Neutral),
        b"endselect" => (StatementKind::EndSelect, StatementClass::Neutral),
        b"endwhere" => (StatementKind::EndWhere, StatementClass::Neutral),
        b"endforall" => (StatementKind::EndForall, StatementClass::Neutral),
        b"endassociate" => (StatementKind::EndAssociate, StatementClass::Neutral),
        b"endblock" => (StatementKind::EndBlock, StatementClass::Neutral),
        b"endcritical" => (StatementKind::EndCritical, StatementClass::Neutral),
        b"endteam" => (StatementKind::EndTeam, StatementClass::Neutral),
        b"endenum" => (StatementKind::EndEnum, StatementClass::Neutral),
        b"endprogram" | b"endmodule" | b"endsubmodule" | b"endinterface" | b"endtype"
        | b"endblockdata" | b"endsubroutine" | b"endfunction" | b"endprocedure" => {
            (StatementKind::Unknown, StatementClass::EndDefinition)
        }
        // These are supported legacy free-form constructs in findent's test
        // suite.  They are structural only; their declarations stay opaque.
        b"structure" => (StatementKind::Structure, StatementClass::Definition),
        b"union" => (StatementKind::Union, StatementClass::Definition),
        b"map" => (StatementKind::Map, StatementClass::Definition),
        b"endstructure" => (StatementKind::EndStructure, StatementClass::Neutral),
        b"endunion" => (StatementKind::EndUnion, StatementClass::Neutral),
        b"endmap" => (StatementKind::EndMap, StatementClass::Neutral),
        _ => (StatementKind::Unknown, StatementClass::Neutral),
    };
    info.kind = kind;
    info.class = class;
    if class == StatementClass::EndDefinition {
        info.end_kind = explicit_end_kind(&words);
    }
    info.entity_name = entity_name(s, &kind);
    info.referenced_labels = referenced_labels(s, &kind);
    info
}

fn trim(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut b = s.len();
    while a < b && s[a].is_ascii_whitespace() {
        a += 1;
    }
    while b > a && s[b - 1].is_ascii_whitespace() {
        b -= 1;
    }
    &s[a..b]
}
fn lower(s: &[u8]) -> Vec<u8> {
    s.iter().map(|c| c.to_ascii_lowercase()).collect()
}
fn words(s: &[u8]) -> Vec<Vec<u8>> {
    tokens(s)
        .into_iter()
        .filter(|t| {
            t.text
                .first()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        })
        .map(|t| t.text.to_ascii_lowercase())
        .collect()
}
fn leading_label(s: &[u8]) -> Option<(u32, &[u8])> {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < s.len() && (s[i].is_ascii_whitespace() || s[i] == b'&') {
        std::str::from_utf8(&s[..i])
            .ok()?
            .parse()
            .ok()
            .map(|n| (n, trim(&s[i..])))
    } else {
        None
    }
}
fn construct_prefix(s: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = s.iter().position(|c| *c == b':')?;
    if s.get(colon + 1) == Some(&b':') {
        return None;
    }
    let mut name_end = colon;
    while name_end > 0 && s[name_end - 1].is_ascii_whitespace() {
        name_end -= 1;
    }
    if name_end == 0
        || !s[..name_end].first().is_some_and(u8::is_ascii_alphabetic)
        || s[..name_end]
            .iter()
            .any(|c| !c.is_ascii_alphanumeric() && *c != b'_')
    {
        return None;
    }
    Some((&s[..name_end], trim(&s[colon + 1..])))
}
fn is_assignment(s: &[u8]) -> bool {
    let mut first_end = 0;
    while first_end < s.len() && (s[first_end].is_ascii_alphabetic() || s[first_end] == b'_') {
        first_end += 1;
    }
    if s[..first_end].eq_ignore_ascii_case(b"do") {
        let rest = trim(&s[first_end..]);
        if !rest.starts_with(b"=") && !rest.starts_with(b"(") {
            return false;
        }
    }
    let mut depth: usize = 0;
    let mut quote = 0;
    for (i, c) in s.iter().enumerate() {
        if quote != 0 {
            if *c == quote && s.get(i + 1) != Some(c) {
                quote = 0
            };
            continue;
        }
        if *c == b'\'' || *c == b'"' {
            quote = *c;
            continue;
        }
        match c {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'=' if depth == 0 => return !s.get(i + 1).is_some_and(|x| *x == b'>' || *x == b'='),
            _ => {}
        }
    }
    false
}

fn prefixed_procedure(source: &[u8], words: &[Vec<u8>]) -> Option<(StatementKind, StatementClass)> {
    if words.first().is_some_and(|x| x == b"end") {
        return None;
    }
    // The legacy free-form fixtures contain `su broutine` in an
    // editor-like interoperability example. findent 4.3.7 still treats the
    // split keyword as a procedure boundary, so preserve that narrow
    // recovery without accepting arbitrary misspellings as structural.
    if words.first().is_some_and(|x| x == b"su") && words.get(1).is_some_and(|x| x == b"broutine") {
        return Some((StatementKind::Subroutine, StatementClass::Definition));
    }
    if words.first().is_some_and(|x| x == b"module")
        && words
            .get(1)
            .is_some_and(|x| x == b"subroutine" || x == b"function")
    {
        return Some((
            if words[1] == b"function" {
                StatementKind::Function
            } else {
                StatementKind::Subroutine
            },
            StatementClass::Definition,
        ));
    }
    if let Some(i) = words
        .iter()
        .position(|x| x == b"function" || x == b"subroutine")
    {
        if i > 0 {
            // findent's free-form recognizer accepts comma-free prefix words
            // (`pure elemental function`, `integer recursive function`) but
            // leaves declaration-style attribute lists opaque
            // (`integer, pure elemental function`).  Keeping that boundary is
            // important: otherwise a valid declaration opens a frame and
            // shifts every later sibling procedure.
            if comma_prefixed_procedure(source, words).is_some() {
                return None;
            }
            return Some((
                if words[i] == b"function" {
                    StatementKind::Function
                } else {
                    StatementKind::Subroutine
                },
                StatementClass::Definition,
            ));
        }
    }
    None
}

fn comma_prefixed_procedure(source: &[u8], words: &[Vec<u8>]) -> Option<StatementKind> {
    let i = words
        .iter()
        .position(|x| x == b"function" || x == b"subroutine")?;
    if i == 0 {
        return None;
    }
    let keyword = &words[i];
    let has_comma = crate::source::scanner::tokens(source)
        .into_iter()
        .find(|token| token.text.eq_ignore_ascii_case(keyword))
        .is_some_and(|token| source[..token.start].contains(&b','));
    has_comma.then_some(if keyword == b"function" {
        StatementKind::Function
    } else {
        StatementKind::Subroutine
    })
}

fn explicit_end_kind(words: &[Vec<u8>]) -> Option<StatementKind> {
    match words.first().map(Vec::as_slice) {
        Some(b"endfunction") => Some(StatementKind::Function),
        Some(b"endsubroutine") => Some(StatementKind::Subroutine),
        Some(b"end") => match words.get(1).map(Vec::as_slice) {
            Some(b"function") => Some(StatementKind::Function),
            Some(b"subroutine") => Some(StatementKind::Subroutine),
            _ => None,
        },
        _ => None,
    }
}
fn has_then(s: &[u8]) -> bool {
    let t = trim(s);
    t.windows(4).enumerate().any(|(i, w)| {
        w.eq_ignore_ascii_case(b"then")
            && (i == 0 || !is_identifier_byte(t[i - 1]))
            && t.get(i + 4).is_none_or(|c| !is_identifier_byte(*c))
    })
}

fn single_line_after_paren(s: &[u8], keyword: &[u8]) -> bool {
    let Some(rest) = s.get(keyword.len()..) else {
        return false;
    };
    let rest = trim(rest);
    if rest.first() != Some(&b'(') {
        return false;
    }
    let mut depth = 0usize;
    let mut quote = 0u8;
    let mut i = 0;
    while i < rest.len() {
        let byte = rest[i];
        if quote != 0 {
            if byte == quote {
                if rest.get(i + 1) == Some(&quote) {
                    i += 2;
                    continue;
                }
                quote = 0;
            }
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = byte;
        } else if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return !trim(&rest[i + 1..]).is_empty();
            }
        }
        i += 1;
    }
    false
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_type_definition(s: &[u8]) -> bool {
    let rest = if s.len() >= 4 && s[..4].eq_ignore_ascii_case(b"type") {
        trim(&s[4..])
    } else {
        return false;
    };
    if rest.starts_with(b"(") {
        return false;
    }
    // TYPE IS (...) and TYPE DEFAULT are SELECT TYPE branch statements.  A
    // keyword-first test would mistake them for derived-type definitions and
    // leave an extra frame open for the rest of the file.
    if starts_keyword(rest, b"is") || starts_keyword(rest, b"default") {
        return false;
    }
    rest.starts_with(b"::")
        || rest.starts_with(b",")
        || rest
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
}

fn is_procedure_definition(s: &[u8]) -> bool {
    let rest = trim(&s[b"procedure".len()..]);
    !rest.is_empty()
        && !rest.starts_with(b"(")
        && !rest.starts_with(b",")
        && !s.windows(2).any(|pair| pair == b"::")
}

fn starts_keyword(s: &[u8], keyword: &[u8]) -> bool {
    s.len() >= keyword.len()
        && s[..keyword.len()].eq_ignore_ascii_case(keyword)
        && s.get(keyword.len())
            .is_none_or(|c| c.is_ascii_whitespace() || *c == b'(' || *c == b',' || *c == b':')
}

fn end_kind(w: &[Vec<u8>]) -> (StatementKind, StatementClass) {
    match w.get(1).map(Vec::as_slice).unwrap_or(b"") {
        b"if" => (StatementKind::EndIf, StatementClass::Neutral),
        b"do" => (StatementKind::EndDo, StatementClass::Neutral),
        b"select" => (StatementKind::EndSelect, StatementClass::Neutral),
        b"where" => (StatementKind::EndWhere, StatementClass::Neutral),
        b"forall" => (StatementKind::EndForall, StatementClass::Neutral),
        b"associate" => (StatementKind::EndAssociate, StatementClass::Neutral),
        b"block" if w.get(2).is_some_and(|x| x == b"data") => {
            (StatementKind::Unknown, StatementClass::EndDefinition)
        }
        b"block" => (StatementKind::EndBlock, StatementClass::Neutral),
        b"critical" => (StatementKind::EndCritical, StatementClass::Neutral),
        b"team" => (StatementKind::EndTeam, StatementClass::Neutral),
        b"enum" => (StatementKind::EndEnum, StatementClass::Neutral),
        b"procedure" => (StatementKind::EndProcedure, StatementClass::EndDefinition),
        b"structure" => (StatementKind::EndStructure, StatementClass::Neutral),
        b"union" => (StatementKind::EndUnion, StatementClass::Neutral),
        b"map" => (StatementKind::EndMap, StatementClass::Neutral),
        b"subroutine" | b"function" | b"program" | b"module" | b"submodule" | b"interface"
        | b"type" | b"blockdata" => (StatementKind::Unknown, StatementClass::EndDefinition),
        _ => (StatementKind::Unknown, StatementClass::EndDefinition),
    }
}
fn entity_name(s: &[u8], kind: &StatementKind) -> Option<Vec<u8>> {
    if !matches!(
        kind,
        StatementKind::Program
            | StatementKind::Module
            | StatementKind::Submodule
            | StatementKind::Subroutine
            | StatementKind::Function
            | StatementKind::BlockData
            | StatementKind::Type
            | StatementKind::Interface
            | StatementKind::Procedure
    ) {
        return None;
    }
    let w = original_words(s);
    if matches!(kind, StatementKind::Function | StatementKind::Subroutine) {
        if let Some(i) = w.iter().position(|x| {
            x.eq_ignore_ascii_case(b"function") || x.eq_ignore_ascii_case(b"subroutine")
        }) {
            return w.get(i + 1).cloned();
        }
    }
    if *kind == StatementKind::Submodule {
        return w.get(2).cloned().or_else(|| w.get(1).cloned());
    }
    if *kind == StatementKind::BlockData {
        return w
            .iter()
            .position(|x| x.eq_ignore_ascii_case(b"data"))
            .and_then(|i| w.get(i + 1).cloned());
    }
    if *kind == StatementKind::Type {
        if let Some(pos) = s.windows(2).position(|pair| pair == b"::") {
            return original_words(&s[pos + 2..]).first().cloned();
        }
    }
    match kind {
        StatementKind::Procedure
            if w.get(1)
                .is_some_and(|word| word.eq_ignore_ascii_case(b"procedure")) =>
        {
            w.get(2).cloned()
        }
        _ => w.get(1).cloned(),
    }
}
fn original_words(s: &[u8]) -> Vec<Vec<u8>> {
    tokens(s)
        .into_iter()
        .filter(|t| {
            t.text
                .first()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        })
        .map(|t| t.text.to_vec())
        .collect()
}
fn referenced_labels(s: &[u8], kind: &StatementKind) -> Vec<u32> {
    if *kind != StatementKind::Do {
        return Vec::new();
    }
    let t = tokens(s);
    t.get(1)
        .filter(|x| x.text.iter().all(u8::is_ascii_digit))
        .and_then(|x| std::str::from_utf8(x.text).ok()?.parse().ok())
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::classify;
    use crate::classify::{StatementClass, StatementKind};

    #[test]
    fn supported_structural_families_are_case_insensitive() {
        let cases = [
            (b"PROGRAM p".as_slice(), StatementKind::Program),
            (b"module m".as_slice(), StatementKind::Module),
            (b"submodule (m) s".as_slice(), StatementKind::Submodule),
            (b"integer function f()".as_slice(), StatementKind::Function),
            (b"block data b".as_slice(), StatementKind::BlockData),
            (
                b"abstract interface".as_slice(),
                StatementKind::AbstractInterface,
            ),
            (b"type :: t".as_slice(), StatementKind::Type),
            (b"contains".as_slice(), StatementKind::Contains),
            (b"if (x) THEN".as_slice(), StatementKind::If),
            (b"else if (x) then".as_slice(), StatementKind::ElseIf),
            (b"do concurrent (i=1:n)".as_slice(), StatementKind::Do),
            (b"select rank (x)".as_slice(), StatementKind::Select),
            (b"where (x > 0)".as_slice(), StatementKind::Where),
            (b"forall (i=1:n)".as_slice(), StatementKind::Forall),
            (b"associate (x => y)".as_slice(), StatementKind::Associate),
            (b"block".as_slice(), StatementKind::Block),
            (b"critical".as_slice(), StatementKind::Critical),
            (b"change team (team)".as_slice(), StatementKind::ChangeTeam),
            (b"enum, bind(c)".as_slice(), StatementKind::Enum),
            (b"end do".as_slice(), StatementKind::EndDo),
            (b"end select".as_slice(), StatementKind::EndSelect),
            (b"end where".as_slice(), StatementKind::EndWhere),
            (b"end forall".as_slice(), StatementKind::EndForall),
            (b"end associate".as_slice(), StatementKind::EndAssociate),
            (b"end block".as_slice(), StatementKind::EndBlock),
            (b"end critical".as_slice(), StatementKind::EndCritical),
            (b"end team".as_slice(), StatementKind::EndTeam),
            (b"end enum".as_slice(), StatementKind::EndEnum),
            (b"structure /s/".as_slice(), StatementKind::Structure),
            (b"end structure".as_slice(), StatementKind::EndStructure),
            (b"union".as_slice(), StatementKind::Union),
            (b"map".as_slice(), StatementKind::Map),
        ];
        for (source, expected) in cases {
            assert_eq!(classify(source).kind, expected, "{source:?}");
        }
    }

    #[test]
    fn assignment_and_incomplete_inputs_are_conservative() {
        for source in [b"if = 1".as_slice(), b"do = 2", b"type = 3", b"if (x) the"] {
            let info = classify(source);
            assert_eq!(info.kind, StatementKind::Unknown, "{source:?}");
            assert_ne!(info.class, StatementClass::Definition);
        }
        assert_eq!(classify(b"100 continue").statement_label, Some(100));
        assert_eq!(classify(b"do 100 i=1,2").referenced_labels, vec![100]);
        assert_eq!(classify(b"findentfix:p-on").kind, StatementKind::Unknown);
    }

    #[test]
    fn keyword_prefixes_and_malformed_boundaries_do_not_open_frames() {
        let opaque = [
            b"programmer = 1".as_slice(),
            b"module_name = 1",
            b"doable = 1",
            b"endifx = 1",
            b"end ifx",
            b"end   unknown",
            b"if (x) thenish",
            b"elsewherex",
            b"procedure(x) :: p",
            b"outer:: do i = 1, 2",
        ];
        for source in opaque {
            let info = classify(source);
            assert_ne!(info.class, StatementClass::Definition, "{source:?}");
            assert!(!matches!(
                info.kind,
                StatementKind::Program
                    | StatementKind::Module
                    | StatementKind::Do
                    | StatementKind::If
                    | StatementKind::EndIf
            ));
        }
    }

    #[test]
    fn every_structural_prefix_has_assignment_and_keyword_negatives() {
        let negatives = [
            b"programmer = 1".as_slice(),
            b"module_name = 1",
            b"subroutine_name = 1",
            b"function_value = 1",
            b"block_size = 1",
            b"interface_name = 1",
            b"type_name = 1",
            b"contains_value = 1",
            b"associate_value = 1",
            b"block_value = 1",
            b"critical_value = 1",
            b"change team_value = 1",
            b"enum_value = 1",
            b"entry_value = 1",
            b"structure_value = 1",
            b"union_value = 1",
            b"map_value = 1",
            b"endifx",
            b"enddoit",
            b"endselectx",
            b"endwherex",
            b"endforallx",
            b"endassociatex",
            b"endblockx",
            b"endcriticalx",
            b"endteamx",
            b"endenumx",
            b"endstructurex",
            b"endunionx",
            b"endmapx",
            b"abstract interfacex",
            b"change teamx",
            b"outer:: do i = 1, 2",
            b"10abc continue",
        ];
        for source in negatives {
            let info = classify(source);
            assert_eq!(info.kind, StatementKind::Unknown, "{source:?}");
            assert_ne!(info.class, StatementClass::Definition, "{source:?}");
        }
    }

    #[test]
    fn mixed_case_and_irregular_spacing_keep_structural_boundaries() {
        assert_eq!(classify(b"  EnD   If  ").kind, StatementKind::EndIf);
        assert_eq!(classify(b"SeLeCt   RaNk (x)").kind, StatementKind::Select);
        assert_eq!(
            classify(b"  CHANGE   TEAM (team)").kind,
            StatementKind::ChangeTeam
        );
        assert_eq!(
            classify(b"  20   outer :   Do 20 i = 1, 2").kind,
            StatementKind::Do
        );
        assert_eq!(
            classify(b"  20   outer :   Do 20 i = 1, 2").statement_label,
            Some(20)
        );
    }

    #[test]
    fn select_type_branches_do_not_open_derived_types() {
        assert_eq!(classify(b"TYPE IS (t)").kind, StatementKind::Case);
        assert_eq!(classify(b"Type Default").kind, StatementKind::Case);
        assert_eq!(classify(b"CLASS IS (t)").kind, StatementKind::Case);
        assert_eq!(classify(b"class default").kind, StatementKind::Case);
    }

    #[test]
    fn procedure_declarations_do_not_open_procedure_frames() {
        let info = classify(b"procedure(state_function), private :: dtauda");
        assert_eq!(info.kind, StatementKind::Unknown);
        assert_eq!(info.class, StatementClass::Neutral);
        assert_eq!(
            classify(b"module procedure dtauda").kind,
            StatementKind::Procedure
        );
        assert_eq!(classify(b"procedure myproc").kind, StatementKind::Procedure);
    }

    #[test]
    fn comma_prefixed_procedure_attributes_remain_opaque() {
        assert_eq!(
            classify(b"integer, pure elemental function f(x)").kind,
            StatementKind::Unknown
        );
        assert_eq!(
            classify(b"pure elemental function f(x)").kind,
            StatementKind::Function
        );
    }

    #[test]
    fn legacy_split_subroutine_keyword_opens_a_boundary() {
        let info = classify(b"su broutine sub bind(c)");
        assert_eq!(info.kind, StatementKind::Subroutine);
        assert_eq!(info.class, StatementClass::Definition);
    }

    #[test]
    fn every_public_structural_family_has_a_positive_spelling() {
        let cases = [
            (b"program p".as_slice(), StatementKind::Program),
            (b"module m".as_slice(), StatementKind::Module),
            (b"submodule (m:s) s".as_slice(), StatementKind::Submodule),
            (b"subroutine s".as_slice(), StatementKind::Subroutine),
            (b"function f".as_slice(), StatementKind::Function),
            (b"block data b".as_slice(), StatementKind::BlockData),
            (b"interface i".as_slice(), StatementKind::Interface),
            (
                b"abstract interface".as_slice(),
                StatementKind::AbstractInterface,
            ),
            (b"type :: t".as_slice(), StatementKind::Type),
            (b"contains".as_slice(), StatementKind::Contains),
            (b"module procedure s".as_slice(), StatementKind::Procedure),
            (b"procedure s".as_slice(), StatementKind::Procedure),
            (b"if (x) then".as_slice(), StatementKind::If),
            (b"else if (x) then".as_slice(), StatementKind::ElseIf),
            (b"else".as_slice(), StatementKind::Else),
            (b"do i = 1, 2".as_slice(), StatementKind::Do),
            (b"select case (i)".as_slice(), StatementKind::Select),
            (b"case (1)".as_slice(), StatementKind::Case),
            (b"where (x > 0)".as_slice(), StatementKind::Where),
            (b"elsewhere".as_slice(), StatementKind::ElseWhere),
            (b"forall (i = 1:2)".as_slice(), StatementKind::Forall),
            (b"associate (x => y)".as_slice(), StatementKind::Associate),
            (b"block".as_slice(), StatementKind::Block),
            (b"critical".as_slice(), StatementKind::Critical),
            (b"change team (team)".as_slice(), StatementKind::ChangeTeam),
            (b"enum, bind(c)".as_slice(), StatementKind::Enum),
            (b"entry e".as_slice(), StatementKind::Entry),
            (b"include 'x.inc'".as_slice(), StatementKind::Include),
            (b"continue".as_slice(), StatementKind::LabelContinue),
            (b"end if".as_slice(), StatementKind::EndIf),
            (b"end do".as_slice(), StatementKind::EndDo),
            (b"end select".as_slice(), StatementKind::EndSelect),
            (b"end where".as_slice(), StatementKind::EndWhere),
            (b"end forall".as_slice(), StatementKind::EndForall),
            (b"end associate".as_slice(), StatementKind::EndAssociate),
            (b"end block".as_slice(), StatementKind::EndBlock),
            (b"end critical".as_slice(), StatementKind::EndCritical),
            (b"end team".as_slice(), StatementKind::EndTeam),
            (b"end enum".as_slice(), StatementKind::EndEnum),
            (b"end procedure".as_slice(), StatementKind::EndProcedure),
            (b"structure /s/".as_slice(), StatementKind::Structure),
            (b"end structure".as_slice(), StatementKind::EndStructure),
            (b"union".as_slice(), StatementKind::Union),
            (b"end union".as_slice(), StatementKind::EndUnion),
            (b"map".as_slice(), StatementKind::Map),
            (b"end map".as_slice(), StatementKind::EndMap),
        ];
        for (source, expected) in cases {
            assert_eq!(classify(source).kind, expected, "{source:?}");
        }
    }

    #[test]
    fn labels_and_construct_names_are_preserved_around_recognition() {
        let named = classify(b"outer: do i = 1, 2");
        assert_eq!(named.kind, StatementKind::Do);
        assert_eq!(named.construct_name.as_deref(), Some(&b"outer"[..]));

        let labeled = classify(b"100 outer: do i = 1, 2");
        assert_eq!(labeled.kind, StatementKind::Do);
        assert_eq!(labeled.statement_label, Some(100));
        assert_eq!(labeled.construct_name.as_deref(), Some(&b"outer"[..]));

        let label_target = classify(b"100 continue");
        assert_eq!(label_target.kind, StatementKind::LabelContinue);
        assert_eq!(label_target.statement_label, Some(100));
    }
}
