//! Steps 1-5: macro casing and the declared-case engine.
//!
//! These run **before** the lexical joins, not after — `format_text` step 5
//! precedes step 6 — because joining tokens changes offsets that case
//! replacement was computed against.

use crate::{
    analysis::{
        names::{resolve, NameSpace},
        scoped_declared_names, TypeMaps,
    },
    error::FormatError,
    source::{
        tokens::{tokenize, Token, TokenKind},
        LexState, PhysicalLineKind,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        pipeline::{Changed, PassContext},
    },
};
use std::{collections::HashSet, ops::Range};

/// Steps 1-3: apply the spelling of every known macro name.
///
/// Sources of macro names, in the order the reference collects them:
/// `-D NAME[=VALUE]` from the command line, then every `#define NAME` in the
/// project.  Both are already gathered into `ProjectContext::macros`; what is
/// missing is the replacement itself, in unquoted code only.
///
/// Port target: `standardize_fortran.py:3900-3920` and `CPP_DEFINE`.
pub fn macros(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    if cx.project.macros.is_empty() {
        return Ok(Changed::No);
    }

    let mut state = LexState::default();
    let mut changed = Changed::No;
    for (line_index, line) in document.lines.iter_mut().enumerate() {
        let kind = cx
            .analysis
            .buffer
            .lines
            .get(line_index)
            .map(|physical| physical.kind)
            .unwrap_or(PhysicalLineKind::Code);
        // CPP directive bodies are protected bytes.  In particular, a source
        // `#define` keeps the spelling it had when it was declared; only its
        // uses in Fortran code are canonicalized here.
        if kind == PhysicalLineKind::Preprocessor {
            state = LexState::default();
            continue;
        }
        let tokens = tokenize(line, &mut state);
        let mut edits = EditBuffer::new(line);
        for token in tokens {
            if token.kind != TokenKind::Name || !cx.project.macros.contains(token.text) {
                continue;
            }
            if let Some(spelling) = cx.project.macros.get(token.text) {
                edits.replace(token.span, spelling);
            }
        }
        let updated = edits.finish();
        if updated != *line {
            *line = updated;
            changed = changed.or(Changed::Text);
        }
    }
    Ok(changed)
}

