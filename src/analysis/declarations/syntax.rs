use crate::source::{
    declaration_separator::{declaration_separator, DeclarationSeparator},
    tokens::{tokenize, Token, TokenKind},
    LexState,
};

pub(super) fn declared_variable_names(text: &[u8]) -> Vec<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return Vec::new();
    };
    let first = &tokens[first_index];
    if first.kind != TokenKind::Name
        || matches!(
            first.text.to_ascii_lowercase().as_slice(),
            b"use" | b"import" | b"public" | b"private"
        )
    {
        return Vec::new();
    }
    let start = match declaration_separator(&tokens) {
        DeclarationSeparator::Present => tokens
            .iter()
            .position(|token| token.depth == 0 && token.text == b"::")
            .map(|separator| separator + 1),
        DeclarationSeparator::Missing { insert_before } => Some(insert_before),
        DeclarationSeparator::NotApplicable => None,
    };
    let Some(start) = start else {
        return Vec::new();
    };
    if matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"type" | b"class" | b"typeof" | b"classof"
    ) && tokens
        .get(first_index + 1)
        .is_none_or(|token| token.kind != TokenKind::LParen)
    {
        return Vec::new();
    }
    if matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"generic" | b"final"
    ) {
        return Vec::new();
    }
    declaration_entity_names(&tokens, start)
}

pub(super) fn declared_binding_names(text: &[u8]) -> Vec<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return Vec::new();
    };
    if !matches!(
        tokens[first].text.to_ascii_lowercase().as_slice(),
        b"procedure" | b"generic" | b"final"
    ) {
        return Vec::new();
    }
    let start = if tokens[first].is_name(b"generic") {
        tokens
            .iter()
            .position(|token| token.depth == 0 && token.text == b"::")
            .map(|separator| separator + 1)
    } else {
        match declaration_separator(&tokens) {
            DeclarationSeparator::Present => tokens
                .iter()
                .position(|token| token.depth == 0 && token.text == b"::")
                .map(|separator| separator + 1),
            DeclarationSeparator::Missing { insert_before } => Some(insert_before),
            DeclarationSeparator::NotApplicable => None,
        }
    };
    start.map_or_else(Vec::new, |start| declaration_entity_names(&tokens, start))
}

fn declaration_entity_names(tokens: &[Token<'_>], start: usize) -> Vec<Vec<u8>> {
    let mut names = Vec::new();
    let mut expect_name = true;
    let mut initializer = false;
    for token in tokens.iter().skip(start) {
        if token.depth > 0 {
            continue;
        }
        if token.text == b"=" || token.text == b"=>" {
            initializer = true;
            expect_name = false;
            continue;
        }
        if token.kind == TokenKind::Comma {
            initializer = false;
            expect_name = true;
            continue;
        }
        if !initializer && expect_name && token.kind == TokenKind::Name {
            names.push(token.text.to_vec());
            expect_name = false;
        }
    }
    names
}

pub(super) fn procedure_header_names(text: &[u8]) -> Vec<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let Some(kind) = tokens.iter().position(|token| {
        token.kind == TokenKind::Name && (token.is(b"function") || token.is(b"subroutine"))
    }) else {
        return Vec::new();
    };
    let Some(name_index) = tokens
        .iter()
        .enumerate()
        .skip(kind + 1)
        .find(|(_, token)| token.kind == TokenKind::Name)
        .map(|(index, _)| index)
    else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut index = name_index + 1;
    if tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        index += 1;
        let mut expect_name = true;
        while let Some(token) = tokens.get(index) {
            if token.kind == TokenKind::RParen && token.depth == 0 {
                index += 1;
                break;
            }
            if token.kind == TokenKind::Comma && token.depth == 1 {
                expect_name = true;
            } else if expect_name && token.kind == TokenKind::Name && token.depth == 1 {
                names.push(token.text.to_vec());
                expect_name = false;
            }
            index += 1;
        }
    }
    for window in tokens[index..].windows(2) {
        if window[0].is_name(b"result") && window[1].kind == TokenKind::LParen {
            if let Some(name) = tokens
                .iter()
                .skip(index)
                .skip_while(|token| !token.is_name(b"result"))
                .nth(2)
                .filter(|token| token.kind == TokenKind::Name)
            {
                names.push(name.text.to_vec());
            }
            break;
        }
    }
    names
}

