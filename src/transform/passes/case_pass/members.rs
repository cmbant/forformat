//! Component-owner resolution and inherited member spelling.

use super::{associations::AssociateFrame, syntax::preceded_by_percent};
use crate::{
    analysis::{project::ResolvedType, TypeMaps},
    source::tokens::{Token, TokenKind},
    transform::pipeline::PassContext,
};
use std::collections::HashSet;

pub(super) fn exact_member_owner(
    names: &[&[u8]],
    line: usize,
    cx: &PassContext,
    associates: Option<&AssociateFrame>,
) -> Option<ResolvedType> {
    let root = *names.first()?;
    let mut current = associates
        .and_then(|frame| frame.resolved_types.get(root).cloned())
        .or_else(|| cx.project.visible_variable_type(cx.local, line, root))?;
    for &link in &names[1..] {
        current = cx
            .project
            .visible_component_type(cx.local, line, &current, link)?;
    }
    Some(current)
}

pub(super) fn inherited_component_spelling(
    cx: &PassContext,
    owner: &[u8],
    name: &[u8],
    allow_project: bool,
) -> Option<Vec<u8>> {
    if cx.project.macros.contains(name) {
        return cx.project.macros.get(name).map(ToOwned::to_owned);
    }

    let mut current = owner.to_ascii_lowercase();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current.clone()) {
            return None;
        }
        if cx.local.cases.components.contains(&current, name) {
            return cx
                .local
                .cases
                .components
                .get(&current, name)
                .map(ToOwned::to_owned);
        }
        if allow_project && cx.project.cases.components.contains(&current, name) {
            return cx
                .project
                .cases
                .components
                .get(&current, name)
                .map(ToOwned::to_owned);
        }
        let parent = if cx.local.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.local.types.parent_types.contains_key(&current) {
            cx.local.types.parent_type(&current)
        } else if allow_project && cx.project.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if allow_project && cx.project.types.parent_types.contains_key(&current) {
            cx.project.types.parent_type(&current)
        } else {
            None
        };
        current = parent?.to_vec();
    }
}

pub(super) fn inherited_type_procedure_spelling(
    cx: &PassContext,
    owner: &[u8],
    name: &[u8],
) -> Option<Vec<u8>> {
    if cx.local.generic_type_procedures.contains(name) {
        return None;
    }
    let resolver = cx.resolver();
    let mut current = owner.to_ascii_lowercase();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current.clone()) {
            return None;
        }
        if let Some(spelling) = resolver.type_procedure_spelling(&current, name) {
            return Some(spelling.to_vec());
        }
        if let Some(spelling) = cx.project.generic_bound_type_procedures.get(&current, name) {
            return Some(spelling.to_vec());
        }
        let parent = if cx.local.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.local.types.parent_types.contains_key(&current) {
            cx.local.types.parent_type(&current)
        } else if cx.project.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.project.types.parent_types.contains_key(&current) {
            cx.project.types.parent_type(&current)
        } else {
            None
        };
        current = parent?.to_vec();
    }
}

pub(super) fn member_owner_type(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &crate::analysis::FileFacts,
    project: Option<&TypeMaps>,
    indexed_chain: bool,
    associates: Option<&AssociateFrame>,
) -> Option<Vec<u8>> {
    let names = component_owner_names(tokens, index, indexed_chain)?;
    let root = names.first()?;
    if let Some(associates) =
        associates.filter(|context| context.names.contains(root.to_ascii_lowercase().as_slice()))
    {
        let current = associates
            .types
            .get(root.to_ascii_lowercase().as_slice())?
            .clone();
        return resolve_component_owner(current, &names[1..], &local.types, project);
    }
    if local
        .types
        .resolve_chain_with_locals(procedure, root, &[])
        .is_some()
    {
        member_owner_type_with_project_components(tokens, index, procedure, &local.types, project)
    } else if procedure.is_none() && local.types.has_procedure_local_root(root) {
        None
    } else if let (Some(project), Some(imported)) = (
        project,
        project.and_then(|types| local.imported_variable_type(types, root)),
    ) {
        resolve_component_owner(imported, &names[1..], &local.types, Some(project))
    } else {
        project.and_then(|types| types.resolve_chain(root, &names[1..]))
    }
}

fn member_owner_type_with_project_components(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &TypeMaps,
    project: Option<&TypeMaps>,
) -> Option<Vec<u8>> {
    let names = component_owner_names(tokens, index, true)?;
    let root = names.first()?;
    let current = local.resolve_chain_with_locals(procedure, root, &[])?;
    resolve_component_owner(current, &names[1..], local, project)
}

pub(super) fn resolve_component_owner(
    mut current: Vec<u8>,
    links: &[&[u8]],
    local: &TypeMaps,
    project: Option<&TypeMaps>,
) -> Option<Vec<u8>> {
    for link in links {
        current = local
            .component_type(&current, link)
            .or_else(|| project.and_then(|types| types.component_type(&current, link)))?;
    }
    Some(current)
}

pub(super) fn component_owner_names<'a>(
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
