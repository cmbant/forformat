use crate::source::{
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
    if first.kind != TokenKind::Name || first.is(b"use") {
        return Vec::new();
    }
    let separator = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::");
    let Some(separator) = separator else {
        return old_style_variable_names(&tokens, first_index);
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
    declaration_entity_names(&tokens, separator + 1)
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
    let Some(separator) = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")
    else {
        return Vec::new();
    };
    declaration_entity_names(&tokens, separator + 1)
}

fn old_style_variable_names(tokens: &[Token<'_>], first_index: usize) -> Vec<Vec<u8>> {
    let first = &tokens[first_index];
    let declaration = matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer"
            | b"real"
            | b"complex"
            | b"logical"
            | b"character"
            | b"type"
            | b"class"
            | b"typeof"
            | b"classof"
    ) || first.is(b"double")
        && tokens
            .get(first_index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    if !declaration {
        return Vec::new();
    }
    let start = first_index
        + 1
        + usize::from(
            first.is(b"double")
                && tokens
                    .get(first_index + 1)
                    .is_some_and(|token| token.is_name(b"precision")),
        );
    if tokens.iter().skip(start).any(|token| {
        token.kind == TokenKind::Name && token.depth == 0 && token.is_name(b"function")
    }) {
        return Vec::new();
    }
    declaration_entity_names(tokens, start)
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

pub(super) fn select_type_alias(text: &[u8]) -> Option<Vec<u8>> {
    let tokens = tokenize(text, &mut LexState::default());
    let select = tokens.iter().position(|token| token.is_name(b"select"))?;
    if !tokens
        .get(select + 1)
        .is_some_and(|token| token.is_name(b"type"))
        || !tokens
            .get(select + 2)
            .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        return None;
    }
    let alias = tokens.get(select + 3)?;
    let arrow = tokens.get(select + 4)?;
    (alias.kind == TokenKind::Name && arrow.text == b"=>").then(|| alias.text.to_vec())
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

pub(super) fn is_old_style_type_context(tokens: &[Token<'_>], first_index: usize) -> bool {
    let first = &tokens[first_index];
    if !(first.is(b"type") || first.is(b"class")) {
        return false;
    }
    tokens
        .get(first_index + 1)
        .is_none_or(|token| token.is_name(b"is") || token.is_name(b"default"))
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
