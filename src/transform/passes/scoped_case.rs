//! Scope-aware reconciliation for the declared-case pass.
//!
//! `case_pass::declared` remains the local spelling engine and compatibility
//! surface. This wrapper records the authored token spelling, runs that pass,
//! then replaces only cross-file evidence with the entity that is actually
//! visible through lexical/host/USE association at the occurrence. If a
//! project-wide compatibility table changed an unrelated token, its authored
//! spelling is restored before the later keyword/intrinsic rule sees it.

use crate::{
    analysis::{project::ResolvedType, scoped_declared_names, DeclaredNameIndex},
    error::FormatError,
    source::{
        syntax::is_end_construct_keyword,
        tokens::{tokenize, Token, TokenKind},
        LexState,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::{
            case_pass,
            provenance::{source_spans, spread_replacement},
        },
        pipeline::{Changed, PassContext},
    },
};
use std::{collections::HashMap, ops::Range};

pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    // Both passes classify against the same declared names, so the index is
    // built once and lent to the base pass rather than computed twice.
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let mut changed = case_pass::declared_with_names(document, cx, &declared_names)?;
    let mut line_edits: Vec<Vec<(Range<usize>, Vec<u8>)>> = vec![Vec::new(); document.lines.len()];
    let mut associate_stack: Vec<HashMap<Vec<u8>, Vec<u8>>> = Vec::new();

    for group in &cx.analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            let opening_aliases = associate_opening_aliases(&tokens);

            for (index, token) in tokens.iter().enumerate() {
                if token.kind != TokenKind::Name || cx.project.macros.contains(token.text) {
                    continue;
                }
                let spans = source_spans(group, statement, token);
                let Some(&(line, _)) = spans.first() else {
                    continue;
                };
                let decision = scoped_spelling(
                    &tokens,
                    index,
                    line,
                    &declared_names,
                    cx,
                    &associate_stack,
                );
                let replacement = match decision {
                    Decision::KeepBase => continue,
                    Decision::Replace(spelling) => spelling,
                    Decision::Restore => token.text.to_vec(),
                };
                // Reuse the base pass's own distribution rule rather than a
                // parallel one: writing back piece by piece is only sound
                // because a replacement always has the token's length, and
                // `spread_replacement` is where that is decided.
                let Some(pieces) = spread_replacement(&spans, token, &replacement) else {
                    continue;
                };
                for (source_line, span, piece) in pieces {
                    let line_start = cx.analysis.buffer.lines[source_line].span.start as usize;
                    line_edits[source_line].push((
                        span.start - line_start..span.end - line_start,
                        piece.to_vec(),
                    ));
                }
            }

            if let Some(aliases) = opening_aliases {
                associate_stack.push(aliases);
            }
            if is_end_associate(&tokens) {
                associate_stack.pop();
            }
        }
    }

    for (line, edits) in line_edits.into_iter().enumerate() {
        if edits.is_empty() {
            continue;
        }
        let source = &document.lines[line];
        let mut buffer = EditBuffer::new(source);
        for (span, replacement) in edits {
            buffer.replace(span, &replacement);
        }
        let updated = buffer.finish();
        if updated != *source {
            document.lines[line] = updated;
            changed = changed.or(Changed::Text);
        }
    }

    Ok(changed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    KeepBase,
    Replace(Vec<u8>),
    Restore,
}

