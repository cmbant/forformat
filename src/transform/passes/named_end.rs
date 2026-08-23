//! A named END restates the name of the scope it closes.
//!
//! `end function cdotc` is not a second occurrence of `cdotc` to be decided on
//! its own evidence: it names the procedure its header already named, so it has
//! to be spelled the way that header ended up spelled. Deciding it separately
//! is what made the ABINIT interface blocks oscillate — the header took the
//! project-wide spelling on the first pass while the named END kept the
//! authored one, and only the second pass moved the END. Running after the case
//! passes, over a scope tree built from their output, removes the second
//! decision instead of trying to make two decisions agree.
//!
//! The rewrite is case-only by construction: a name that does not match its
//! scope's name letter for letter, ignoring case, is left alone.

use crate::{
    analysis::ScopeKind,
    error::FormatError,
    source::{
        tokens::{tokenize, Token, TokenKind},
        LexState,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::provenance::{source_spans, spread_replacement},
        pipeline::{Changed, PassContext},
    },
};

pub fn sync_names(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let mut line_edits: Vec<Vec<(std::ops::Range<usize>, Vec<u8>)>> =
        vec![Vec::new(); document.lines.len()];

    for group in &cx.analysis.groups {
        for statement in &group.statements {
            if !opens_with_end(&statement.text) {
                continue;
            }
            let tokens = tokenize(&statement.text, &mut LexState::default());
            let Some((index, kind)) = end_name(&tokens) else {
                continue;
            };
            let token = &tokens[index];
            let spans = source_spans(group, statement, token);
            let Some((line, _)) = spans.first() else {
                continue;
            };
            let Some(scope) = cx.scopes.scope_of_line(*line) else {
                continue;
            };
            if scope.kind != kind {
                continue;
            }
            let Some(name) = scope.name.as_deref() else {
                continue;
            };
            if name == token.text || !name.eq_ignore_ascii_case(token.text) {
                continue;
            }
            let Some(pieces) = spread_replacement(&spans, token, name) else {
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

/// The trailing name of an `END <kind> <name>` statement and the kind of scope
/// such a statement closes, when the statement is exactly that shape.
fn end_name<'a>(tokens: &'a [Token<'a>]) -> Option<(usize, ScopeKind)> {
    let first = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Number)?;
    if !tokens[first].is_name(b"end") {
        return None;
    }
    let keyword = tokens.get(first + 1)?;
    // `END BLOCK DATA name` spells its keyword in two words; `END BLOCKDATA
    // name` spells it in one. A bare `END BLOCK` closes a BLOCK construct,
    // which is a different scope kind and carries no entity name.
    let (index, kind) = if keyword.is_name(b"block") {
        (
            first + 3,
            tokens
                .get(first + 2)
                .filter(|token| token.is_name(b"data"))
                .map(|_| ScopeKind::Procedure)?,
        )
    } else {
        (first + 2, scope_kind(keyword)?)
    };
    if index + 1 != tokens.len() {
        return None;
    }
    let name = tokens.get(index)?;
    (name.kind == TokenKind::Name).then_some((index, kind))
}

fn scope_kind(keyword: &Token<'_>) -> Option<ScopeKind> {
    if keyword.is_name(b"module") {
        return Some(ScopeKind::Module);
    }
    if keyword.is_name(b"submodule") {
        return Some(ScopeKind::Submodule);
    }
    if keyword.is_name(b"program") {
        return Some(ScopeKind::Program);
    }
    if keyword.is_name(b"function")
        || keyword.is_name(b"subroutine")
        || keyword.is_name(b"procedure")
        || keyword.is_name(b"blockdata")
    {
        return Some(ScopeKind::Procedure);
    }
    if keyword.is_name(b"type") {
        return Some(ScopeKind::DerivedType);
    }
    if keyword.is_name(b"interface") {
        return Some(ScopeKind::Interface);
    }
    None
}

/// Cheap rejection before tokenizing: only a statement whose first word is
/// `END` can carry a named END, and the overwhelming majority of statements in
/// a file are not that.
fn opens_with_end(text: &[u8]) -> bool {
    let start = text
        .iter()
        .position(|byte| !byte.is_ascii_whitespace() && !byte.is_ascii_digit())
        .unwrap_or(text.len());
    text[start..].len() >= 3 && text[start..start + 3].eq_ignore_ascii_case(b"end")
}
