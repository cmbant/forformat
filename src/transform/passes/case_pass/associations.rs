//! ASSOCIATE and SELECT TYPE/RANK scope tracking.

use super::members::resolve_component_owner;
use crate::{
    analysis::{project::ResolvedType, TypeMaps},
    source::tokens::{Token, TokenKind},
    transform::pipeline::PassContext,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub(super) struct AssociateFrame {
    pub(super) names: HashSet<Vec<u8>>,
    pub(super) types: HashMap<Vec<u8>, Vec<u8>>,
    pub(super) spellings: HashMap<Vec<u8>, Vec<u8>>,
    pub(super) resolved_types: HashMap<Vec<u8>, ResolvedType>,
}

impl AssociateFrame {
    pub(super) fn extend_visible(&mut self, frame: &Self) {
        for name in &frame.names {
            self.names.insert(name.clone());
            // An untyped inner alias shadows a typed outer alias with the same
            // name instead of exposing the outer entity by accident.
            self.types.remove(name);
            self.spellings.remove(name);
            self.resolved_types.remove(name);
            if let Some(type_name) = frame.types.get(name) {
                self.types.insert(name.clone(), type_name.clone());
            }
            if let Some(spelling) = frame.spellings.get(name) {
                self.spellings.insert(name.clone(), spelling.clone());
            }
            if let Some(owner) = frame.resolved_types.get(name) {
                self.resolved_types.insert(name.clone(), owner.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectAssociationKind {
    Type,
    Rank,
}

#[derive(Debug, Clone)]
pub(super) enum AssociationScope {
    Associate(AssociateFrame),
    Select {
        kind: SelectAssociationKind,
        alias: Vec<u8>,
        base: AssociateFrame,
        active: Box<AssociateFrame>,
    },
}

impl AssociationScope {
    pub(super) fn frame(&self) -> &AssociateFrame {
        match self {
            Self::Associate(frame) => frame,
            Self::Select { active, .. } => active,
        }
    }
}

#[derive(Debug)]
pub(super) struct SelectAssociationSpec<'a> {
    kind: SelectAssociationKind,
    pub(super) alias_index: usize,
    alias: &'a [u8],
    selector: &'a [Token<'a>],
    pub(super) explicit_alias: bool,
}

pub(super) fn association_opening_scope(
    tokens: &[Token<'_>],
    first: Option<usize>,
    line: usize,
    procedure: Option<&[u8]>,
    cx: &PassContext,
    outer: &AssociateFrame,
) -> Option<AssociationScope> {
    if associate_opening(tokens, first).is_some() {
        return Some(AssociationScope::Associate(associate_frame(
            tokens, line, procedure, cx, outer,
        )));
    }
    let spec = select_association_spec(tokens, first)?;
    let alias = spec.alias.to_ascii_lowercase();
    let mut frame = AssociateFrame::default();
    insert_association(
        &mut frame,
        spec.alias,
        spec.selector,
        line,
        procedure,
        cx,
        outer,
    );
    if spec.explicit_alias {
        frame.spellings.insert(alias.clone(), spec.alias.to_vec());
    }
    Some(AssociationScope::Select {
        kind: spec.kind,
        alias,
        base: frame.clone(),
        active: Box::new(frame),
    })
}

pub(super) fn apply_select_guard(
    tokens: &[Token<'_>],
    line: usize,
    cx: &PassContext,
    stack: &mut [AssociationScope],
) {
    let Some(AssociationScope::Select {
        kind,
        alias,
        base,
        active,
    }) = stack.last_mut()
    else {
        return;
    };
    match kind {
        SelectAssociationKind::Type => {
            let Some(guard) = select_type_guard_name(tokens) else {
                return;
            };
            **active = base.clone();
            let Some(type_name) = guard else {
                return;
            };
            active.types.remove(alias.as_slice());
            active.resolved_types.remove(alias.as_slice());
            if let Some(owner) = cx.project.visible_type(cx.local, line, type_name) {
                active.types.insert(alias.clone(), owner.name.clone());
                active.resolved_types.insert(alias.clone(), owner);
            }
        }
        SelectAssociationKind::Rank => {
            if is_select_rank_guard(tokens) {
                **active = base.clone();
            }
        }
    }
}

pub(super) fn select_association_spec<'a>(
    tokens: &'a [Token<'a>],
    first: Option<usize>,
) -> Option<SelectAssociationSpec<'a>> {
    let (select, kind) = select_type_rank_opening(tokens, first)?;
    let compact = tokens[select].is_name(b"selecttype") || tokens[select].is_name(b"selectrank");
    let open_index = select + if compact { 1 } else { 2 };
    let open = tokens
        .get(open_index)
        .filter(|token| token.kind == TokenKind::LParen)?;
    let close = tokens
        .iter()
        .enumerate()
        .skip(open_index + 1)
        .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == open.depth)
        .map(|(index, _)| index)?;
    let entry = &tokens[open_index + 1..close];
    if let [alias, arrow, selector @ ..] = entry {
        if alias.kind == TokenKind::Name
            && alias.depth == open.depth + 1
            && arrow.text == b"=>"
            && arrow.depth == alias.depth
            && !selector.is_empty()
        {
            return Some(SelectAssociationSpec {
                kind,
                alias_index: open_index + 1,
                alias: alias.text,
                selector,
                explicit_alias: true,
            });
        }
    }
    let alias = entry
        .first()
        .filter(|token| entry.len() == 1 && token.kind == TokenKind::Name)?;
    Some(SelectAssociationSpec {
        kind,
        alias_index: open_index + 1,
        alias: alias.text,
        selector: entry,
        explicit_alias: false,
    })
}

pub(super) fn associate_spelling<'a>(frame: &'a AssociateFrame, name: &[u8]) -> Option<&'a [u8]> {
    frame
        .spellings
        .get(name.to_ascii_lowercase().as_slice())
        .map(Vec::as_slice)
}