fn scoped_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &DeclaredNameIndex,
    cx: &PassContext,
    enclosing_aliases: &[HashMap<Vec<u8>, Vec<u8>>],
) -> Decision {
    let token = &tokens[index];

    if is_end_construct_keyword(tokens, index)
        || (index > 0 && is_end_construct_keyword(tokens, index - 1))
        || named_end_space(tokens, index)
        || scope_header(tokens, index)
    {
        return Decision::KeepBase;
    }

    // A remote identifier in a USE statement belongs to that statement's
    // module, not to the union of every USE association in the active unit.
    // Handle USE before the generic `::` declaration heuristic as the optional
    // double colon is part of normal USE syntax too.
    if let Some(module_index) = use_module_index(tokens) {
        if index <= module_index || is_use_only_keyword(tokens, index) {
            return Decision::KeepBase;
        }
        // A rename's left-hand name is declared by this USE statement. Its own
        // authored spelling governs the local alias; only the right-hand remote
        // name is resolved through the named module.
        if is_use_rename_local(tokens, index) {
            return Decision::Restore;
        }
        return cx
            .project
            .visible_use_symbol_spelling(tokens[module_index].text, token.text)
            .map(Decision::Replace)
            .unwrap_or(Decision::Restore);
    }

    if is_declaration_entity(tokens, index) {
        return Decision::KeepBase;
    }

    if preceded_by_percent(tokens, index) {
        return scoped_member_spelling(tokens, index, line, cx, enclosing_aliases);
    }

    // The alias declaration itself belongs to the opening ASSOCIATE statement,
    // but its selector is evaluated in the surrounding scope. Restore the
    // declaration token exactly as authored; the new alias is pushed onto the
    // stack only after every selector token on this statement has been handled.
    if is_associate_alias_declaration(tokens, index) {
        return Decision::Restore;
    }

    // Only aliases from already-open constructs are visible here. The current
    // opening ASSOCIATE is deliberately absent: its associate names govern the
    // block, not the selectors that establish those associations.
    if let Some(spelling) = alias_spelling(enclosing_aliases, token.text) {
        return Decision::Replace(spelling.to_vec());
    }

    if is_type_spec_name(tokens, index) {
        return cx
            .project
            .visible_type_spelling(cx.local, line, token.text)
            .map(Decision::Replace)
            .unwrap_or(Decision::Restore);
    }

    // Only declarations owned by the active local/construct scope can
    // bypass project visibility. Host declarations must pass through the
    // semantic host graph so IMPORT restrictions and submodule ancestry apply.
    if declared_names.local_contains(line, token.text) {
        return Decision::KeepBase;
    }

    if let Some(spelling) = cx
        .project
        .visible_symbol_spelling(cx.local, line, token.text)
    {
        return Decision::Replace(spelling);
    }

    if is_external_reference(tokens, index) {
        if let Some(spelling) = cx.project.external_symbol_spelling(token.text) {
            return Decision::Replace(spelling);
        }
    }

    Decision::Restore
}

fn scoped_member_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    cx: &PassContext,
    enclosing_aliases: &[HashMap<Vec<u8>, Vec<u8>>],
) -> Decision {
    let Some(names) = component_owner_names(tokens, index, true) else {
        return Decision::KeepBase;
    };
    let Some(root) = names.first() else {
        return Decision::KeepBase;
    };
    // An alias has no entry in the type graph, so a member reached through one
    // cannot be resolved and must keep whatever the base pass decided. Only
    // enclosing constructs count: Fortran evaluates a selector in the scope
    // outside its own ASSOCIATE, where the alias is not yet visible.
    if alias_spelling(enclosing_aliases, root).is_some() {
        return Decision::KeepBase;
    }

    let Some(current) = cx.project.visible_variable_type(cx.local, line, root) else {
        return Decision::Restore;
    };
    let Some(owner) = resolve_component_owner(current, &names[1..], cx) else {
        return Decision::Restore;
    };
    cx.project
        .visible_member_spelling(cx.local, &owner, tokens[index].text)
        .map(Decision::Replace)
        .unwrap_or(Decision::Restore)
}

fn resolve_component_owner(
    mut current: ResolvedType,
    links: &[&[u8]],
    cx: &PassContext,
) -> Option<ResolvedType> {
    for link in links {
        current = cx
            .project
            .visible_component_type(cx.local, &current, link)?;
    }
    Some(current)
}

