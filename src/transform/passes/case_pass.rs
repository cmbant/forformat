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
use std::collections::HashSet;
use std::ops::Range;

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

    // A macro is a higher-priority namespace, including when its spelling is
    // ambiguous.  Silence here prevents a declaration from re-casing it.
    if cx.project.macros.contains(token.text) {
        return None;
    }

    // ASSOCIATE aliases are procedure-local names, but their extractor is
    // deliberately deferred to Chunk B.  Protect the alias and its uses here
    // rather than allowing a same-spelled declaration elsewhere in the file
    // to leak through the file-wide table.
    if associate_names
        .is_some_and(|names| names.contains(token.text.to_ascii_lowercase().as_slice()))
    {
        return None;
    }

    // The reference's character scanner sees `e8_dl` (and analogous D/Q
    // exponents) as one identifier beginning at the exponent marker.  Our
    // token stream intentionally exposes `_dl` separately so ordinary kind
    // suffixes can be rewritten; retain the reference's exponent exception.
    if numeric_kind_suffix(tokens, index) {
        return None;
    }
    if is_numeric_literal_kind_name(tokens, index) {
        return file_symbol_spelling(declared_names, cx, token.text);
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
        return resolver_spelling(cx, NameSpace::Type, token.text);
    }

    // A kind selector in an intrinsic type-spec (REAL(DP), COMPLEX(DP), ...)
    // is an ordinary declared parameter in the reference, not a derived-type
    // name.  This also reaches legacy declarations without `::`.
    if is_intrinsic_kind_name(tokens, index) {
        return file_symbol_spelling(declared_names, cx, token.text);
    }

    if preceded_by_percent(tokens, index) {
        let resolver = cx.resolver();
        let procedure = cx
            .scopes
            .ancestors(cx.scopes.index_of_line(line))
            .into_iter()
            .find(|scope| {
                cx.scopes.scopes[*scope].kind == crate::analysis::scope::ScopeKind::Procedure
            })
            .and_then(|scope| cx.scopes.scopes[scope].name.as_deref());
        let owner_type =
            member_owner_type(tokens, index, procedure, &cx.local.types, &cx.project.types);
        let Some(_owner_type) = owner_type else {
            // The typed component table cannot safely reproduce the
            // reference's (type, component) key when the use-site chain is
            // unresolved.
            // I4 therefore requires silence until B8 supplies enough type
            // information to classify this occurrence.
            return None;
        };
        if let Some(spelling) = resolver.spelling(NameSpace::TypeProcedure, token.text) {
            return Some(spelling.to_vec());
        }
        if let Some(spelling) = inherited_component_spelling(cx, &_owner_type, token.text) {
            return Some(spelling.to_vec());
        }
        return file_symbol_spelling(declared_names, cx, token.text);
    }

    // The B9 procedure map contains spellings, not merely membership.  A
    // local ambiguity must block the project-wide fallback.
    if let Some(local) = declared_names.local_at(line) {
        if local.contains(token.text) {
            return local.get(token.text).map(ToOwned::to_owned);
        }
    }
    match declared_names.file_declared_case(line, token.text) {
        Some(Some(spelling)) => Some(spelling.to_vec()),
        Some(None) => None,
        None if declared_names.local_declared_outside(line, token.text) => None,
        None => file_symbol_spelling(declared_names, cx, token.text),
    }
}

/// Resolve a component at its declared owner or one of that type's parents.
/// An exact declaration at the nearest level wins, including an ambiguity;
/// only a genuinely absent entry permits the walk to continue.
fn inherited_component_spelling(cx: &PassContext, owner: &[u8], name: &[u8]) -> Option<Vec<u8>> {
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
        if cx.project.cases.components.contains(&current, name) {
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
        } else if cx.project.types.parent_type_is_ambiguous(&current) {
            return None;
        } else if cx.project.types.parent_types.contains_key(&current) {
            cx.project.types.parent_type(&current)
        } else {
            None
        };
        let parent = parent?;
        current = parent.to_vec();
    }
}

fn resolver_spelling(cx: &PassContext, space: NameSpace, name: &[u8]) -> Option<Vec<u8>> {
    cx.resolver().spelling(space, name).map(ToOwned::to_owned)
}

fn file_symbol_spelling(
    declared_names: &crate::analysis::DeclaredNameIndex,
    cx: &PassContext,
    name: &[u8],
) -> Option<Vec<u8>> {
    match declared_names.file_declared_anywhere(name) {
        Some(Some(spelling)) => Some(spelling.to_vec()),
        Some(None) => None,
        None => resolve(
            &crate::analysis::CaseMap::default(),
            &cx.project.cases.symbols,
            name,
        )
        .map(ToOwned::to_owned),
    }
}

fn preceded_by_percent(tokens: &[Token<'_>], index: usize) -> bool {
    index > 0 && tokens[index - 1].text == b"%"
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
    !initializer
}

fn numeric_kind_suffix(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].text == b"_"
        && tokens[index - 2].kind == TokenKind::Number
        && number_has_exponent(tokens[index - 2].text)
}

fn is_numeric_literal_kind_name(tokens: &[Token<'_>], index: usize) -> bool {
    index >= 2 && tokens[index - 1].text == b"_" && tokens[index - 2].kind == TokenKind::Number
}

fn number_has_exponent(number: &[u8]) -> bool {
    let Some(marker) = number
        .iter()
        .rposition(|byte| matches!(byte, b'e' | b'E' | b'd' | b'D' | b'q' | b'Q'))
    else {
        return false;
    };
    // A signed exponent breaks the reference scanner's identifier at the
    // sign, so the later `_kind` suffix is still visited.  Only `e8_kind`,
    // `d0_kind`, and `q12_kind` remain one identifier in that scanner.
    let index = marker + 1;
    index < number.len()
        && !matches!(number[index], b'+' | b'-')
        && number[index..].iter().all(u8::is_ascii_digit)
}

fn member_owner_type(
    tokens: &[Token<'_>],
    index: usize,
    procedure: Option<&[u8]>,
    local: &TypeMaps,
    project: &TypeMaps,
) -> Option<Vec<u8>> {
    if index < 2 || !preceded_by_percent(tokens, index) {
        return None;
    }
    let mut names = Vec::new();
    let mut cursor = index - 2;
    loop {
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
    let root = names.first()?;
    let links: Vec<&[u8]> = names[1..].to_vec();
    local
        .resolve_chain_with_locals(procedure, root, &links)
        .or_else(|| project.resolve_chain(root, &links))
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
    fn numeric_kind_suffixes_follow_declared_case_but_not_exponent_identifiers() {
        let source = b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_dl\nreal(DL), parameter :: Y = 1.0e8_dl\nend module Constants\n";
        let project = analyze_project([(Path::new("constants.f90"), source.as_slice())]).unwrap();
        assert_eq!(
            run_pass(source, &project, |document, context| {
                declared(document, context).unwrap()
            }),
            b"module Precision\ninteger, parameter :: DL = 8\nend module Precision\nmodule Constants\nuse Precision\nreal(DL), parameter :: X = 1.0_DL\nreal(DL), parameter :: Y = 1.0e8_dl\nend module Constants\n"
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
}
