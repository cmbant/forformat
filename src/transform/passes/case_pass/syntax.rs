//! Token-shape predicates for declared-name classification.

use super::associations::select_association_spec;
use crate::{
    analysis::{names::NameSpace, ScopeTree},
    source::tokens::{Token, TokenKind},
    transform::vocab,
};

pub(super) fn is_use_statement(tokens: &[Token<'_>]) -> bool {
    tokens
        .iter()
        .find(|token| token.kind != TokenKind::Number)
        .is_some_and(|token| token.is_name(b"use"))
}

pub(super) fn implicit_guard_applies(tokens: &[Token<'_>], index: usize) -> bool {
    if tokens
        .get(index + 1)
        .is_some_and(|token| token.text == b"%")
    {
        return false;
    }
    !index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| token.is_name(b"call"))
}

pub(super) fn preceded_by_percent(tokens: &[Token<'_>], index: usize) -> bool {
    index > 0 && tokens[index - 1].text == b"%"
}

pub(super) fn active_procedure(scopes: &ScopeTree, line: usize) -> Option<&[u8]> {
    scopes
        .ancestors(scopes.index_of_line(line))
        .into_iter()
        .find(|scope| {
            matches!(
                scopes.scopes[*scope].kind,
                crate::analysis::scope::ScopeKind::Program
                    | crate::analysis::scope::ScopeKind::Procedure
            )
        })
        .and_then(|scope| scopes.scopes[scope].name.as_deref())
}

pub(super) fn use_module_index(tokens: &[Token<'_>]) -> Option<usize> {
    let use_index = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens[use_index].is_name(b"use") {
        return None;
    }
    let mut cursor = use_index + 1;
    if tokens
        .get(cursor)
        .is_some_and(|token| token.kind == TokenKind::Comma)
    {
        while cursor < tokens.len() && tokens[cursor].text != b"::" {
            cursor += 1;
        }
        if cursor == tokens.len() {
            return None;
        }
        cursor += 1;
    } else if tokens.get(cursor).is_some_and(|token| token.text == b"::") {
        cursor += 1;
    }
    tokens
        .get(cursor)
        .filter(|token| token.kind == TokenKind::Name)
        .map(|_| cursor)
}

pub(super) fn is_use_intrinsic(tokens: &[Token<'_>]) -> bool {
    let Some(use_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    let Some(separator) = tokens
        .iter()
        .position(|token| token.depth == 0 && token.text == b"::")
    else {
        return false;
    };
    tokens[use_index + 1..separator]
        .iter()
        .any(|token| token.depth == 0 && token.is_name(b"intrinsic"))
}

pub(super) fn is_use_module(tokens: &[Token<'_>], index: usize) -> bool {
    use_module_index(tokens) == Some(index)
}

pub(super) fn is_use_only_keyword(tokens: &[Token<'_>], index: usize) -> bool {
    tokens[index].is_name(b"only")
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.text == b":" && token.depth == tokens[index].depth)
}

pub(super) fn is_use_rename_local(tokens: &[Token<'_>], index: usize) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.text == b"=>" && token.depth == tokens[index].depth)
}

pub(super) fn is_external_reference(tokens: &[Token<'_>], index: usize) -> bool {
    index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| token.is_name(b"call"))
        || tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen)
}

pub(super) fn is_type_spec_name(tokens: &[Token<'_>], index: usize) -> bool {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number);
    if select_association_spec(tokens, first).is_some_and(|spec| spec.alias_index == index) {
        return false;
    }
    if index < 2 || tokens[index - 1].kind != TokenKind::LParen {
        return false;
    }
    if tokens[index - 2].kind == TokenKind::Name
        && (tokens[index - 2].is_name(b"type") || tokens[index - 2].is_name(b"class"))
    {
        return true;
    }
    index >= 3
        && tokens[index - 2].is_name(b"is")
        && (tokens[index - 3].is_name(b"type") || tokens[index - 3].is_name(b"class"))
}

pub(super) fn is_intrinsic_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
    if index < 2 || tokens[index - 1].kind != TokenKind::LParen {
        return false;
    }
    let Some(type_name) = tokens.get(index - 2) else {
        return false;
    };
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    type_name.kind == TokenKind::Name
        && index - 2 == first
        && [b"integer".as_slice(), b"real", b"complex", b"logical"]
            .iter()
            .any(|candidate| type_name.is_name(candidate))
}

/// The namespace a named `END` statement closes, when `index` is that name.
///
/// `end type t` and `endtype t` are one statement written two ways, and which
/// one is in the buffer depends on whether the compound-keyword split has run
/// over this line yet. That split runs in the same pass as this one, so reading
/// only the split spelling meant `endtype t_NAME` was a statement with no name
/// in it on the run that split the keyword, and carried a name only on the run
/// after -- which is why it took two to settle. `endif` and `endmodule` hid the
/// same gap: neither closes a construct whose name this pass would recase.
pub(super) fn named_end_space(tokens: &[Token<'_>], index: usize) -> Option<NameSpace> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    let head = tokens.get(first)?;
    let (kind, name) = if head.is_name(b"end") {
        (tokens.get(first + 1)?.text.to_ascii_lowercase(), first + 2)
    } else {
        (joined_end_construct(head)?, first + 1)
    };
    if index != name {
        return None;
    }
    match kind.as_slice() {
        b"module" | b"submodule" => Some(NameSpace::Module),
        b"function" | b"subroutine" | b"program" | b"procedure" | b"blockdata" => {
            Some(NameSpace::Symbol)
        }
        b"type" => Some(NameSpace::Type),
        _ => None,
    }
}

/// The construct keyword inside a joined `END` keyword, for a head token.
///
/// A thin wrapper over [`vocab::joined_end_construct`], which is shared with
/// [`super::super::named_end`]: the token kind is the only thing this reader
/// adds, and the table lookup is the part that must not be written twice.
fn joined_end_construct(head: &Token<'_>) -> Option<Vec<u8>> {
    (head.kind == TokenKind::Name)
        .then(|| vocab::joined_end_construct(head.text))
        .flatten()
}

pub(super) fn scope_header_space(tokens: &[Token<'_>], index: usize) -> Option<NameSpace> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if first + 1 != index {
        return None;
    }
    match tokens[first].text.to_ascii_lowercase().as_slice() {
        b"module" | b"submodule" => Some(NameSpace::Module),
        b"program" | b"function" | b"subroutine" | b"procedure" | b"blockdata" => {
            Some(NameSpace::Symbol)
        }
        _ => None,
    }
}

pub(super) fn is_declaration_entity(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(separator) = tokens[..index]
        .iter()
        .enumerate()
        .filter(|(_, token)| token.depth == 0 && token.text == b"::")
        .map(|(position, _)| position)
        .next_back()
    else {
        return old_style_declaration_entity(tokens, index);
    };
    let mut initializer = false;
    let mut array_depth = 0usize;
    for token in &tokens[separator + 1..index] {
        if token.kind == TokenKind::LBracket {
            array_depth += 1;
            continue;
        }
        if token.kind == TokenKind::RBracket {
            array_depth = array_depth.saturating_sub(1);
            continue;
        }
        if token.depth != 0 && array_depth == 0 {
            continue;
        }
        if token.text == b"=" || token.text == b"=>" {
            initializer = true;
        } else if token.kind == TokenKind::Comma {
            initializer = false;
        }
    }
    !initializer && array_depth == 0 && tokens[index].depth == 0
}

fn old_style_declaration_entity(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(first_index) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    let first = &tokens[first_index];
    let is_type = matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer" | b"real" | b"complex" | b"logical" | b"character" | b"type" | b"class"
    ) || first.is_name(b"double")
        && tokens
            .get(first_index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    if !is_type || index <= first_index {
        return false;
    }
    if tokens
        .iter()
        .skip(first_index + 1)
        .take(index.saturating_sub(first_index + 1))
        .any(|token| token.kind == TokenKind::Name && token.is_name(b"function"))
    {
        return false;
    }
    if tokens[..index]
        .iter()
        .any(|token| token.depth == 0 && (token.text == b"=" || token.text == b"=>"))
    {
        return false;
    }
    let mut start = first_index + 1;
    if first.is_name(b"double") {
        start += 1;
    }
    if tokens
        .get(start)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        let depth = tokens[start].depth;
        let Some(close) = tokens
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == depth)
            .map(|(position, _)| position)
        else {
            return false;
        };
        start = close + 1;
    }
    if index < start || tokens[index].kind != TokenKind::Name || tokens[index].depth != 0 {
        return false;
    }

    let entity_start = tokens[start..index]
        .iter()
        .enumerate()
        .filter(|(_, token)| token.depth == 0 && token.kind == TokenKind::Comma)
        .map(|(offset, _)| start + offset + 1)
        .next_back()
        .unwrap_or(start);
    let before = &tokens[entity_start..index];
    !before
        .iter()
        .any(|token| token.depth == 0 && (token.text == b"=" || token.text == b"=>"))
        && !before.iter().any(|token| {
            token.kind == TokenKind::Name && token.depth == 0 && token.text != b"intent"
        })
}

pub(super) fn is_numeric_literal_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2 && tokens[index - 1].text == b"_" && tokens[index - 2].kind == TokenKind::Number
}