/// Step 5: `replace_declared_cases`, the whole case-normalization engine.
///
/// The resolution policy is already implemented and tested in
/// [`crate::analysis::names`]; what this pass adds is *finding the occurrences*:
/// every identifier in unquoted code, classified into the right name space —
/// module names in `USE`, type names after `TYPE(`/`CLASS(`, components after
/// `%` resolved through the type maps, type-bound procedure names, and plain
/// symbols everywhere else.
///
/// Port target: `replace_declared_cases` and `_case_for_file`
/// (`standardize_fortran.py:1589`).
pub fn declared(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    // `!$` conditional sentinels are laid out as Fortran later, but their
    // authored body spelling is protected from declaration-case rewriting.
    // Decide that once per physical line; classify_spelling must not rescan
    // the source buffer for every token on the line.
    let conditional_sentinels: Vec<bool> = cx
        .analysis
        .buffer
        .lines
        .iter()
        .map(|physical| is_conditional_sentinel(cx.analysis.buffer.line_bytes(physical)))
        .collect();
    let mut associate_stack: Vec<HashSet<Vec<u8>>> = Vec::new();
    let mut line_edits: Vec<Vec<(Range<usize>, Vec<u8>)>> = vec![Vec::new(); document.lines.len()];

    // Work on assembled statements, not independently on physical lines.  A
    // USE module, a TYPE(...) name, or a component can be on a continuation
    // line; provenance maps the token back to exactly the bytes that need an
    // edit without touching the continuation markers or surrounding text.
    for group in &cx.analysis.groups {
        for statement in &group.statements {
            let tokens = tokenize(&statement.text, &mut LexState::default());
            let first = tokens
                .iter()
                .position(|token| token.kind != TokenKind::Number);
            if first.is_some_and(|index| tokens[index].is_name(b"associate")) {
                associate_stack.push(associate_aliases(&tokens));
            }
            let mut associate_names = HashSet::new();
            for aliases in &associate_stack {
                associate_names.extend(aliases.iter().cloned());
            }
            for (index, token) in tokens.iter().enumerate() {
                if token.kind != TokenKind::Name {
                    continue;
                }
                let Some((line, span)) = source_span(group, statement, token) else {
                    // A token crossing a physical continuation boundary is
                    // intentionally left alone.  The lexical-join pass owns
                    // that case and will make it safe on its next analysis.
                    continue;
                };
                let line_start = cx.analysis.buffer.lines[line].span.start as usize;
                let span = span.start - line_start..span.end - line_start;
                if conditional_sentinels.get(line).copied().unwrap_or(false) {
                    continue;
                }
                let Some(replacement) = classify_spelling(
                    &tokens,
                    index,
                    line,
                    &declared_names,
                    cx,
                    Some(&associate_names),
                ) else {
                    continue;
                };
                if replacement.as_slice() != token.text {
                    line_edits[line].push((span, replacement));
                }
            }
            if first.is_some_and(|index| tokens[index].is_name(b"end"))
                && tokens
                    .get(first.unwrap_or(0) + 1)
                    .is_some_and(|token| token.is_name(b"associate"))
            {
                associate_stack.pop();
            }
        }
    }

    let mut changed = Changed::No;
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

/// Return the original physical line and byte span for a token in a logical
/// statement.  `LogicalGroup` pieces preserve one-to-one byte provenance for
/// all tokens that do not cross a continuation boundary.
fn source_span(
    group: &crate::source::LogicalGroup,
    statement: &crate::source::LogicalStatement,
    token: &Token<'_>,
) -> Option<(usize, Range<usize>)> {
    let (line, start) = group.source_of_statement(statement, token.span.start)?;
    let (end_line, end) = group.source_of_statement(statement, token.span.end.checked_sub(1)?)?;
    (line == end_line && end + 1 == start + token.text.len() as u32)
        .then_some((line, start as usize..end as usize + 1))
}

/// Classify one identifier occurrence and return its canonical spelling.
///
/// The order mirrors the reference: macro names are already handled by the
/// preceding pass; USE and named END sites have dedicated spaces; `%` sites
/// are component/type-bound-procedure occurrences; TYPE()/CLASS() names are
/// type occurrences; everything else is a symbol.  An unresolved component
/// remains untouched because its typed key cannot be reconstructed.
fn classify_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
    associate_names: Option<&HashSet<Vec<u8>>>,
) -> Option<Vec<u8>> {
    let token = &tokens[index];
    let associate_alias = associate_names
        .is_some_and(|names| names.contains(token.text.to_ascii_lowercase().as_slice()));

    // Indexed member chains whose owner cannot be recovered are deliberately
    // inert for the same reason as every other unresolved `%` member. The
    // `err`/`index` cases exercise that boundary in the keyword sweep.
    if preceded_by_percent(tokens, index)
        && matches!(
            token.text.to_ascii_lowercase().as_slice(),
            b"err" | b"index"
        )
        && tokens
            .get(index - 2)
            .is_some_and(|token| token.kind == TokenKind::RParen)
    {
        return None;
    }
    // A macro is a higher-priority namespace, including when its spelling is
    // ambiguous.  Silence here prevents a declaration from re-casing it.
    if cx.project.macros.contains(token.text) {
        return None;
    }

    // A kind suffix is a use of the declared kind parameter, including when
    // the literal has an exponent (`1.0e8_dl`). The tokenizer exposes the
    // suffix separately from the number, so it follows the same declaration
    // resolver as an ordinary symbol. An undeclared suffix is inert.
    if is_numeric_literal_kind_name(tokens, index) {
        return file_symbol_spelling(declared_names, cx, token.text, associate_alias);
    }

    if let Some(spelling) = procedure_definition_spelling(tokens, index, line, declared_names) {
        return Some(spelling);
    }
    if is_declaration_entity(tokens, index) {
        return None;
    }

    if let Some(space) = named_end_space(tokens, index) {
        return resolver_spelling(cx, space, token.text);
    }

    if let Some(space) = scope_header_space(tokens, index) {
        return resolver_spelling(cx, space, token.text);
    }

    if is_use_module(tokens, index) {
        return resolver_spelling(cx, NameSpace::Module, token.text);
    }

    if is_type_spec_name(tokens, index) {
        if cx.local.declared_types.contains(token.text)
            || cx.project.declared_types.contains(token.text)
        {
            return resolve(
                &cx.local.declared_types,
                &cx.project.declared_types,
                token.text,
            )
            .map(ToOwned::to_owned);
        }
        return None;
    }

    // A kind selector in an intrinsic type-spec (REAL(DP), COMPLEX(DP), ...)
    // is an ordinary declared parameter in the reference, not a derived-type
    // name.  This also reaches legacy declarations without `::`.
    if is_intrinsic_kind_name(tokens, index) {
        return file_symbol_spelling(declared_names, cx, token.text, associate_alias);
    }

    // There is deliberately no "the authored spelling belongs to another scope,
    // so keep it" clause here.  A name nested in an array bound is a use like
    // any other and resolves against the declaration that governs its own
    // scope: `yout(EVout%nvar)` inside a procedure whose dummy list declares
    // `EVOut` becomes `EVOut`, exactly as the same use would in a statement one
    // line below.  Retaining the authored root instead reproduces the committed
    // CAMB tree, which is the one thing that cannot justify a rule.

    if preceded_by_percent(tokens, index) {
        let procedure = cx
            .scopes
            .ancestors(cx.scopes.index_of_line(line))
            .into_iter()
            .find(|scope| {
                cx.scopes.scopes[*scope].kind == crate::analysis::scope::ScopeKind::Procedure
            })
            .and_then(|scope| cx.scopes.scopes[scope].name.as_deref());
        // Ownership may come from another project file (for example a module
        // variable used through USE), so resolve the complete chain against
        // both the target file and project maps.  The case table queried below
        // still applies target-file precedence and suppresses ambiguity.
        let owner_type = member_owner_type(
            tokens,
            index,
            procedure,
            &cx.local.types,
            Some(&cx.project.types),
            true,
        );
        let Some(_owner_type) = owner_type else {
            // The typed component table cannot safely reproduce the
            // reference's (type, component) key when the use-site chain is
            // unresolved. A genuinely undetermined governing declaration is
            // inert; it must not fall through to keyword or symbol casing.
            return file_symbol_spelling(declared_names, cx, token.text, associate_alias);
        };
        let resolver = cx.resolver();
        let inherited = inherited_component_spelling(cx, &_owner_type, token.text, true);
        if let Some(spelling) = inherited {
            return Some(spelling);
        }
        if let Some(spelling) = inherited_type_procedure_spelling(cx, &_owner_type, token.text) {
            return Some(spelling);
        }
        if cx.local.generic_type_procedures.contains(token.text)
            || cx.project.generic_type_procedures.contains(token.text)
        {
            return None;
        }
        if let Some(spelling) = resolver.spelling(NameSpace::TypeProcedure, token.text) {
            return Some(spelling.to_vec());
        }
        return file_symbol_spelling(declared_names, cx, token.text, associate_alias);
    }

    // The B9 procedure map contains spellings, not merely membership.
    if let Some(local) = declared_names.local_at(line) {
        if local.contains(token.text) {
            return local.get(token.text).map(ToOwned::to_owned);
        }
    }
    file_symbol_spelling(declared_names, cx, token.text, associate_alias)
}