fn preceded_by_percent(tokens: &[Token<'_>], index: usize) -> bool {
    index > 0 && tokens[index - 1].text == b"%"
}

fn component_owner_names<'a>(
    tokens: &'a [Token<'a>],
    index: usize,
    indexed_chain: bool,
) -> Option<Vec<&'a [u8]>> {
    if index < 2 || !preceded_by_percent(tokens, index) {
        return None;
    }
    let mut names = Vec::new();
    let mut cursor = index - 2;
    loop {
        if indexed_chain && tokens.get(cursor)?.kind == TokenKind::RParen {
            let mut depth = 1;
            while cursor > 0 {
                cursor -= 1;
                match tokens[cursor].kind {
                    TokenKind::RParen => depth += 1,
                    TokenKind::LParen => {
                        depth -= 1;
                        if depth == 0 {
                            cursor = cursor.checked_sub(1)?;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        let token = tokens.get(cursor)?;
        if token.kind != TokenKind::Name {
            return None;
        }
        names.push(token.text);
        if cursor < 2 || tokens[cursor - 1].text != b"%" {
            break;
        }
        cursor -= 2;
    }
    names.reverse();
    Some(names)
}

fn is_type_spec_name(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].kind == TokenKind::LParen
        && tokens[index - 2].kind == TokenKind::Name
        && (tokens[index - 2].is_name(b"type") || tokens[index - 2].is_name(b"class"))
}

fn use_module_index(tokens: &[Token<'_>]) -> Option<usize> {
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

fn is_use_only_keyword(tokens: &[Token<'_>], index: usize) -> bool {
    tokens[index].is_name(b"only")
        && tokens.get(index + 1).is_some_and(|token| {
            token.text == b":" && token.depth == tokens[index].depth
        })
}

fn is_use_rename_local(tokens: &[Token<'_>], index: usize) -> bool {
    tokens.get(index + 1).is_some_and(|token| {
        token.text == b"=>" && token.depth == tokens[index].depth
    })
}

fn is_external_reference(tokens: &[Token<'_>], index: usize) -> bool {
    index
        .checked_sub(1)
        .and_then(|previous| tokens.get(previous))
        .is_some_and(|token| token.is_name(b"call"))
        || tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen)
}

fn scope_header(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    first + 1 == index
        && matches!(
            tokens[first].text.to_ascii_lowercase().as_slice(),
            b"module"
                | b"submodule"
                | b"program"
                | b"function"
                | b"subroutine"
                | b"procedure"
                | b"blockdata"
        )
}

fn named_end_space(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    tokens.get(first).is_some_and(|token| token.is_name(b"end")) && index == first + 2
}

fn is_declaration_entity(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(separator) = tokens[..index]
        .iter()
        .enumerate()
        .filter(|(_, token)| token.depth == 0 && token.text == b"::")
        .map(|(position, _)| position)
        .next_back()
    else {
        return false;
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

fn is_associate_alias_declaration(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(associate) = tokens
        .iter()
        .position(|token| token.is_name(b"associate"))
    else {
        return false;
    };
    let Some(open) = tokens
        .get(associate + 1)
        .filter(|token| token.kind == TokenKind::LParen)
    else {
        return false;
    };
    tokens[index].depth == open.depth + 1
        && tokens.get(index + 1).is_some_and(|token| {
            token.text == b"=>" && token.depth == tokens[index].depth
        })
}

/// The spelling an already-open ASSOCIATE construct gave `name`, or `None` if
/// no enclosing construct declared it.
///
/// Frames are searched innermost first so a nested construct that reuses an
/// outer alias name governs uses inside it. Lowercasing is deferred until there
/// is a frame to search: nearly every file has no ASSOCIATE at all, and this
/// runs on every name token.
fn alias_spelling<'a>(
    enclosing: &'a [HashMap<Vec<u8>, Vec<u8>>],
    name: &[u8],
) -> Option<&'a [u8]> {
    if enclosing.iter().all(HashMap::is_empty) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    enclosing
        .iter()
        .rev()
        .find_map(|frame| frame.get(&lower))
        .map(Vec::as_slice)
}

/// The aliases one ASSOCIATE statement introduces, keyed by their lowercased
/// name and carrying the spelling the author gave each one. That spelling is
/// the alias's declaration: it is what every use inside the block resolves to.
fn associate_opening_aliases(tokens: &[Token<'_>]) -> Option<HashMap<Vec<u8>, Vec<u8>>> {
    let associate = tokens
        .iter()
        .position(|token| token.is_name(b"associate"))?;
    let open = tokens
        .get(associate + 1)
        .filter(|token| token.kind == TokenKind::LParen)?;
    let entry_depth = open.depth + 1;
    let close = tokens
        .iter()
        .enumerate()
        .skip(associate + 2)
        .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == open.depth)
        .map(|(index, _)| index)?;
    let mut aliases = HashMap::new();
    let mut start = associate + 2;
    for end in (start..close)
        .filter(|index| {
            tokens[*index].kind == TokenKind::Comma && tokens[*index].depth == entry_depth
        })
        .chain(std::iter::once(close))
    {
        let entry = &tokens[start..end];
        if entry.len() >= 3
            && entry[0].kind == TokenKind::Name
            && entry[0].depth == entry_depth
            && entry[1].text == b"=>"
            && entry[1].depth == entry_depth
        {
            aliases.insert(entry[0].text.to_ascii_lowercase(), entry[0].text.to_vec());
        }
        start = end.saturating_add(1);
    }
    Some(aliases)
}

fn is_end_associate(tokens: &[Token<'_>]) -> bool {
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    tokens[first].is_name(b"end")
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.is_name(b"associate"))
}