pub(super) fn type_spec_name<'a>(
    tokens: &[Token<'a>],
    start: usize,
    limit: usize,
) -> Option<&'a [u8]> {
    let open = tokens.get(start + 1)?;
    if open.kind != TokenKind::LParen {
        return None;
    }
    tokens
        .get(start + 2..limit)?
        .iter()
        .take_while(|token| token.depth > open.depth)
        .find(|token| token.kind == TokenKind::Name)
        .map(|token| token.text)
}

pub(super) fn is_type_definition(tokens: &[Token<'_>], first_index: usize) -> bool {
    let first = &tokens[first_index];
    if !first.is(b"type") {
        return false;
    }
    !tokens
        .get(first_index + 1)
        .is_some_and(|token| token.kind == TokenKind::LParen)
        && tokens
            .get(first_index + 1)
            .is_none_or(|token| token.text == b"::" || token.kind == TokenKind::Comma)
}

/// Return `(child, direct_parent)` from `TYPE, EXTENDS(parent) :: child`.
pub(super) fn type_definition_parent(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let tokens = tokenize(text, &mut LexState::default());
    let first_index = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !is_type_definition(&tokens, first_index) {
        return None;
    }
    let separator = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")?;
    let extends = tokens
        .iter()
        .enumerate()
        .skip(first_index + 1)
        .take_while(|(index, _)| *index < separator)
        .find(|(_, token)| token.depth == 0 && token.is_name(b"extends"))
        .map(|(index, _)| index)?;
    let parent = tokens
        .get(extends + 1..separator)?
        .iter()
        .find(|token| token.kind == TokenKind::Name)?
        .text;
    let child = tokens
        .get(separator + 1..)?
        .iter()
        .find(|token| token.kind == TokenKind::Name)?
        .text;
    Some((child, parent))
}

pub(super) fn old_style_type_name<'a>(
    tokens: &'a [Token<'a>],
    first_index: usize,
) -> Option<&'a [u8]> {
    let open = tokens.get(first_index + 1)?;
    if open.kind != TokenKind::LParen {
        return None;
    }
    tokens
        .get(first_index + 2..)?
        .iter()
        .take_while(|token| token.depth > open.depth)
        .find(|token| token.kind == TokenKind::Name)
        .map(|token| token.text)
}

#[cfg(test)]
mod tests {
    use super::{declared_binding_names, declared_variable_names};

    #[test]
    fn old_style_declarations_use_shared_separator_sites() {
        for (source, expected) in [
            (b"DOUBLEPRECISION sum".as_slice(), b"sum".as_slice()),
            (b"TYPEOF(x) result", b"result"),
            (b"CLASSOF(x) kind", b"kind"),
            (b"DOUBLE COMPLEX product", b"product"),
            (b"INTEGER*1 count", b"count"),
            (b"DIMENSION RADSAV(2)", b"RADSAV"),
            (b"INTENT(IN) Arg", b"Arg"),
        ] {
            assert_eq!(declared_variable_names(source), vec![expected.to_vec()]);
        }
    }

    #[test]
    fn association_and_access_names_are_not_declarations() {
        for source in [
            b"import HostName".as_slice(),
            b"import :: HostName".as_slice(),
            b"public ExportedName".as_slice(),
            b"public :: ExportedName".as_slice(),
            b"private HiddenName".as_slice(),
            b"private :: HiddenName".as_slice(),
        ] {
            assert!(declared_variable_names(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn declaration_facts_do_not_depend_on_separator_modernization() {
        for (legacy, modern) in [
            (
                b"dimension RADSAV(2)".as_slice(),
                b"dimension :: RADSAV(2)".as_slice(),
            ),
            (b"intent(in) Arg", b"intent(in) :: Arg"),
            (b"procedure(cb) Handler", b"procedure(cb) :: Handler"),
        ] {
            assert_eq!(
                declared_variable_names(legacy),
                declared_variable_names(modern),
                "facts differ for {:?}",
                String::from_utf8_lossy(legacy)
            );
        }
    }

    #[test]
    fn binding_facts_do_not_depend_on_separator_modernization() {
        for (legacy, modern) in [
            (
                b"procedure(cb) Handler".as_slice(),
                b"procedure(cb) :: Handler".as_slice(),
            ),
            (b"final Finish".as_slice(), b"final :: Finish".as_slice()),
        ] {
            assert_eq!(
                declared_binding_names(legacy),
                declared_binding_names(modern),
                "binding facts differ for {:?}",
                String::from_utf8_lossy(legacy)
            );
        }
    }
}