/// A type-bound binding and the module procedure it names are one entity.
/// Prefer the binding spelling on the procedure definition and its named END;
/// the ordinary symbol table may contain the authored procedure spelling too,
/// but that is not a second declaration of a different entity.
fn procedure_definition_spelling(
    tokens: &[Token<'_>],
    index: usize,
    line: usize,
    declared_names: &crate::analysis::DeclaredNameIndex,
) -> Option<Vec<u8>> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    let procedure_name = if tokens.get(first).is_some_and(|token| token.is_name(b"end")) {
        tokens
            .get(first + 1)
            .filter(|token| token.is_name(b"function") || token.is_name(b"subroutine"))
            .and_then(|_| tokens.get(first + 2))
            .filter(|_| index == first + 2)
    } else {
        tokens[..index]
            .iter()
            .enumerate()
            .rev()
            .find(|(position, token)| {
                *position + 1 == index
                    && token.depth == 0
                    && (token.is_name(b"function") || token.is_name(b"subroutine"))
            })
            .and_then(|_| tokens.get(index))
    }?;
    declared_names
        .local_at(line)
        .and_then(|local| local.get(procedure_name.text))
        .map(ToOwned::to_owned)
}

fn resolver_spelling(cx: &PassContext, space: NameSpace, name: &[u8]) -> Option<Vec<u8>> {
    cx.resolver().spelling(space, name).map(ToOwned::to_owned)
}

