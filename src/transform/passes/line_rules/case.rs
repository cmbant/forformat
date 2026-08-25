use super::*;
use crate::{source::syntax::is_end_construct_keyword, transform::vocab_2023};

/// Rule 1: keyword case, and the case decisions the project agreed on.
///
/// A word is cased only when it is a Fortran keyword and nothing in the file or
/// project declares an identifier by that name, and it is not a macro name. Derived-type
/// components remain owned by the declared-case pass; the one exception is the standard
/// complex-part designators `%RE` and `%IM`, which follow keyword case when no tracked
/// derived-type component spelling governs the occurrence.
pub fn lowercase_line(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    state: &mut LexState,
) -> Vec<u8> {
    lowercase_line_with_context(
        line,
        cx,
        declared_names,
        line_index,
        state,
        &super::super::LineContext::default(),
    )
}

pub(in crate::transform::passes::line_rules) fn lowercase_line_with_context(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    state: &mut LexState,
    context: &super::super::LineContext<'_>,
) -> Vec<u8> {
    let tokens = tokenize(line, state);
    let inside_paren = inside_paren_at(line, context.open_groups, &tokens);
    // A declaration's `::` need not be on the physical line that is being
    // cased. Until it has been passed, the statement is still in its attribute
    // half, and both of the judgements below have to be made from the whole
    // statement rather than from this line's tokens.
    let separator_pending = context.statement_separator && !context.continued_separator;
    let separator_below = separator_pending && !tokens.iter().any(|token| token.text == b"::");
    let continued_entity_list = (context.continued_declaration || context.continued_separator)
        && context.open_groups.is_empty()
        && !context.continued_initializer
        && !separator_pending;
    let normalize_whitespace = cx.config.mode.normalizes_whitespace();
    let mut edits = EditBuffer::new(line);
    if !is_format_statement(&tokens) && !context.continued_format {
        for pair in tokens.windows(2) {
            if pair[0].kind == TokenKind::Name
                && pair[1].kind == TokenKind::Name
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
                && is_fortran_2023_multiword_pair(pair[0].text, pair[1].text)
            {
                // Multiword language tokens have a canonical one-space spelling.
                // This is a token-spelling edit even in canonicalize-only mode.
                edits.replace(pair[0].span.end..pair[1].span.start, b" ");
            }
        }
    }
    let verdicts = if normalize_whitespace {
        operator_verdicts(line, &tokens, cx, context, &inside_paren)
    } else {
        Vec::new()
    };
    let mut spacing = OperatorSpacing::default();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Number => {
                if let Some(at) = exponent_marker(line, &tokens, index) {
                    let marker = match cx.config.style.keyword_case {
                        KeywordCase::Lower => line[at].to_ascii_lowercase(),
                        KeywordCase::Upper => line[at].to_ascii_uppercase(),
                        KeywordCase::Preserve => line[at],
                    };
                    edits.replace(at..at + 1, &[marker]);
                }
            }
            TokenKind::DotOp => {
                if cx.config.style.relational_symbols {
                    if let Some(operator) = modern_operator(token.text) {
                        add_operator_by_mode(
                            line,
                            &mut edits,
                            token,
                            operator,
                            normalize_whitespace,
                            &mut spacing,
                        );
                    } else if is_spaced_dotted_operator(token.text) {
                        let operator = dotted_case(token.text, cx.config.style.keyword_case);
                        add_operator_by_mode(
                            line,
                            &mut edits,
                            token,
                            &operator,
                            normalize_whitespace,
                            &mut spacing,
                        );
                    } else if token.text.eq_ignore_ascii_case(b".nil.") {
                        let cased = dotted_case(token.text, cx.config.style.keyword_case);
                        edits.replace(token.span.clone(), &cased);
                    } else if let Some(cased) =
                        dotted_word_case(token.text, cx.config.style.keyword_case)
                    {
                        edits.replace(token.span.clone(), &cased);
                    }
                } else if modern_operator(token.text).is_some()
                    || is_spaced_dotted_operator(token.text)
                {
                    let operator = dotted_case(token.text, cx.config.style.keyword_case);
                    add_operator_by_mode(
                        line,
                        &mut edits,
                        token,
                        &operator,
                        normalize_whitespace,
                        &mut spacing,
                    );
                } else if token.text.eq_ignore_ascii_case(b".nil.") {
                    let cased = dotted_case(token.text, cx.config.style.keyword_case);
                    edits.replace(token.span.clone(), &cased);
                } else if let Some(cased) =
                    dotted_word_case(token.text, cx.config.style.keyword_case)
                {
                    edits.replace(token.span.clone(), &cased);
                }
            }
            TokenKind::Operator if normalize_whitespace => {
                let next = tokens.get(index + 1).map(|token| token.text);
                match verdicts[index] {
                    OperatorVerdict::Untouched => {}
                    OperatorVerdict::Spaced => {
                        add_operator_edit(
                            line,
                            &mut edits,
                            token,
                            token.text,
                            true,
                            next,
                            &mut spacing,
                        );
                    }
                    OperatorVerdict::Compact { named } => {
                        add_operator_edit(
                            line,
                            &mut edits,
                            token,
                            token.text,
                            false,
                            next,
                            &mut spacing,
                        );
                        spacing.previous_compact_named = named;
                    }
                    OperatorVerdict::Tightened => {
                        remove_operator_trailing_whitespace(
                            line,
                            &mut edits,
                            token,
                            next,
                            &mut spacing,
                        );
                    }
                }
            }
            TokenKind::Name => {
                let component_selector = index > 0 && tokens[index - 1].text == b"%"
                    || continues_component_selector(&tokens, index, context);
                if component_selector {
                    if cx.project.macros.contains(token.text)
                        || !vocab::contains(vocab_2023::COMPLEX_PART_DESIGNATORS, token.text)
                        || tracked_component_spelling_governs(cx, token.text)
                    {
                        continue;
                    }
                    let cased = apply_case(token.text, cx.config.style.keyword_case);
                    if token.text != cased {
                        edits.replace(token.span.clone(), &cased);
                    }
                    continue;
                }
                if cx.project.macros.contains(token.text) {
                    continue;
                }
                if index == first_statement_index(&tokens)
                    && cx.config.style.split_compound_keywords
                    && tokens.get(index + 1).is_none_or(|next| next.text != b"=")
                    && !declared_names.suppresses_keyword(line_index, token.text, false)
                {
                    if let Some(canonical) =
                        vocab::lookup_pair(vocab_2023::COMPOUND_KEYWORDS, token.text)
                    {
                        let mut replacement = compound_spelling(token.text, canonical);
                        if cx.config.style.keyword_case != KeywordCase::Preserve {
                            replacement = case_compound_words(
                                &replacement,
                                cx.config.style.keyword_case,
                                declared_names,
                                line_index,
                            );
                        }
                        edits.replace(token.span.clone(), &replacement);
                        continue;
                    }
                }
                let cased = apply_case(token.text, cx.config.style.keyword_case);
                // The `C` in a BIND(C) language-binding-spec is syntax, not an
                // ordinary identifier. A local/project variable named C must
                // therefore not suppress its keyword-case normalization. Keep
                // the preceding BIND declaration check so an ordinary declared
                // procedure call `bind(C)` remains an identifier use.
                if is_bind_c_marker(&tokens, index)
                    && !declared_names.suppresses_keyword(line_index, b"bind", false)
                {
                    if token.text != cased {
                        edits.replace(token.span.clone(), &cased);
                    }
                    continue;
                }
                let specifier_argument =
                    is_specifier_keyword_argument(&tokens, index, &inside_paren, context);
                if (is_contextual_declaration_name(line, &tokens, index, continued_entity_list)
                    || is_old_style_declaration_entity(&tokens, index))
                    && !specifier_argument
                {
                    continue;
                }
                let end_construct_keyword = is_end_construct_keyword(&tokens, index);
                if (vocab::contains(vocab::FORTRAN_KEYWORDS, token.text)
                    || vocab::contains(vocab_2023::KEYWORDS, token.text))
                    && (end_construct_keyword
                        || !declared_names.suppresses_keyword(
                            line_index,
                            token.text,
                            specifier_argument,
                        ))
                    && keyword_in_context(&tokens, index, separator_below)
                {
                    if token.text != cased {
                        edits.replace(token.span.clone(), &cased);
                    }
                    continue;
                }
                if !end_construct_keyword
                    && declared_names.suppresses_keyword(line_index, token.text, specifier_argument)
                {
                    continue;
                }
                if (vocab::contains(vocab_2023::INTRINSIC_PROCEDURES, token.text)
                    && (is_followed_by_lparen(&tokens, index)
                        || is_call_procedure_designator(&tokens, index)))
                    || vocab::contains(vocab::INTRINSIC_NAMES, token.text)
                    || vocab::contains(vocab_2023::STANDARD_NAMES, token.text)
                    || vocab::contains(vocab::FORTRAN_SPECIFIERS, token.text)
                    || vocab::contains(vocab_2023::SPECIFIERS, token.text)
                {
                    if token.is(b"precision")
                        && !is_followed_by_lparen(&tokens, index)
                        && !previous_name_is(&tokens, index, b"double")
                    {
                        continue;
                    }
                    if token.text != cased {
                        edits.replace(token.span.clone(), &cased);
                    }
                    continue;
                }
                if cx.config.uppercase_single_l && token.is(b"l") {
                    edits.replace(token.span.clone(), b"L");
                }
            }
            _ => {}
        }
    }
    edits.finish()
}

