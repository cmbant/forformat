//! Implicit-typing permission analysis.
//!
//! Case normalization only needs to know whether an unresolved identifier may
//! denote an implicitly typed entity. It does not need to infer the entity's
//! type. Unsupported or malformed syntax therefore produces an uncertain,
//! all-permitted policy which cannot subsequently be narrowed.

use crate::source::{
    tokens::{tokenize, Token, TokenKind},
    LexState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ImplicitPolicy {
    permitted: [bool; 26],
    uncertain: bool,
}

impl ImplicitPolicy {
    pub(super) const ALL: Self = Self {
        permitted: [true; 26],
        uncertain: false,
    };

    const UNKNOWN: Self = Self {
        permitted: [true; 26],
        uncertain: true,
    };

    fn none() -> Self {
        Self {
            permitted: [false; 26],
            uncertain: false,
        }
    }

    pub(super) fn permits(self, name: &[u8]) -> bool {
        let Some(first) = name.first().copied().map(|byte| byte.to_ascii_lowercase()) else {
            return true;
        };
        if !first.is_ascii_lowercase() {
            return true;
        }
        self.permitted[(first - b'a') as usize]
    }

    pub(super) fn apply(self, text: &[u8]) -> Self {
        if self.uncertain {
            return self;
        }
        parse_implicit_policy(self, text).unwrap_or(Self::UNKNOWN)
    }
}

pub(super) fn is_implicit_statement(text: &[u8]) -> bool {
    tokenize(text, &mut LexState::default())
        .into_iter()
        .find(|token| token.kind != TokenKind::Number)
        .is_some_and(|token| token.is_name(b"implicit"))
}

/// True when `index` is one of the one-letter names in an IMPLICIT letter-spec
/// list, rather than an identifier occurrence whose declaration spelling may
/// be propagated into this statement.
pub(crate) fn is_implicit_letter_name(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(token) = tokens.get(index) else {
        return false;
    };
    if one_letter_name(token).is_none() {
        return false;
    }
    let Some(start) = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)
    else {
        return false;
    };
    if !tokens[start].is_name(b"implicit")
        || tokens
            .get(start + 1)
            .is_some_and(|token| token.is_name(b"none"))
    {
        return false;
    }

    let clauses = &tokens[start + 1..];
    let target = index.saturating_sub(start + 1);
    let mut clause_start = 0;
    loop {
        let clause_end = clauses[clause_start..]
            .iter()
            .position(|token| token.kind == TokenKind::Comma && token.depth == 0)
            .map(|offset| clause_start + offset)
            .unwrap_or(clauses.len());
        let clause = &clauses[clause_start..clause_end];
        if let Some((open, close)) = implicit_clause_letter_bounds(clause) {
            let absolute_open = clause_start + open;
            let absolute_close = clause_start + close;
            if target > absolute_open && target < absolute_close {
                return true;
            }
        }
        if clause_end == clauses.len() {
            break;
        }
        clause_start = clause_end + 1;
        if clause_start >= clauses.len() {
            break;
        }
    }
    false
}

/// Apply one supported IMPLICIT statement to an inherited permission mask.
///
/// `None` means the syntax could not be proved valid. The public policy
/// operation converts that into the latched `UNKNOWN` state.
fn parse_implicit_policy(current: ImplicitPolicy, text: &[u8]) -> Option<ImplicitPolicy> {
    let tokens = tokenize(text, &mut LexState::default());
    let tokens = &tokens[..tokens
        .iter()
        .position(|token| token.kind == TokenKind::Comment)
        .unwrap_or(tokens.len())];
    let start = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens[start].is_name(b"implicit") {
        return Some(current);
    }
    let next = tokens.get(start + 1)?;

    if next.is_name(b"none") {
        if start + 2 == tokens.len() {
            return Some(ImplicitPolicy::none());
        }
        tokens
            .get(start + 2)
            .filter(|token| token.kind == TokenKind::LParen)?;
        let close = matching_paren(tokens, start + 2)?;
        if close + 1 != tokens.len() {
            return None;
        }
        let mut expect_name = true;
        let mut saw_name = false;
        let mut disables_type = false;
        for token in &tokens[start + 3..close] {
            if expect_name {
                if token.kind != TokenKind::Name
                    || !(token.is_name(b"type") || token.is_name(b"external"))
                {
                    return None;
                }
                saw_name = true;
                disables_type |= token.is_name(b"type");
            } else if token.kind != TokenKind::Comma {
                return None;
            }
            expect_name = !expect_name;
        }
        if !saw_name || expect_name {
            return None;
        }
        return Some(if disables_type {
            ImplicitPolicy::none()
        } else {
            current
        });
    }

    let mut policy = current;
    let clauses = &tokens[start + 1..];
    let mut clause_start = 0;
    loop {
        let clause_end = clauses[clause_start..]
            .iter()
            .position(|token| token.kind == TokenKind::Comma && token.depth == 0)
            .map(|offset| clause_start + offset)
            .unwrap_or(clauses.len());
        let ranges = implicit_clause_ranges(&clauses[clause_start..clause_end])?;
        for (first, last) in ranges {
            for letter in first..=last {
                policy.permitted[(letter - b'a') as usize] = true;
            }
        }
        if clause_end == clauses.len() {
            break;
        }
        clause_start = clause_end + 1;
        if clause_start == clauses.len() {
            return None;
        }
    }
    Some(policy)
}

