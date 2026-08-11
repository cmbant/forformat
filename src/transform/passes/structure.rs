//! Steps 6, 7, 14: the passes that change the line count.
//!
//! Each of these must report [`Changed::Structure`], because the statement and
//! scope views are derived from line numbers and become stale the moment a line
//! appears or disappears.

use crate::{
    error::FormatError,
    transform::{
        document::Document,
        pipeline::{Changed, PassContext},
    },
};

/// Step 6: rejoin `&` splits that cut a token in half.
///
/// A continuation may legally fall inside a name (`some_long_&\n  &name`), and
/// every later pass would see two half-tokens.  Joining them first is what makes
/// the token stream trustworthy.
///
/// Port target: `join_lexical_token_continuations`.
pub fn join_lexical_token_continuations(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}

/// Step 7: remove redundant nested parentheses.
///
/// Eligible: a right-hand side, an `IF` condition, a `DO WHILE` condition.
/// Protected: procedure arguments and `ASSOCIATE` targets, where an extra pair
/// can change meaning or intent.
///
/// Port target: `remove_redundant_nested_parentheses`.
pub fn remove_redundant_nested_parentheses(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}

/// Step 14: drop a bare terminal `RETURN`.
///
/// Only when it is the final single-line statement before a procedure's `END`,
/// and never when it carries an inline comment.
///
/// Port target: `remove_terminal_procedure_returns`.
pub fn remove_terminal_procedure_returns(
    document: &mut Document,
    cx: &PassContext,
) -> Result<Changed, FormatError> {
    let _ = (document, cx);
    Ok(Changed::No)
}