/// What the operator rules will do to one `Operator` token.
///
/// Deciding this separately from emitting it is what lets a whole glued run of
/// operators be judged together — see [`operator_verdicts`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum OperatorVerdict {
    /// Not a spacing site: the token and its surroundings are left as written.
    Untouched,
    /// One space on each side.
    Spaced,
    /// Rewritten hard against its operands. `named` distinguishes a keyword
    /// argument's `=`, which the operator that follows it must know about.
    Compact { named: bool },
    /// Not binary here — a unary sign, or an I/O `*`. Only the whitespace that
    /// trails it is removed.
    Tightened,
}

/// Decide every operator token on the line before any of them is written.
///
/// The neighbour guards in `is_spaced_operator_token` read the bytes beside a
/// token to decide it is glued to another operator. Those bytes are exactly what
/// this pass rewrites, so a run of glued operators used to come apart one
/// operator per run of the formatter: `x=<1` spaced the `=`, which unglued the
/// `<` for the run after it to space, so the input took two runs to settle. The
/// comment path has never had this problem because `format_comment_operators`
/// consults the output it is building rather than its input.
///
/// So does this, one step earlier: a run of operators with no bytes between them
/// is judged as a unit, and if any member is being spaced the run is coming apart
/// on this pass, so the members that only declined because they were glued are
/// asked again with that fact in hand. Members that are compact for a reason of
/// their own keep it -- the `=` of a keyword argument, a unary sign -- which is
/// why `x =-1` stays `x =-1` rather than being pulled open by the `=` beside it.
///
/// A run whose members all decline is left glued, so `a<<b` still survives.
fn operator_verdicts(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    cx: &PassContext,
    context: &super::super::LineContext<'_>,
    inside_paren: &[bool],
) -> Vec<OperatorVerdict> {
    let mut verdicts: Vec<OperatorVerdict> = (0..tokens.len())
        .map(|index| operator_verdict(line, tokens, index, cx, context, inside_paren, false))
        .collect();
    let mut start = 0;
    while start < tokens.len() {
        if tokens[start].kind != TokenKind::Operator {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < tokens.len()
            && tokens[end].kind == TokenKind::Operator
            && tokens[end].span.start == tokens[end - 1].span.end
        {
            end += 1;
        }
        if end - start > 1 && verdicts[start..end].contains(&OperatorVerdict::Spaced) {
            for (offset, verdict) in verdicts[start..end].iter_mut().enumerate() {
                if *verdict == OperatorVerdict::Untouched {
                    *verdict = operator_verdict(
                        line,
                        tokens,
                        start + offset,
                        cx,
                        context,
                        inside_paren,
                        true,
                    );
                }
            }
        }
        start = end;
    }
    verdicts
}

/// One operator token's verdict, judged on its own.
///
/// `run_separates` is the caller's answer to "is this token's glued run coming
/// apart anyway"; it reaches only the neighbour guards.
fn operator_verdict(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    index: usize,
    cx: &PassContext,
    context: &super::super::LineContext<'_>,
    inside_paren: &[bool],
    run_separates: bool,
) -> OperatorVerdict {
    let token = &tokens[index];
    if token.kind != TokenKind::Operator
        || is_labelled_format_statement(tokens)
        || context.continued_format
    {
        return OperatorVerdict::Untouched;
    }
    if is_spaced_operator_token(line, token, run_separates) {
        let named =
            token.text == b"=" && is_keyword_argument_equals(tokens, index, inside_paren, context);
        return if named {
            OperatorVerdict::Compact { named }
        } else {
            OperatorVerdict::Spaced
        };
    }
    if !is_arithmetic_operator(token.text) || is_data_value_delimiter(tokens, token, context) {
        return OperatorVerdict::Untouched;
    }
    if is_io_specifier_star(tokens, index, token, context)
        || !(is_binary_arithmetic_operator(line, token.span.start, token.text)
            || context.continued_infix && is_leading_continuation_arithmetic(tokens, index, token))
    {
        return OperatorVerdict::Tightened;
    }
    if is_declaration_type_star(tokens, index, token.text)
        || !binary_operator_spaced(token.text, cx.config.style.compact_multiplicative)
    {
        return OperatorVerdict::Compact { named: false };
    }
    OperatorVerdict::Spaced
}

/// Canonicalize an operator token without taking ownership of its surrounding
/// whitespace unless the active style includes whitespace normalization.
fn add_operator_by_mode(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    replacement: &[u8],
    normalize_whitespace: bool,
    spacing: &mut OperatorSpacing,
) {
    if normalize_whitespace {
        // A spaced operator leaves a trailing space, so it can never glue the
        // token after it on: `next` has nothing to decide here.
        add_operator_edit(line, edits, token, replacement, true, None, spacing);
    } else {
        edits.replace(token.span.clone(), replacement);
    }
}

fn is_fortran_2023_multiword_pair(first: &[u8], second: &[u8]) -> bool {
    vocab_2023::MULTIWORD_KEYWORD_PAIRS
        .iter()
        .any(|(left, right)| {
            first.eq_ignore_ascii_case(left.as_bytes())
                && second.eq_ignore_ascii_case(right.as_bytes())
        })
}

/// A top-level slash in a `DATA` statement closes or opens a value list.
///
/// Whether a given slash looked like a division depended on what the wrapper
/// had left beside it on the physical line, so the same `DATA` statement was
/// spaced one way before a reflow and another after it (ABINIT `m_bessel.F90`).
/// The wrapper measures a rejoined statement with no line context, so the
/// statement's own head is consulted as well as the threaded flag.
fn is_data_value_delimiter(
    tokens: &[crate::source::Token<'_>],
    token: &crate::source::Token<'_>,
    context: &super::super::LineContext<'_>,
) -> bool {
    token.text == b"/" && token.depth == 0 && (context.data_statement || is_data_statement(tokens))
}

fn is_call_procedure_designator(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    let Some(call) = index.checked_sub(1) else {
        return false;
    };
    if !tokens[call].is_name(b"call") {
        return false;
    }
    call == first_statement_index(tokens)
        || if_condition_close(tokens).is_some_and(|close| call == close + 1)
}

/// The declared-case pass runs before this rule and has already put every resolvable
/// derived-type component into its governing spelling. Preserve that spelling rather than
/// interpreting a custom component named `RE` or `IM` as a complex-part designator.
///
/// If component declarations disagree project-wide, stay conservative and preserve the
/// authored spelling: there is no unique global spelling with which to distinguish a
/// tracked custom member from the standard designator at this token-local stage.
fn tracked_component_spelling_governs(cx: &PassContext<'_>, name: &[u8]) -> bool {
    for components in [&cx.local.cases.components, &cx.project.cases.components] {
        if !components.contains_name(name) {
            continue;
        }
        match components.unique_spelling(name) {
            Some(spelling) if spelling == name => return true,
            None => return true,
            Some(_) => {}
        }
    }
    false
}

fn is_bind_clause_head(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    if is_call_procedure_designator(tokens, index) {
        return false;
    }
    tokens
        .get(index + 1)
        .is_some_and(|token| token.kind == TokenKind::LParen)
        && tokens
            .get(index + 2)
            .is_some_and(|token| token.is_name(b"c"))
        && tokens.get(index + 3).is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::RParen | TokenKind::Comma | TokenKind::Ampersand
            )
        })
}