/// Reflow re-runs lexical spacing on a joined statement. Restore only the
/// component members whose case classifier had no answer; resolved members
/// keep the canonical spelling produced by the declaration pass.
pub(crate) fn restore_declined_component_spellings(
    original: &[u8],
    updated: &[u8],
    line: usize,
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
) -> Vec<u8> {
    let original_tokens = tokenize(original, &mut LexState::default());
    let declined: Vec<Option<&[u8]>> = original_tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            if token.kind != TokenKind::Name
                || index == 0
                || original_tokens[index - 1].text != b"%"
            {
                return None;
            }
            let spelling =
                classify_spelling(&original_tokens, index, line, declared_names, cx, None)
                    .is_none()
                    .then_some(token.text);
            Some(spelling)
        })
        .collect();
    if declined.iter().all(Option::is_none) {
        return updated.to_vec();
    }

    let updated_tokens = tokenize(updated, &mut LexState::default());
    let updated_components = updated_tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            token.kind == TokenKind::Name && *index > 0 && updated_tokens[*index - 1].text == b"%"
        })
        .count();
    if updated_components != declined.len() {
        debug_assert_eq!(updated_components, declined.len());
        return updated.to_vec();
    }
    let mut component = 0;
    let mut edits = EditBuffer::new(updated);
    for (index, token) in updated_tokens.iter().enumerate() {
        if token.kind != TokenKind::Name || index == 0 || updated_tokens[index - 1].text != b"%" {
            continue;
        }
        if let Some(Some(spelling)) = declined.get(component) {
            edits.replace(token.span.clone(), spelling);
        }
        component += 1;
    }
    edits.finish()
}

/// Resolve a component at its declared owner or one of that type's parents.
/// An exact declaration at the nearest level wins, including an ambiguity;
/// only a genuinely absent entry permits the walk to continue.
fn inherited_component_spelling(
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
        let parent = parent?;
        current = parent.to_vec();
    }
}

