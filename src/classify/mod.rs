mod recognizers;
pub mod statement;

pub use statement::{StatementClass, StatementInfo, StatementKind};

/// Classify a statement, including Fortran 2023 structural spellings that the
/// legacy recognizer predates.
pub fn classify(input: &[u8]) -> StatementInfo {
    let mut info = recognizers::classify(input);
    let words: Vec<Vec<u8>> = crate::source::scanner::tokens(input)
        .into_iter()
        .filter(|token| {
            token
                .text
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        })
        .map(|token| token.text.to_ascii_lowercase())
        .collect();

    match words.as_slice() {
        [first, second, ..]
            if info.kind == StatementKind::Unknown
                && info.class == StatementClass::Neutral
                && first == b"enumeration"
                && second == b"type" =>
        {
            info.kind = StatementKind::Enum;
            info.class = StatementClass::Definition;
            info.end_kind = None;
        }
        [first, second, third, ..]
            if info.class == StatementClass::EndDefinition
                && first == b"end"
                && second == b"enumeration"
                && third == b"type" =>
        {
            info.kind = StatementKind::EndEnum;
            info.class = StatementClass::Neutral;
            info.end_kind = None;
        }
        // ELSE WHERE is one of the standard optional-blank adjacent keyword forms.
        [first, second, ..]
            if info.kind == StatementKind::Else && first == b"else" && second == b"where" =>
        {
            info.kind = StatementKind::ElseWhere;
            info.class = StatementClass::Executable;
            info.end_kind = None;
        }
        // END FILE is the separated spelling of ENDFILE, not a definition end.
        [first, second, ..]
            if info.class == StatementClass::EndDefinition
                && first == b"end"
                && second == b"file" =>
        {
            info.kind = StatementKind::Unknown;
            info.class = StatementClass::Neutral;
            info.end_kind = None;
        }
        _ => {}
    }
    info
}
