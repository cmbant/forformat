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
    let inside_paren = inside_paren_at(context.open_groups, &tokens);
    let continued_entity_list = context.continued_declaration
        && context.open_groups.is_empty()
        && !context.continued_initializer;
    let mut edits = EditBuffer::new(line);
    if !is_format_statement(&tokens) && !context.continued_format {
        for pair in tokens.windows(2) {
            if pair[0].kind == TokenKind::Name
                && pair[1].kind == TokenKind::Name
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
                && is_fortran_2023_multiword_pair(pair[0].text, pair[1].text)
            {
                edits.replace(pair[0].span.end..pair[1].span.start, b" ");
            }
        }
    }
    let mut spacing = OperatorSpacing::default();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Number => {
                if let Some(marker) = real_exponent_marker(token.text) {
                    let at = token.span.start + marker;
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
                        add_operator_edit(line, &mut edits, token, operator, true, &mut spacing);
                    } else if is_spaced_dotted_operator(token.text) {
                        let operator = dotted_case(token.text, cx.config.style.keyword_case);
                        add_operator_edit(line, &mut edits, token, &operator, true, &mut spacing);
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
                    add_operator_edit(line, &mut edits, token, &operator, true, &mut spacing);
                } else if token.text.eq_ignore_ascii_case(b".nil.") {
                    let cased = dotted_case(token.text, cx.config.style.keyword_case);
                    edits.replace(token.span.clone(), &cased);
                } else if let Some(cased) =
                    dotted_word_case(token.text, cx.config.style.keyword_case)
                {
                    edits.replace(token.span.clone(), &cased);
                }
            }
            TokenKind::Operator => {
                if !is_labelled_format_statement(&tokens)
                    && !context.continued_format
                    && is_spaced_operator_token(line, &tokens, index, token)
                {
                    let named = token.text == b"="
                        && (is_named_parameter_token(&tokens, index)
                            || context.continued_statement
                                && (!context.continued_declaration
                                    && context.continued_named_parameter
                                    || context.continued_bind_parameter)
                                && is_continued_named_parameter(
                                    &tokens,
                                    index,
                                    inside_paren[index],
                                ));
                    add_operator_edit(line, &mut edits, token, token.text, !named, &mut spacing);
                    spacing.previous_compact_named = named;
                } else if !is_labelled_format_statement(&tokens)
                    && !context.continued_format
                    && is_arithmetic_operator(token.text)
                {
                    if !is_io_specifier_star(&tokens, index, token)
                        && (is_binary_arithmetic_operator(line, token.span.start, token.text)
                            || context.continued_infix
                                && is_leading_continuation_arithmetic(&tokens, index, token))
                    {
                        let declaration_star = is_declaration_type_star(&tokens, index, token.text);
                        add_operator_edit(
                            line,
                            &mut edits,
                            token,
                            token.text,
                            !declaration_star
                                && binary_operator_spaced(
                                    token.text,
                                    cx.config.style.compact_multiplicative,
                                ),
                            &mut spacing,
                        );
                    } else {
                        remove_operator_trailing_whitespace(line, &mut edits, token, &mut spacing);
                    }
                }
            }
            TokenKind::Name => {
                let after_percent = index > 0 && tokens[index - 1].text == b"%";
                if after_percent {
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
                            replacement = apply_case(&replacement, cx.config.style.keyword_case);
                        }
                        edits.replace(token.span.clone(), &replacement);
                        continue;
                    }
                }
                let cased = apply_case(token.text, cx.config.style.keyword_case);
                let specifier_argument = is_specifier_keyword_argument(&tokens, index);
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
                    && keyword_in_context(&tokens, index)
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

fn is_fortran_2023_multiword_pair(first: &[u8], second: &[u8]) -> bool {
    vocab_2023::MULTIWORD_KEYWORD_PAIRS
        .iter()
        .any(|(left, right)| {
            first.eq_ignore_asci_case(left.as_bytes())
                && second.eq_ignore_asci_case(right.as_bytes())
        })
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

fn keyword_in_context(tokens: &[crate::source::Token], index: usize) -> bool {
    let token = &tokens[index];
    let next = tokens.get(index + 1);
    if vocab::contains(vocab::DECLARATION_ATTRIBUTES, token.text) {
        return tokens[index + 1..].iter().any(|t| t.text == b"::");
    }
    if token.is(b"only") {
        return next.is_some_and(|t| t.text == b":");
    }
    if token.is(b"bind") {
        return next.is_some_and(|t| t.kind == TokenKind::LParen)
            && tokens.get(index + 2).is_some_and(|t| t.is_name(b"c"))
            && tokens
                .get(index + 3)
                .is_some_and(|t| t.kind == TokenKind::RParen);
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