fn implicit_clause_letter_bounds(tokens: &[Token<'_>]) -> Option<(usize, usize)> {
    tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.kind == TokenKind::LParen)
        .filter_map(|(open, _)| matching_paren(tokens, open).map(|close| (open, close)))
        .find(|(open, close)| {
            *open > 0
                && *close + 1 == tokens.len()
                && supported_implicit_type_spec(&tokens[..*open]).is_some()
                && implicit_letter_ranges(&tokens[*open + 1..*close]).is_some()
        })
}

fn implicit_clause_ranges(tokens: &[Token<'_>]) -> Option<Vec<(u8, u8)>> {
    let (open, close) = implicit_clause_letter_bounds(tokens)?;
    implicit_letter_ranges(&tokens[open + 1..close])
}

fn supported_implicit_type_spec(tokens: &[Token<'_>]) -> Option<()> {
    let first = tokens.first()?;
    let base_end = if first.is_name(b"double") {
        let second = tokens.get(1)?;
        (second.is_name(b"precision") || second.is_name(b"complex")).then_some(2)?
    } else if first.is_name(b"integer")
        || first.is_name(b"real")
        || first.is_name(b"complex")
        || first.is_name(b"logical")
        || first.is_name(b"character")
        || first.is_name(b"type")
        || first.is_name(b"class")
    {
        1
    } else {
        return None;
    };
    let selector = &tokens[base_end..];
    if selector.is_empty()
        || (selector.first()?.kind == TokenKind::LParen
            && matching_paren(selector, 0)? + 1 == selector.len())
        || (selector.len() == 2
            && selector[0].text == b"*"
            && matches!(selector[1].kind, TokenKind::Name | TokenKind::Number))
    {
        Some(())
    } else {
        None
    }
}

fn matching_paren(tokens: &[Token<'_>], open: usize) -> Option<usize> {
    let depth = tokens.get(open)?.depth;
    tokens
        .iter()
        .enumerate()
        .skip(open + 1)
        .find(|(_, token)| token.kind == TokenKind::RParen && token.depth == depth)
        .map(|(index, _)| index)
}

fn implicit_letter_ranges(tokens: &[Token<'_>]) -> Option<Vec<(u8, u8)>> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let first = one_letter_name(tokens.get(index)?)?;
        index += 1;
        let last = if tokens.get(index).is_some_and(|token| token.text == b"-") {
            index += 1;
            let last = one_letter_name(tokens.get(index)?)?;
            index += 1;
            last
        } else {
            first
        };
        if first > last {
            return None;
        }
        result.push((first, last));
        if index == tokens.len() {
            break;
        }
        if tokens[index].kind != TokenKind::Comma {
            return None;
        }
        index += 1;
        if index == tokens.len() {
            return None;
        }
    }
    (!result.is_empty()).then_some(result)
}

fn one_letter_name(token: &Token<'_>) -> Option<u8> {
    (token.kind == TokenKind::Name && token.text.len() == 1)
        .then(|| token.text[0].to_ascii_lowercase())
        .filter(u8::is_ascii_lowercase)
}

#[cfg(test)]
mod tests {
    use super::{is_implicit_letter_name, ImplicitPolicy};
    use crate::source::{tokens::tokenize, LexState, TokenKind};

    #[test]
    fn malformed_syntax_latches_the_conservative_policy() {
        let policy = ImplicitPolicy::ALL
            .apply(b"implicit none(type)")
            .apply(b"implicit real(a-)")
            .apply(b"implicit none");
        assert!(policy.permits(b"I"));
    }

    #[test]
    fn none_external_does_not_disable_implicit_typing() {
        let policy = ImplicitPolicy::ALL.apply(b"implicit none(external)");
        assert!(policy.permits(b"A"));
        assert!(policy.permits(b"I"));
    }

    #[test]
    fn letter_specs_are_distinct_from_kind_names() {
        let tokens = tokenize(b"implicit real(kind=H) (a-h,o-z)", &mut LexState::default());
        let names = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| token.kind == TokenKind::Name)
            .map(|(index, token)| (token.text, is_implicit_letter_name(&tokens, index)))
            .collect::<Vec<_>>();
        assert!(names.contains(&(b"H".as_slice(), false)));
        for letter in [b"a".as_slice(), b"h", b"o", b"z"] {
            assert!(
                names.contains(&(letter, true)),
                "missing {letter:?}: {names:?}"
            );
        }
    }
}
