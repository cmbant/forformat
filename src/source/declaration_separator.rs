use super::{
    syntax::{
        declaration_type_head_len, first_statement_index, matching_close, top_level_separator,
    },
    Token, TokenKind,
};

/// Whether a declaration-family statement already has its optional `::`, can
/// receive one at a known token boundary, or does not admit this modernization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclarationSeparator {
    Present,
    Missing { insert_before: usize },
    NotApplicable,
}

/// Locate the optional declaration separator from source shape alone.
///
/// This is deliberately narrower than [`super::syntax::is_declaration_statement`].
/// That predicate is a broad formatter classification; this recognizer answers
/// the grammar-specific question needed by syntax modernization: whether `::`
/// is optional at this statement shape and, when absent, exactly where it may
/// be inserted.
pub(crate) fn declaration_separator(tokens: &[Token<'_>]) -> DeclarationSeparator {
    let first = first_statement_index(tokens);
    let Some(head) = tokens
        .get(first)
        .filter(|token| token.kind == TokenKind::Name)
    else {
        return DeclarationSeparator::NotApplicable;
    };

    if let Some(head_len) = declaration_type_head_len(tokens, first) {
        return type_declaration_separator(tokens, first, head_len);
    }

    if matches_name(
        head,
        &[
            b"allocatable",
            b"asynchronous",
            b"codimension",
            b"contiguous",
            b"dimension",
            b"external",
            b"intrinsic",
            b"optional",
            b"pointer",
            b"protected",
            b"target",
            b"value",
            b"volatile",
            b"public",
            b"private",
            b"final",
            b"enumerator",
        ],
    ) {
        return name_list_separator(tokens, first + 1, false);
    }

    if head.is_name(b"save") {
        return name_list_separator(tokens, first + 1, true);
    }
    if head.is_name(b"intent") {
        return parenthesized_head_separator(tokens, first + 1, false);
    }
    if head.is_name(b"bind") {
        return parenthesized_head_separator(tokens, first + 1, true);
    }
    if head.is_name(b"procedure") {
        return procedure_separator(tokens, first);
    }
    if head.is_name(b"import") {
        return import_separator(tokens, first);
    }
    if head.is_name(b"enumeration")
        && tokens
            .get(first + 1)
            .is_some_and(|token| token.is_name(b"type"))
    {
        return enumeration_type_separator(tokens, first);
    }

    DeclarationSeparator::NotApplicable
}

fn type_declaration_separator(
    tokens: &[Token<'_>],
    first: usize,
    head_len: usize,
) -> DeclarationSeparator {
    if is_select_type_guard(tokens, first) || is_typed_function_header(tokens, first + head_len) {
        return DeclarationSeparator::NotApplicable;
    }

    let mut entity = first + head_len;
    if tokens
        .get(entity)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        entity = match matching_close(tokens, entity) {
            Some(close) => close + 1,
            None => return DeclarationSeparator::NotApplicable,
        };
    } else if tokens
        .get(entity)
        .is_some_and(|token| token.kind == TokenKind::LBracket)
    {
        return DeclarationSeparator::NotApplicable;
    }
    if tokens.get(entity).is_some_and(is_star) {
        entity = match after_star_selector(tokens, entity) {
            Some(index) => index,
            None => return DeclarationSeparator::NotApplicable,
        };
    }

    let Some(token) = tokens.get(entity).filter(|token| token.depth == 0) else {
        return DeclarationSeparator::NotApplicable;
    };
    if is_separator(token) {
        return DeclarationSeparator::Present;
    }
    if token.kind == TokenKind::Comma {
        // Type/component attributes make `::` mandatory rather than optional;
        // report an authored separator, but do not repair an invalid omission.
        return if top_level_separator(tokens).is_some() {
            DeclarationSeparator::Present
        } else {
            DeclarationSeparator::NotApplicable
        };
    }
    if token.kind != TokenKind::Name {
        return DeclarationSeparator::NotApplicable;
    }
    if tokens.iter().skip(entity + 1).any(is_top_level_assignment) {
        // Initializers require an authored separator in a type declaration.
        return DeclarationSeparator::NotApplicable;
    }
    if token.is_name(b"function")
        && tokens
            .get(entity + 1)
            .is_some_and(|next| next.kind == TokenKind::Name)
    {
        return DeclarationSeparator::NotApplicable;
    }

    DeclarationSeparator::Missing {
        insert_before: entity,
    }
}

fn name_list_separator(
    tokens: &[Token<'_>],
    entity: usize,
    allow_common_block: bool,
) -> DeclarationSeparator {
    let Some(token) = tokens.get(entity).filter(|token| token.depth == 0) else {
        return DeclarationSeparator::NotApplicable;
    };
    if is_separator(token) {
        return DeclarationSeparator::Present;
    }
    if token.kind == TokenKind::Name
        || (allow_common_block && token.kind == TokenKind::Operator && token.text == b"/")
    {
        DeclarationSeparator::Missing {
            insert_before: entity,
        }
    } else {
        DeclarationSeparator::NotApplicable
    }
}

fn parenthesized_head_separator(
    tokens: &[Token<'_>],
    open: usize,
    allow_common_block: bool,
) -> DeclarationSeparator {
    let Some(open_token) = tokens.get(open) else {
        return DeclarationSeparator::NotApplicable;
    };
    if open_token.kind != TokenKind::LParen {
        return DeclarationSeparator::NotApplicable;
    }
    let Some(close) = matching_close(tokens, open) else {
        return DeclarationSeparator::NotApplicable;
    };
    name_list_separator(tokens, close + 1, allow_common_block)
}

fn procedure_separator(tokens: &[Token<'_>], first: usize) -> DeclarationSeparator {
    let mut entity = first + 1;
    if tokens
        .get(entity)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        entity = match matching_close(tokens, entity) {
            Some(close) => close + 1,
            None => return DeclarationSeparator::NotApplicable,
        };
    }
    let Some(token) = tokens.get(entity).filter(|token| token.depth == 0) else {
        return DeclarationSeparator::NotApplicable;
    };
    if is_separator(token) {
        return DeclarationSeparator::Present;
    }
    if token.kind == TokenKind::Comma {
        // Procedure attributes require the separator.
        return if top_level_separator(tokens).is_some() {
            DeclarationSeparator::Present
        } else {
            DeclarationSeparator::NotApplicable
        };
    }
    name_list_separator(tokens, entity, false)
}

fn import_separator(tokens: &[Token<'_>], first: usize) -> DeclarationSeparator {
    let entity = first + 1;
    if tokens
        .get(entity)
        .is_some_and(|token| token.kind == TokenKind::Comma && token.depth == 0)
    {
        // `IMPORT, ONLY:`, `IMPORT, NONE`, and `IMPORT, ALL` are distinct forms.
        return DeclarationSeparator::NotApplicable;
    }
    name_list_separator(tokens, entity, false)
}

fn enumeration_type_separator(tokens: &[Token<'_>], first: usize) -> DeclarationSeparator {
    let entity = first + 2;
    let Some(token) = tokens.get(entity).filter(|token| token.depth == 0) else {
        return DeclarationSeparator::NotApplicable;
    };
    if is_separator(token) {
        return DeclarationSeparator::Present;
    }
    if token.kind == TokenKind::Comma {
        // With an access specifier the separator is mandatory.
        return if top_level_separator(tokens).is_some() {
            DeclarationSeparator::Present
        } else {
            DeclarationSeparator::NotApplicable
        };
    }
    name_list_separator(tokens, entity, false)
}

fn is_select_type_guard(tokens: &[Token<'_>], first: usize) -> bool {
    let Some(head) = tokens.get(first) else {
        return false;
    };
    let Some(next) = tokens.get(first + 1) else {
        return false;
    };
    (head.is_name(b"type") && next.is_name(b"is"))
        || (head.is_name(b"class") && (next.is_name(b"is") || next.is_name(b"default")))
}

fn is_typed_function_header(tokens: &[Token<'_>], start: usize) -> bool {
    tokens.iter().enumerate().skip(start).any(|(index, token)| {
        token.depth == 0
            && token.is_name(b"function")
            && tokens
                .get(index + 1)
                .is_some_and(|name| name.depth == 0 && name.kind == TokenKind::Name)
    })
}

fn after_star_selector(tokens: &[Token<'_>], star: usize) -> Option<usize> {
    let selector = tokens.get(star + 1)?;
    if selector.kind == TokenKind::LParen {
        return matching_close(tokens, star + 1).map(|close| close + 1);
    }

    let mut start = star + 2;
    if selector.kind == TokenKind::Name
        && tokens
            .get(start)
            .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        start = matching_close(tokens, start)? + 1;
    }
    Some(start)
}

fn is_separator(token: &Token<'_>) -> bool {
    token.depth == 0 && token.kind == TokenKind::Operator && token.text == b"::"
}

fn is_top_level_assignment(token: &Token<'_>) -> bool {
    token.depth == 0 && token.kind == TokenKind::Operator && matches!(token.text, b"=" | b"=>")
}

fn is_star(token: &Token<'_>) -> bool {
    token.depth == 0 && token.kind == TokenKind::Operator && token.text == b"*"
}

fn matches_name(token: &Token<'_>, names: &[&[u8]]) -> bool {
    names.iter().any(|name| token.is_name(name))
}

#[cfg(test)]
mod tests {
    use super::{declaration_separator, DeclarationSeparator};
    use crate::source::tokens::tokens;

    enum Expected {
        Missing(&'static [u8]),
        Present,
        NotApplicable,
    }

    #[test]
    fn declaration_separator_sites() {
        let cases = [
            ("real x", Expected::Missing(b"x")),
            ("integer*4 i", Expected::Missing(b"i")),
            ("double precision y", Expected::Missing(b"y")),
            ("character*(*) name", Expected::Missing(b"name")),
            ("type(foo) item", Expected::Missing(b"item")),
            ("type box", Expected::Missing(b"box")),
            ("typeof(x) result", Expected::Missing(b"result")),
            ("dimension a(10)", Expected::Missing(b"a")),
            ("allocatable a", Expected::Missing(b"a")),
            ("intent(in) x", Expected::Missing(b"x")),
            ("optional x", Expected::Missing(b"x")),
            ("pointer p", Expected::Missing(b"p")),
            ("target x", Expected::Missing(b"x")),
            ("save x", Expected::Missing(b"x")),
            ("save /state/", Expected::Missing(b"/")),
            ("external foo", Expected::Missing(b"foo")),
            ("intrinsic sin", Expected::Missing(b"sin")),
            ("protected x", Expected::Missing(b"x")),
            ("value x", Expected::Missing(b"x")),
            ("volatile x", Expected::Missing(b"x")),
            ("asynchronous x", Expected::Missing(b"x")),
            ("contiguous x", Expected::Missing(b"x")),
            ("codimension x[*]", Expected::Missing(b"x")),
            ("procedure(cb) handler", Expected::Missing(b"handler")),
            ("procedure handler", Expected::Missing(b"handler")),
            ("bind(c) x", Expected::Missing(b"x")),
            ("bind(c) /state/", Expected::Missing(b"/")),
            ("public foo", Expected::Missing(b"foo")),
            ("private foo", Expected::Missing(b"foo")),
            ("final finish", Expected::Missing(b"finish")),
            ("enumerator red=1", Expected::Missing(b"red")),
            ("import name", Expected::Missing(b"name")),
            ("enumeration type color", Expected::Missing(b"color")),
            ("real :: x", Expected::Present),
            ("intent(in) :: x", Expected::Present),
            ("external :: foo", Expected::Present),
            ("type, public :: box", Expected::Present),
            ("save :: /state/", Expected::Present),
            ("real function f()", Expected::NotApplicable),
            ("real elemental function f()", Expected::NotApplicable),
            ("pointer(i) = x", Expected::NotApplicable),
            ("save(i) = x", Expected::NotApplicable),
            ("real(i) = x", Expected::NotApplicable),
            ("real(i)%x = y", Expected::NotApplicable),
            ("real x = 1", Expected::NotApplicable),
            ("pointer (p, x)", Expected::NotApplicable),
            ("type is(real)", Expected::NotApplicable),
            ("class default", Expected::NotApplicable),
            ("parameter (n=3)", Expected::NotApplicable),
            ("real, dimension(3) x", Expected::NotApplicable),
            ("procedure(iface), pointer p", Expected::NotApplicable),
            ("import, only: x", Expected::NotApplicable),
            ("module procedure foo", Expected::NotApplicable),
            ("save", Expected::NotApplicable),
        ];

        for (source, expected) in cases {
            let statement_tokens = tokens(source.as_bytes());
            let actual = declaration_separator(&statement_tokens);
            match expected {
                Expected::Missing(text) => {
                    let DeclarationSeparator::Missing { insert_before } = actual else {
                        panic!("{source:?}: expected missing separator, got {actual:?}");
                    };
                    assert_eq!(statement_tokens[insert_before].text, text, "{source:?}");
                }
                Expected::Present => {
                    assert_eq!(actual, DeclarationSeparator::Present, "{source:?}");
                }
                Expected::NotApplicable => {
                    assert_eq!(actual, DeclarationSeparator::NotApplicable, "{source:?}");
                }
            }
        }
    }
}