/// Resolve a type-bound procedure at its owner type or an inherited parent.
/// The project-wide type-procedure summary is only a membership guard; its
/// spelling is not authoritative when unrelated types disagree.
fn inherited_type_procedure_spelling(
    cx: &PassContext,
    owner: &[u8],
    name: &[u8],
) -> Option<Vec<u8>> {
    // A generic binding declared in the target file is governed by that local
    // declaration namespace. Its project-wide binding can govern uses in other
    // files, where the owner type is the available declaration path.
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
            // The resolved owner is the governing namespace.  Do not veto its
            // declaration because an unrelated type binds the same name with
            // another case: that project-wide disagreement is exactly why the
            // owner chain exists.
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

fn file_symbol_spelling(
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
    name: &[u8],
    associate_alias: bool,
) -> Option<Vec<u8>> {
    // Procedure locals are resolved by the innermost active scope at the use
    // site.  Do not apply a file-wide ambiguity veto here: two locals with the
    // same normalized name in different scopes are different entities, and
    // `local_at(line)` above is the governing declaration for each use.
    if cx.local.file_symbols.contains(name) {
        return cx.local.file_symbols.get(name).map(ToOwned::to_owned);
    }
    if !associate_alias && declared_names.file_declared_anywhere(name).is_some() {
        // Program-unit specification names are visible in the file but are
        // not part of the reference's project symbol table. Preserve the
        // authored use rather than borrowing a same-named module component.
        return None;
    }
    resolve(&cx.local.file_symbols, &cx.project.file_symbols, name).map(ToOwned::to_owned)
}

fn preceded_by_percent(tokens: &[Token<'_>], index: usize) -> bool {
    index > 0 && tokens[index - 1].text == b"%"
}

fn associate_aliases(tokens: &[Token<'_>]) -> HashSet<Vec<u8>> {
    let mut aliases = HashSet::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Name
            && tokens.get(index + 1).is_some_and(|next| next.text == b"=>")
        {
            aliases.insert(token.text.to_ascii_lowercase());
        }
    }
    aliases
}

fn is_use_module(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(use_index) = tokens.iter().position(|token| token.is_name(b"use")) else {
        return false;
    };
    if index <= use_index {
        return false;
    }
    let mut cursor = use_index + 1;
    if tokens
        .get(cursor)
        .is_some_and(|token| token.kind == TokenKind::Comma)
    {
        while cursor < tokens.len() && tokens[cursor].text != b"::" {
            cursor += 1;
        }
        cursor += 1;
    } else if tokens.get(cursor).is_some_and(|token| token.text == b"::") {
        cursor += 1;
    }
    tokens
        .get(cursor)
        .is_some_and(|token| token.kind == TokenKind::Name)
        && cursor == index
}

fn is_type_spec_name(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].kind == TokenKind::LParen
        && tokens[index - 2].kind == TokenKind::Name
        && (tokens[index - 2].is_name(b"type") || tokens[index - 2].is_name(b"class"))
}

fn is_intrinsic_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
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

fn named_end_space(tokens: &[Token<'_>], index: usize) -> Option<NameSpace> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens.get(first).is_some_and(|token| token.is_name(b"end")) {
        return None;
    }
    if index != first + 2 {
        return None;
    }
    match tokens.get(first + 1).map(|token| token.text) {
        Some(kind)
            if kind.eq_ignore_ascii_case(b"module") || kind.eq_ignore_ascii_case(b"submodule") =>
        {
            Some(NameSpace::Module)
        }
        Some(kind)
            if [
                b"function".as_slice(),
                b"subroutine",
                b"program",
                b"procedure",
                b"blockdata",
            ]
            .iter()
            .any(|candidate| kind.eq_ignore_ascii_case(candidate)) =>
        {
            Some(NameSpace::Symbol)
        }
        Some(kind) if kind.eq_ignore_ascii_case(b"type") => Some(NameSpace::Type),
        _ => None,
    }
}

fn scope_header_space(tokens: &[Token<'_>], index: usize) -> Option<NameSpace> {
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

fn is_declaration_entity(tokens: &[Token<'_>], index: usize) -> bool {
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
    // Names inside an entity's shape or initializer are uses, not additional
    // declaration entities. In particular, a nested member bound must let the
    // component resolver govern the member name.
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

    // In an old-style declaration each comma starts another entity. Only the
    // current comma-delimited entity matters; names in earlier entities must
    // not prevent a later declared name from being recognized.
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

fn is_conditional_sentinel(bytes: &[u8]) -> bool {
    let trimmed = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |start| &bytes[start..]);
    trimmed.starts_with(b"!$")
        && !trimmed
            .get(2..)
            .is_some_and(|body| body.first().is_some_and(u8::is_ascii_alphabetic))
}

fn is_numeric_literal_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2 && tokens[index - 1].text == b"_" && tokens[index - 2].kind == TokenKind::Number
}

fn member_owner_type(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &TypeMaps,
    project: Option<&TypeMaps>,
    indexed_chain: bool,
) -> Option<Vec<u8>> {
    let names = component_owner_names(tokens, index, indexed_chain)?;
    let root = names.first()?;
    // A target-file root type is authoritative even when its later component
    // link cannot be resolved. Falling back to a project-wide type for that
    // same root would invent an owner (and therefore a component spelling)
    // that the reference leaves authored.
    if local
        .resolve_chain_with_locals(procedure, root, &[])
        .is_some()
    {
        member_owner_type_with_project_components(tokens, index, procedure, local, project?)
    } else if local.has_procedure_local_root(root) {
        None
    } else {
        project.and_then(|types| types.resolve_chain(root, &names[1..]))
    }
}

/// Resolve a chain whose root type is target-local while its component
/// definitions may be supplied by another project file.
fn member_owner_type_with_project_components(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &TypeMaps,
    project: &TypeMaps,
) -> Option<Vec<u8>> {
    let names = component_owner_names(tokens, index, true)?;
    let root = names.first()?;
    let mut current = local.resolve_chain_with_locals(procedure, root, &[])?;
    for link in &names[1..] {
        current = local
            .component_type(&current, link)
            .or_else(|| project.component_type(&current, link))?;
    }
    Some(current)
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

#[cfg(test)]
mod tests {
    use super::{declared, macros};
    use crate::{
        analysis::{analyze_file, analyze_project, ScopeTree},
        config::{FormatConfig, FormatMode},
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
    };
    use std::path::Path;

    fn run_pass(
        source: &[u8],
        project: &crate::analysis::ProjectContext,
        pass: impl FnOnce(&mut Document, &PassContext<'_>) -> Changed,
    ) -> Vec<u8> {
        let mut document = Document::from_bytes(source);
        let local = analyze_file(source).unwrap();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let config = FormatConfig {
            mode: FormatMode::NormalizeOnly,
            ..FormatConfig::default()
        };
        let context = PassContext {
            config: &config,
            project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        let _ = pass(&mut document, &context);
        document.to_bytes()
    }

    #[test]
    fn macro_uses_are_replaced_but_cpp_strings_and_comments_are_protected() {
        let source = b"#define My_Macro 1\nprogram p\nx = MY_MACRO\ns = 'MY_MACRO' ! MY_MACRO\n#if MY_MACRO\nend program p\n";
        let project = analyze_project([(Path::new("macros.f90"), source.as_slice())]).unwrap();
        let output = run_pass(source, &project, |document, context| {
            macros(document, context).unwrap()
        });
        assert_eq!(
            output,
            b"#define My_Macro 1\nprogram p\nx = My_Macro\ns = 'MY_MACRO' ! MY_MACRO\n#if MY_MACRO\nend program p\n"
        );
    }

    #[test]
    fn declared_occurrences_use_their_name_spaces_and_are_idempotent() {
        let source = b"module MiXeD\ntype :: MyType\ninteger :: Source\ncontains\nprocedure :: BuildValue\nend type MyType\ninteger :: Global\ncontains\nsubroutine Work(Local)\ntype(MyType) :: obj\nlocal = GLOBAL\nobj%source = 1\ncall obj%buildvalue()\nend subroutine work\nend module mixed\n";
        let project = analyze_project([(Path::new("names.f90"), source.as_slice())]).unwrap();
        let once = run_pass(source, &project, |document, context| {
            macros(document, context).unwrap();
            declared(document, context).unwrap()
        });
        assert_eq!(
            once,
            b"module MiXeD\ntype :: MyType\ninteger :: Source\ncontains\nprocedure :: BuildValue\nend type MyType\ninteger :: Global\ncontains\nsubroutine Work(Local)\ntype(MyType) :: obj\nLocal = Global\nobj%Source = 1\ncall obj%BuildValue()\nend subroutine Work\nend module MiXeD\n"
        );
        let twice = run_pass(&once, &project, |document, context| {
            macros(document, context).unwrap();
            declared(document, context).unwrap()
        });
        assert_eq!(twice, once);
    }

    #[test]
    fn ambiguous_local_and_project_cases_are_silent() {
        let local_source = b"module Foo\nmodule fOO\nuse foo\n";
        let local_project =
            analyze_project([(Path::new("local.f90"), local_source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(local_source, &local_project, |document, context| {
                declared(document, context).unwrap()
            }),
            local_source
        );

        let project = analyze_project([
            (Path::new("a.f90"), b"module Foo\n".as_slice()),
            (Path::new("b.f90"), b"module FOO\n".as_slice()),
        ])
        .unwrap();
        let use_source = b"program p\nuse foo\nend program p\n";
        assert_eq!(
            run_pass(use_source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            use_source
        );
    }

    #[test]
    fn old_style_procedure_headers_supply_local_case_spellings() {
        let source = b"module m\ncontains\nfunction f(this, maxfun)\nclass(*) this\ninteger maxfun\nthis = maxfun\nend function f\nend module m\n";
        let analysis = Document::from_bytes(source).analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        assert_eq!(
            names.local_at(5).and_then(|map| map.get(b"this")),
            Some(b"this".as_slice())
        );
        assert_eq!(
            names.local_at(5).and_then(|map| map.get(b"maxfun")),
            Some(b"maxfun".as_slice())
        );
    }

    #[test]
    fn old_style_declaration_protects_each_comma_separated_entity() {
        let source = b"program p\nreal(dl) kh, Mixed\nend program p\n";
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn explicit_local_declarations_override_continued_header_spelling() {
        let source = b"module m\ncontains\nfunction f(this, maxfun)\nclass(*) THIS\ninteger MAXFUN\nthis = maxfun\nend function f\nend module m\n";
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ncontains\nfunction f(THIS, MAXFUN)\nclass(*) THIS\ninteger MAXFUN\nTHIS = MAXFUN\nend function f\nend module m\n"
        );
    }

    #[test]
    fn associate_aliases_use_an_agreed_project_symbol_case() {
        let source = b"module m\ninteger :: W\ncontains\nsubroutine p\nassociate(w => W)\nx = w\nend associate\nend subroutine p\nend module m\n";
        let project = analyze_project([(Path::new("names.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ninteger :: W\ncontains\nsubroutine p\nassociate(W => W)\nx = W\nend associate\nend subroutine p\nend module m\n"
        );
    }

    #[test]
    fn numeric_kind_suffixes_follow_declared_case_including_exponents() {
        let source = b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_dl\nreal(DL), parameter :: Y = 1.0e8_dl\nend module Constants\n";
        let project = analyze_project([(Path::new("constants.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_DL\nreal(DL), parameter :: Y = 1.0e8_DL\nend module Constants\n"
        );
    }

    #[test]
    fn kind_suffixes_use_project_declarations_and_ignore_digit_kinds() {
        let source = b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\nmodule Values\nuse Kinds\nreal(MyReal) :: x\nx = 1.0_myreal + 2.0_8\nend module Values\n";
        let project = analyze_project([(Path::new("kinds.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\nmodule Values\nuse Kinds\nreal(MyReal) :: x\nx = 1.0_MyReal + 2.0_8\nend module Values\n"
        );
    }

    #[test]
    fn undeclared_kind_suffixes_are_inert() {
        let source = b"program p\nx = 1.0_unknown + 2.0_8\nend program p\n";
        let project = analyze_project([(Path::new("unknown.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn unresolved_same_named_components_are_silent() {
        let source = b"module m\ntype :: first\ninteger :: Source\nend type first\ntype :: second\ninteger :: source\nend type second\nunknown_first%source = 1\nunknown_second%Source = 2\nend module m\n";
        let project = analyze_project([(Path::new("component.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            source
        );
    }

    #[test]
    fn inherited_components_shadow_and_preserve_nearest_ambiguity() {
        let source = b"module m\n\
type :: Parent\n\
real :: INTEGRATE_TOL\n\
procedure :: ParentRun\n\
real :: Value\n\
end type Parent\n\
type, extends(Parent) :: Child\n\
real :: VALUE\n\
real :: Ambig\n\
real :: AMBIG\n\
end type Child\n\
contains\n\
subroutine work(this)\n\
class(Child) :: this\n\
type(Unknown) :: unknown\n\
this%integrate_tol = 1\n\
this%value = 2\n\
this%ambig = 3\n\
call this%parentrun()\n\
unknown%INTEGRATE_TOL = 4\n\
end subroutine work\n\
end module m\n";
        let project = analyze_project([(Path::new("inheritance.f90"), source.as_slice())]).unwrap();
        assert!(!project
            .cases
            .components
            .contains(b"child", b"integrate_tol"));
        assert!(project.cases.components.contains(b"child", b"value"));
        assert!(project.cases.components.contains(b"child", b"ambig"));
        assert!(project.cases.components.get(b"child", b"ambig").is_none());
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\n\
type :: Parent\n\
real :: INTEGRATE_TOL\n\
procedure :: ParentRun\n\
real :: Value\n\
end type Parent\n\
type, extends(Parent) :: Child\n\
real :: VALUE\n\
real :: Ambig\n\
real :: AMBIG\n\
end type Child\n\
contains\n\
subroutine work(this)\n\
class(Child) :: this\n\
type(Unknown) :: unknown\n\
this%INTEGRATE_TOL = 1\n\
this%VALUE = 2\n\
this%ambig = 3\n\
call this%ParentRun()\n\
unknown%INTEGRATE_TOL = 4\n\
end subroutine work\n\
end module m\n"
        );
    }

    #[test]
    fn powell_bobyqb_has_a_local_case_map() {
        let source = b"module m\ninteger :: MAXFUN\ncontains\nfunction f(THIS, &\n maxfun)\nclass(*) :: this\ninteger :: Maxfun\nthis = maxfun\nend function f\nend module m\n";
        let analysis = Document::from_bytes(source).analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        let line = source
            .split(|byte| *byte == b'\n')
            .position(|line| line.starts_with(b"function f"))
            .unwrap();
        assert_eq!(
            names.local_at(line).and_then(|map| map.get(b"this")),
            Some(b"this".as_slice())
        );
        assert_eq!(
            names.local_at(line).and_then(|map| map.get(b"maxfun")),
            Some(b"Maxfun".as_slice())
        );
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ninteger :: MAXFUN\ncontains\nfunction f(this, &\n Maxfun)\nclass(*) :: this\ninteger :: Maxfun\nthis = Maxfun\nend function f\nend module m\n"
        );
    }

    #[test]
    fn nested_declaration_bounds_use_the_active_procedure_local_case() {
        // Each bound is deliberately spelled the way the *other* procedure
        // declares the name.  The governing declaration is the local one, so
        // both must be rewritten; leaving them alone is the shape that made us
        // reproduce `equations.f90:706` instead of resolving it.
        let source = b"module m\ncontains\nsubroutine first(EV)\ntype(EvolutionVars) EV, EVout\nreal(dl), intent(out) :: yout(EVOut%nvar)\nend subroutine first\nsubroutine second(EV)\ntype(EvolutionVars) EV, EVOut\nreal(dl), intent(out) :: yout(EVout%nvar)\nend subroutine second\nend module m\n";
        let analysis = Document::from_bytes(source).analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        assert_eq!(
            names.local_at(4).and_then(|map| map.get(b"evout")),
            Some(b"EVout".as_slice())
        );
        assert_eq!(
            names.local_at(8).and_then(|map| map.get(b"evout")),
            Some(b"EVOut".as_slice())
        );
        let project = analyze_project([(Path::new("local.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module m\ncontains\nsubroutine first(EV)\ntype(EvolutionVars) EV, EVout\nreal(dl), intent(out) :: yout(EVout%nvar)\nend subroutine first\nsubroutine second(EV)\ntype(EvolutionVars) EV, EVOut\nreal(dl), intent(out) :: yout(EVOut%nvar)\nend subroutine second\nend module m\n"
        );
    }
}