pub(super) fn is_associate_alias_declaration(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(associate) = tokens.iter().position(|token| token.is_name(b"associate")) else {
        return false;
    };
    let Some(open) = tokens
        .get(associate + 1)
        .filter(|token| token.kind == TokenKind::LParen)
    else {
        return false;
    };
    tokens[index].depth == open.depth + 1
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.text == b"=>" && token.depth == tokens[index].depth)
}

pub(super) fn is_select_alias_declaration(tokens: &[Token<'_>], index: usize) -> bool {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number);
    select_association_spec(tokens, first)
        .is_some_and(|spec| spec.explicit_alias && spec.alias_index == index)
}

pub(super) fn is_select_type_rank_keyword(tokens: &[Token<'_>], index: usize) -> bool {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number);
    if let Some((select, _)) = select_type_rank_opening(tokens, first) {
        return index == select || (tokens[select].is_name(b"select") && index == select + 1);
    }
    let Some(first) = first else {
        return false;
    };
    if tokens[first].is_name(b"type") || tokens[first].is_name(b"class") {
        let guard = tokens
            .get(first + 1)
            .is_some_and(|token| token.is_name(b"is") || token.is_name(b"default"));
        return guard && (index == first || index == first + 1);
    }
    if tokens[first].is_name(b"rank") {
        let guard = tokens
            .get(first + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen || token.is_name(b"default"));
        return guard
            && (index == first || (index == first + 1 && tokens[index].is_name(b"default")));
    }
    false
}

fn select_type_rank_opening(
    tokens: &[Token<'_>],
    first: Option<usize>,
) -> Option<(usize, SelectAssociationKind)> {
    let first = first?;
    let select = if tokens[first].kind == TokenKind::Name
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.text == b":")
    {
        first + 2
    } else {
        first
    };

    if tokens
        .get(select)
        .is_some_and(|token| token.is_name(b"selecttype"))
    {
        return Some((select, SelectAssociationKind::Type));
    }
    if tokens
        .get(select)
        .is_some_and(|token| token.is_name(b"selectrank"))
    {
        return Some((select, SelectAssociationKind::Rank));
    }
    if !tokens
        .get(select)
        .is_some_and(|token| token.is_name(b"select"))
    {
        return None;
    }

    let kind = if tokens
        .get(select + 1)
        .is_some_and(|token| token.is_name(b"type"))
    {
        SelectAssociationKind::Type
    } else if tokens
        .get(select + 1)
        .is_some_and(|token| token.is_name(b"rank"))
    {
        SelectAssociationKind::Rank
    } else {
        return None;
    };
    Some((select, kind))
}

fn select_type_guard_name<'a>(tokens: &'a [Token<'a>]) -> Option<Option<&'a [u8]>> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !(tokens[first].is_name(b"type") || tokens[first].is_name(b"class")) {
        return None;
    }
    if tokens
        .get(first + 1)
        .is_some_and(|token| token.is_name(b"default"))
    {
        return Some(None);
    }
    if !tokens
        .get(first + 1)
        .is_some_and(|token| token.is_name(b"is"))
    {
        return None;
    }
    let open = tokens
        .get(first + 2)
        .filter(|token| token.kind == TokenKind::LParen)?;
    let name = tokens
        .get(first + 3)
        .filter(|token| token.kind == TokenKind::Name && token.depth == open.depth + 1)?;
    Some(Some(name.text))
}

fn is_select_rank_guard(tokens: &[Token<'_>]) -> bool {
    let Some(first) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    tokens[first].is_name(b"rank")
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen || token.is_name(b"default"))
}

fn associate_opening(tokens: &[Token<'_>], first: Option<usize>) -> Option<usize> {
    let first = first?;
    if tokens[first].is_name(b"associate") {
        return Some(first);
    }
    (tokens[first].kind == TokenKind::Name
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.text == b":")
        && tokens
            .get(first + 2)
            .is_some_and(|token| token.is_name(b"associate")))
    .then_some(first + 2)
}

fn associate_frame(
    tokens: &[Token<'_>],
    line: usize,
    procedure: Option<&[u8]>,
    cx: &PassContext,
    outer: &AssociateFrame,
) -> AssociateFrame {
    let mut frame = AssociateFrame::default();
    for (alias, selector) in associate_specs(tokens) {
        insert_association(&mut frame, alias, selector, line, procedure, cx, outer);
        frame
            .spellings
            .insert(alias.to_ascii_lowercase(), alias.to_vec());
    }
    frame
}

fn insert_association(
    frame: &mut AssociateFrame,
    alias: &[u8],
    selector: &[Token<'_>],
    line: usize,
    procedure: Option<&[u8]>,
    cx: &PassContext,
    outer: &AssociateFrame,
) {
    let name = alias.to_ascii_lowercase();
    frame.names.insert(name.clone());
    if let Some(type_name) = designator_type(
        selector,
        procedure,
        cx.local,
        Some(&cx.project.types),
        outer,
    ) {
        frame.types.insert(name.clone(), type_name);
    }
    if let Some(names) = designator_names(selector) {
        let root = names[0].to_ascii_lowercase();
        let mut resolved = outer
            .resolved_types
            .get(root.as_slice())
            .cloned()
            .or_else(|| cx.project.visible_variable_type(cx.local, line, &root));
        for link in &names[1..] {
            resolved = resolved.and_then(|owner| {
                cx.project
                    .visible_component_type(cx.local, line, &owner, link)
            });
        }
        if let Some(resolved) = resolved {
            frame.resolved_types.insert(name, resolved);
        }
    }
}

fn associate_specs<'a>(tokens: &'a [Token<'a>]) -> Vec<(&'a [u8], &'a [Token<'a>])> {
    let Some(associate) = tokens.iter().position(|token| token.is_name(b"associate")) else {
        return Vec::new();
    };
    let Some(open) = tokens
        .get(associate + 1)
        .filter(|token| token.kind == TokenKind::LParen)
    else {
        return Vec::new();
    };
    let entry_depth = open.depth + 1;
    let close = tokens
        .iter()
        .enumerate()
        .skip(associate + 2)
        .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == open.depth)
        .map(|(index, _)| index)
        .unwrap_or(tokens.len());

    let mut specs = Vec::new();
    let mut start = associate + 2;
    for end in (start..close)
        .filter(|index| {
            tokens[*index].kind == TokenKind::Comma && tokens[*index].depth == entry_depth
        })
        .chain(std::iter::once(close))
    {
        let entry = &tokens[start..end];
        if let [alias, arrow, selector @ ..] = entry {
            if alias.kind == TokenKind::Name
                && alias.depth == entry_depth
                && arrow.text == b"=>"
                && arrow.depth == entry_depth
                && !selector.is_empty()
            {
                specs.push((alias.text, selector));
            }
        }
        start = end.saturating_add(1);
    }
    specs
}

fn designator_type(
    tokens: &[Token<'_>],
    procedure: Option<&[u8]>,
    local: &crate::analysis::FileFacts,
    project: Option<&TypeMaps>,
    associates: &AssociateFrame,
) -> Option<Vec<u8>> {
    let names = designator_names(tokens)?;
    let root = names.first()?;
    if associates
        .names
        .contains(root.to_ascii_lowercase().as_slice())
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
        let current = local
            .types
            .resolve_chain_with_locals(procedure, root, &[])?;
        return resolve_component_owner(current, &names[1..], &local.types, project);
    }
    if procedure.is_none() && local.types.has_procedure_local_root(root) {
        return None;
    }
    if let (Some(project), Some(imported)) = (
        project,
        project.and_then(|types| local.imported_variable_type(types, root)),
    ) {
        return resolve_component_owner(imported, &names[1..], &local.types, Some(project));
    }
    project.and_then(|types| types.resolve_chain(root, &names[1..]))
}

fn designator_names<'a>(tokens: &'a [Token<'a>]) -> Option<Vec<&'a [u8]>> {
    if tokens.first()?.kind != TokenKind::Name {
        return None;
    }
    let base_depth = tokens[0].depth;
    let mut names = vec![tokens[0].text];
    let mut index = 1;
    while index < tokens.len() {
        if tokens[index].kind.opens_bracket() && tokens[index].depth == base_depth {
            let close_kind = match tokens[index].kind {
                TokenKind::LParen => TokenKind::RParen,
                TokenKind::LBracket => TokenKind::RBracket,
                _ => return None,
            };
            index += 1;
            while index < tokens.len()
                && !(tokens[index].kind == close_kind && tokens[index].depth == base_depth)
            {
                index += 1;
            }
            if index == tokens.len() {
                return None;
            }
            index += 1;
            continue;
        }
        if tokens[index].text != b"%" || tokens[index].depth != base_depth {
            return None;
        }
        let member = tokens.get(index + 1)?;
        if member.kind != TokenKind::Name || member.depth != base_depth {
            return None;
        }
        names.push(member.text);
        index += 2;
    }
    Some(names)
}
