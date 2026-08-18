mod recognizers;
pub mod statement;

pub use statement::{StatementClass, StatementInfo, StatementKind};

/// Classify a statement, including Fortran 2023 structural spellings that the
/// legacy recognizer predates.
pub fn classify(input: &[u8]) -> StatementInfo {
    let mut info = recognizers::classify(input);
    if !needs_supplemental_classification(&info) {
        return info;
    }

    let mut words = crate::source::scanner::iter_tokens(input)
        .filter(|token| is_word(token.text))
        .map(|token| token.text);
    let first = words.next();
    let second = words.next();
    let third = words.next();

    if info.kind == StatementKind::Unknown
        && info.class == StatementClass::Neutral
        && word_is(first, b"enumeration")
        && word_is(second, b"type")
    {
        info.kind = StatementKind::Enum;
        info.class = StatementClass::Definition;
        info.end_kind = None;
    } else if info.class == StatementClass::EndDefinition
        && word_is(first, b"end")
        && word_is(second, b"enumeration")
        && word_is(third, b"type")
    {
        info.kind = StatementKind::EndEnum;
        info.class = StatementClass::Neutral;
        info.end_kind = None;
    } else if info.kind == StatementKind::Else
        && word_is(first, b"else")
        && word_is(second, b"where")
    {
        // ELSE WHERE is one of the standard optional-blank adjacent keyword forms.
        info.kind = StatementKind::ElseWhere;
        info.class = StatementClass::Executable;
        info.end_kind = None;
    } else if info.class == StatementClass::EndDefinition
        && word_is(first, b"end")
        && word_is(second, b"file")
    {
        // END FILE is the separated spelling of ENDFILE, not a definition end.
        info.kind = StatementKind::Unknown;
        info.class = StatementClass::Neutral;
        info.end_kind = None;
    }
    info
}

fn needs_supplemental_classification(info: &StatementInfo) -> bool {
    (info.kind == StatementKind::Unknown && info.class == StatementClass::Neutral)
        || info.class == StatementClass::EndDefinition
        || info.kind == StatementKind::Else
}

fn is_word(text: &[u8]) -> bool {
    text.first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
}

fn word_is(word: Option<&[u8]>, expected: &[u8]) -> bool {
    word.is_some_and(|word| word.eq_ignore_ascii_case(expected))
}