fn is_bind_c_marker(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index].is_name(b"c")
        && tokens[index - 1].kind == TokenKind::LParen
        && tokens[index - 2].is_name(b"bind")
        && is_bind_clause_head(tokens, index - 2)
}

fn keyword_in_context(
    tokens: &[crate::source::Token],
    index: usize,
    separator_below: bool,
) -> bool {
    let token = &tokens[index];
    let next = tokens.get(index + 1);
    if vocab::contains(vocab::DECLARATION_ATTRIBUTES, token.text) {
        // `optional` is an attribute only in a declaration's attribute half,
        // which the `::` closes — so a following `::` is what distinguishes the
        // attribute from a name that merely spells one. `separator_below` says
        // the statement's `::` is on a physical line below this one, which is
        // the same answer for a head or continuation line that does not reach
        // its own separator.
        return separator_below || tokens[index + 1..].iter().any(|t| t.text == b"::");
    }
    if token.is(b"only") {
        return next.is_some_and(|t| t.text == b":");
    }
    if token.is(b"bind") {
        return is_bind_clause_head(tokens, index);
    }
    if token.is(b"kind") {
        return next.is_some_and(|t| t.kind == TokenKind::LParen || t.text == b"=");
    }
    if token.is(b"precision") {
        return index > 0 && tokens[index - 1].is_name(b"double");
    }
    true
}

fn is_leading_continuation_arithmetic(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    token: &crate::source::Token<'_>,
) -> bool {
    matches!(token.text, b"+" | b"-" | b"*" | b"/" | b"**")
        && tokens[..index]
            .iter()
            .all(|previous| previous.kind == TokenKind::Ampersand)
}
