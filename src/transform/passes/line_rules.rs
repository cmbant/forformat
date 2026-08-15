//! Step 11: the per-line rule chain.
//!
//! Normalization order, which must not be permuted:
//!
//! 1. `lowercase_line` — keyword case, operator modernization, real exponent
//!    markers, project case application;
//! 2. `normalize_keyword_spacing`  — compound keywords, `keyword(`, `) then`;
//! 3. `normalize_write_output_spacing`;
//! 4. `normalize_delimiter_spacing`;
//! 5. `normalize_comment_spacing`.
//!
//! The chain is exposed twice on purpose.  [`run`] applies it to the document,
//! and [`respace_joined`] applies rules 1, 2 and 4 to a statement the wrapper
//! has just rejoined — `rewrap_lines` (`:3875`) does exactly that, and without
//! it the spacing at a former continuation boundary is wrong.

use crate::{
    analysis::{scoped_declared_names, DeclaredNameIndex},
    error::FormatError,
    source::{
        regions::{LexState, RegionKind},
        tokens::{tokenize, TokenKind},
        PhysicalLineKind,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        pipeline::{Changed, PassContext},
        vocab,
    },
};

#[derive(Clone, Copy, Default)]
struct LineOptions<'a> {
    preserve_comment_after: bool,
    continued_statement: bool,
    continued_infix: bool,
    continued_declaration: bool,
    continued_named_parameter: bool,
    /// The groups still open when this line starts, innermost last: `true` for
    /// a parenthesis, `false` for a bracket.
    ///
    /// A named argument's `=` is only a named argument inside `(...)`, and a
    /// continuation line can both leave and enter groups — `…))], dim=1)`
    /// closes a bracket and is back inside the call.  Deciding that from the
    /// previous line alone gets one of the two cases wrong whichever way it is
    /// answered, so the decision is made at the `=`, from this stack folded
    /// forward over the line's own tokens.
    open_groups: &'a [bool],
    /// The statement this line continues is a FORMAT statement.  Its edit
    /// descriptors are not expressions, so `/)` there is a record separator
    /// before a closing parenthesis and not an array-constructor delimiter.
    continued_format: bool,
}

/// Apply the whole chain to every line of the document.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let mut changed = Changed::No;
    // The resolver view is built once for this analyzed document.  In
    // particular, do not ask the file-wide case tables once per line: a local
    // declaration must shadow an intrinsic only in its own procedure.
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let mut state = LexState::default();
    let mut continued_statement = false;
    let mut continued_infix = false;
    let mut continued_named_parameter = false;
    let mut open_groups: Vec<bool> = Vec::new();
    for index in 0..document.lines.len() {
        let kind = cx
            .analysis
            .buffer
            .lines
            .get(index)
            .map(|line| line.kind)
            .unwrap_or(PhysicalLineKind::Code);
        if let Some(body_start) = openmp_clause_body_start(&document.lines[index]) {
            // `!$ use ...` is a directive comment whose Fortran-like clause
            // text is normalized by the formatter. Exact `!$OMP` bodies use
            // their separate uppercase-directive rule and remain untouched
            // here.
            let body = apply_with_options(
                &document.lines[index][body_start..],
                cx,
                &declared_names,
                index,
                &mut LexState::default(),
                LineOptions::default(),
            );
            let mut rebuilt = document.lines[index][..body_start].to_vec();
            rebuilt.extend_from_slice(&body);
            if rebuilt != document.lines[index] {
                document.lines[index] = rebuilt;
                changed = changed.or(Changed::Text);
            }
            state = LexState::default();
            continued_statement = false;
            continued_infix = false;
            continued_named_parameter = false;
            open_groups.clear();
            continue;
        }
        if kind == PhysicalLineKind::Preprocessor {
            // A directive body is never Fortran; its spelling is preserved (I3)
            // and it does not carry literal state into the next line.
            state = LexState::default();
            continued_statement = false;
            continued_infix = false;
            continued_named_parameter = false;
            open_groups.clear();
            continue;
        }
        let preserve_comment_after = preserve_full_comment_spacing(document, index, cx);
        let first_statement_tokens = || {
            cx.analysis
                .group_of_line(index)
                .and_then(|group| group.statements.first())
                .map(|statement| crate::source::tokens::tokens(&statement.text))
        };
        let continued_declaration = continued_statement
            && first_statement_tokens().is_some_and(|tokens| is_declaration_statement(&tokens));
        let continued_format = continued_statement
            && first_statement_tokens().is_some_and(|tokens| is_format_statement(&tokens));
        let line = apply_with_options(
            &document.lines[index],
            cx,
            &declared_names,
            index,
            &mut state,
            LineOptions {
                preserve_comment_after,
                continued_statement,
                continued_infix,
                continued_declaration,
                continued_named_parameter,
                continued_format,
                open_groups: &open_groups,
            },
        );
        if let Some(physical) = cx.analysis.buffer.lines.get(index) {
            if matches!(
                physical.kind,
                PhysicalLineKind::Code | PhysicalLineKind::FindentFix
            ) {
                let code = cx.analysis.buffer.code_bytes(physical);
                continued_statement = trailing_ampersand(code);
                continued_infix = trailing_continuation_operand(code);
                // Whether the *statement* has argument lists at all.  Whether a
                // given `=` is inside one is decided at the `=`, from
                // `open_groups`.
                continued_named_parameter = continued_statement && is_call_group(cx, index);
                fold_open_groups(code, &mut open_groups);
                if !continued_statement {
                    open_groups.clear();
                }
            }
        }
        if line != document.lines[index] {
            document.lines[index] = line;
            changed = changed.or(Changed::Text);
        }
    }
    Ok(changed)
}

fn is_call_group(cx: &PassContext, line_index: usize) -> bool {
    let Some(statement) = cx
        .analysis
        .group_of_line(line_index)
        .and_then(|group| group.statements.first())
    else {
        return false;
    };
    let tokens = crate::source::tokens::tokens(&statement.text);
    tokens
        .iter()
        .find(|token| token.kind == TokenKind::Name)
        .is_some_and(|token| token.is_name(b"call"))
        || tokens
            .iter()
            .enumerate()
            .any(|(index, token)| token.text == b"=" && is_named_parameter_token(&tokens, index))
}

/// The full chain for one physical line, carrying literal state across
/// continuations.
pub fn apply(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    state: &mut LexState,
) -> Vec<u8> {
    apply_with_options(
        line,
        cx,
        declared_names,
        line_index,
        state,
        LineOptions::default(),
    )
}

fn apply_with_options(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    state: &mut LexState,
    options: LineOptions,
) -> Vec<u8> {
    let incoming = *state;
    let mut text = lowercase_line_with_context(
        line,
        cx,
        declared_names,
        line_index,
        state,
        options.continued_statement,
        options.continued_infix,
        options.continued_declaration,
        options.continued_named_parameter,
        options.open_groups,
        false,
    );
    text = normalize_keyword_spacing_with_state(
        &text,
        declared_names,
        line_index,
        incoming,
        options.continued_format,
    );
    text = normalize_write_output_spacing_with_state(&text, cx, incoming);
    text = normalize_delimiter_spacing_with_state(&text, cx, incoming);
    let mut text = normalize_comment_spacing_with_state(
        &text,
        cx,
        incoming,
        options.preserve_comment_after,
        code_span_len(&text) as isize - code_span_len(line) as isize,
    );
    if options.continued_statement && options.continued_named_parameter {
        text = compact_continued_named_argument(&text, options.open_groups);
    }
    text
}

/// Rules 1, 2 and 4 for a statement the wrapper has just joined.
///
/// This exists because joining two physical lines creates spacing the per-line
/// pass never saw: `if ( .not. (` only becomes `if (.not. (` once the keyword
/// rule runs after the `.not.` padding that rule 1 adds.
pub fn respace_joined(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
) -> Vec<u8> {
    let mut state = LexState::default();
    let mut text = lowercase_line_with_context(
        line,
        cx,
        declared_names,
        line_index,
        &mut state,
        false,
        false,
        false,
        false,
        &[],
        cx.project.target_local_component_resolution,
    );
    text = normalize_keyword_spacing_with_state(
        &text,
        declared_names,
        line_index,
        LexState::default(),
        false,
    );
    text = normalize_delimiter_spacing(&text, cx);
    compact_joined_named_arguments(&text)
}

/// Joining physical continuation lines can turn a named argument into a token
/// sequence whose original per-line context was unavailable. Keep the
/// `name=value` spelling that the ordinary line pass uses for argument
/// specifiers, without compacting top-level assignments.
fn compact_joined_named_arguments(line: &[u8]) -> Vec<u8> {
    let tokens = tokenize(line, &mut LexState::default());
    let mut edits = EditBuffer::new(line);
    for (index, token) in tokens.iter().enumerate() {
        if token.text != b"=" || !is_named_parameter_token(&tokens, index) {
            continue;
        }
        let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
            continue;
        };
        let Some(next) = tokens.get(index + 1) else {
            continue;
        };
        edits.replace(previous.span.end..next.span.start, b"=");
    }
    edits.finish()
}

fn compact_continued_named_argument(line: &[u8], open_groups: &[bool]) -> Vec<u8> {
    let tokens = tokenize(line, &mut LexState::default());
    let inside_paren = inside_paren_at(open_groups, &tokens);
    let mut edits = EditBuffer::new(line);
    for (index, token) in tokens.iter().enumerate() {
        if token.text != b"=" || !is_continued_named_parameter(&tokens, index, inside_paren[index])
        {
            continue;
        }
        let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
            continue;
        };
        let Some(next) = tokens.get(index + 1) else {
            continue;
        };
        edits.replace(previous.span.end..next.span.start, b"=");
    }
    edits.finish()
}

/// Rule 1: keyword case, and the case decisions the project agreed on.
///
/// Implemented so far: **keyword lowercasing**.  A word is lowercased only when
/// it is a Fortran keyword *and* nothing in the file or project declares an
/// identifier by that name (I4) *and* it is not a macro name and not a
/// component after `%`.  A variable called `data`, `type` or `precision` is a
/// real thing and keeps its spelling.
///
/// Every replacement is made against a token span; the source is never
/// reconstructed from a lossy token spelling.
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
        false,
        false,
        false,
        false,
        &[],
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn lowercase_line_with_context(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    state: &mut LexState,
    continued_statement: bool,
    continued_infix: bool,
    continued_declaration: bool,
    continued_named_parameter: bool,
    open_groups: &[bool],
    preserve_identifier_case: bool,
) -> Vec<u8> {
    let tokens = tokenize(line, state);
    let inside_paren = inside_paren_at(open_groups, &tokens);
    // A declaration continued at the statement's top level is still inside its
    // entity list; inside a group it is inside an expression, where a keyword
    // is a keyword.
    let continued_entity_list = continued_declaration && open_groups.is_empty();
    let mut edits = EditBuffer::new(line);
    let mut spacing = OperatorSpacing::default();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Number => {
                if let Some(marker) = real_exponent_marker(token.text) {
                    let at = token.span.start + marker;
                    edits.replace(at..at + 1, &[line[at].to_ascii_lowercase()]);
                }
            }
            TokenKind::DotOp => {
                if let Some(operator) = modern_operator(token.text) {
                    add_operator_edit(line, &mut edits, token, operator, true, &mut spacing);
                } else if is_spaced_dotted_operator(token.text) {
                    let lowered = token.text.to_ascii_lowercase();
                    add_operator_edit(line, &mut edits, token, &lowered, true, &mut spacing);
                } else if let Some(lowered) = dotted_word_lowering(token.text) {
                    edits.replace(token.span.clone(), &lowered);
                }
            }
            TokenKind::Operator => {
                if is_spaced_operator_token(line, &tokens, index, token) {
                    let named = token.text == b"="
                        && (is_named_parameter_token(&tokens, index)
                            || continued_statement
                                && !continued_declaration
                                && continued_named_parameter
                                && is_continued_named_parameter(
                                    &tokens,
                                    index,
                                    inside_paren[index],
                                ));
                    add_operator_edit(line, &mut edits, token, token.text, !named, &mut spacing);
                    spacing.previous_compact_named = named;
                } else if is_arithmetic_operator(token.text) {
                    if is_binary_arithmetic_operator(line, token.span.start, token.text)
                        || (continued_infix
                            && is_leading_continuation_arithmetic(&tokens, index, token))
                    {
                        add_operator_edit(
                            line,
                            &mut edits,
                            token,
                            token.text,
                            !vocab::contains(vocab::COMPACT_ARITHMETIC_OPERATORS, token.text),
                            &mut spacing,
                        );
                    } else {
                        remove_operator_trailing_whitespace(line, &mut edits, token, &mut spacing);
                    }
                }
            }
            TokenKind::Name => {
                if preserve_identifier_case && index > 0 && tokens[index - 1].text == b"%" {
                    continue;
                }
                // `a%Data` names a component, not the DATA statement.
                if index > 0 && tokens[index - 1].text == b"%" {
                    continue;
                }
                if cx.project.macros.contains(token.text) {
                    continue;
                }

                let lower = token.text.to_ascii_lowercase();
                let specifier_argument = is_specifier_keyword_argument(&tokens, index);
                if is_contextual_declaration_name(line, &tokens, index, continued_entity_list)
                    && !specifier_argument
                {
                    continue;
                }
                if vocab::contains(vocab::FORTRAN_KEYWORDS, token.text)
                    && !declared_names.suppresses_keyword(
                        line_index,
                        token.text,
                        specifier_argument,
                    )
                    && keyword_in_context(&tokens, index)
                {
                    if token.text != lower {
                        edits.replace(token.span.clone(), &lower);
                    }
                    continue;
                }

                if declared_names.suppresses_keyword(line_index, token.text, specifier_argument) {
                    continue;
                }
                if vocab::contains(vocab::INTRINSIC_NAMES, token.text)
                    || vocab::contains(vocab::FORTRAN_SPECIFIERS, token.text)
                {
                    // PRECISION is both an intrinsic and the second word of
                    // DOUBLE PRECISION. A bare
                    // variable named precision is not inferred to be a word
                    // of the language.
                    if token.is(b"precision")
                        && !is_followed_by_lparen(&tokens, index)
                        && !previous_name_is(&tokens, index, b"double")
                    {
                        continue;
                    }
                    if token.text != lower {
                        edits.replace(token.span.clone(), &lower);
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

/// Words that are keywords only in a particular shape. Outside that shape they
/// are ordinary identifiers and must keep their spelling: `BIND(C, name=...)` is not the
/// `bind(c)` language binding, and a `precision` that no `double` precedes is
/// somebody's variable.
fn keyword_in_context(tokens: &[crate::source::Token], index: usize) -> bool {
    let token = &tokens[index];
    let next = tokens.get(index + 1);
    if vocab::contains(vocab::DECLARATION_ATTRIBUTES, token.text) {
        // An attribute is only an attribute in a declaration, which is the
        // statement shape that carries `::`.
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

/// Rule 2: keyword and layout spacing.
///
/// The spacing rule handles `COMMON /blk/`, `(/ .. /)` to `[ .. ]` outside
/// `FORMAT`, `go to` to
/// `goto`, multiword keyword pairs, compound keywords ([`vocab::COMPOUND_KEYWORDS`]),
/// `end x`, `do while (`, `dimension(`, `if (`, `type(`, `select type (`,
/// [`vocab::PARENTHESIZED_STATEMENT_NAMES`], empty `subroutine s()`, `only:`,
/// bracket-adjacent whitespace, `) then`, and the arithmetic/one-line `IF` body
/// separator.
pub fn normalize_keyword_spacing(
    line: &[u8],
    declared_names: &DeclaredNameIndex,
    line_index: usize,
) -> Vec<u8> {
    normalize_keyword_spacing_with_state(
        line,
        declared_names,
        line_index,
        LexState::default(),
        false,
    )
}

fn normalize_keyword_spacing_with_state(
    line: &[u8],
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    incoming: LexState,
    continued_format: bool,
) -> Vec<u8> {
    let tokens = tokenize(line, &mut incoming.clone());
    let mut edits = EditBuffer::new(line);

    // These are deliberately ordered like `_normalize_keyword_spacing_code`:
    // the broad, statement-level rewrites come before the token-local spacing
    // rules.  The edit buffer drops an overlapping narrow edit, which is the
    // safe outcome for an array-constructor delimiter that also looks like a
    // bracket-adjacent space.
    if let Some((start, end, replacement)) = common_block_edit(line, &tokens) {
        edits.replace(start..end, &replacement);
    }
    // A continuation line of a FORMAT statement carries no `format` keyword of
    // its own, so the statement-level flag is the only thing standing between
    // an edit descriptor like `i5 /)` and a rewrite into `i5]`.
    if !is_format_statement(&tokens) && !continued_format {
        for pair in tokens.windows(2) {
            if pair[0].kind == TokenKind::LParen
                && pair[1].kind == TokenKind::Operator
                && pair[1].text == b"/"
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            {
                let mut end = pair[1].span.end;
                while end < line.len() && matches!(line[end], b' ' | b'\t') {
                    end += 1;
                }
                edits.replace(pair[0].span.start..end, b"[");
            }
        }
        for pair in tokens.windows(2) {
            if pair[0].kind == TokenKind::Operator
                && pair[0].text == b"/"
                && pair[1].kind == TokenKind::RParen
                && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            {
                let mut start = pair[0].span.start;
                while start > 0 && matches!(line[start - 1], b' ' | b'\t') {
                    start -= 1;
                }
                edits.replace(start..pair[1].span.end, b"]");
            }
        }
    }

    // `go to` is a word pair rather than a token pair in the generated
    // vocabulary, because its canonical spelling is shorter.
    for pair in tokens.windows(2) {
        if pair[0].is_name(b"go")
            && pair[1].is_name(b"to")
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
        {
            edits.replace(pair[0].span.start..pair[1].span.end, b"goto");
        }
    }

    for pair in tokens.windows(2) {
        if pair[0].kind == TokenKind::Name
            && pair[1].kind == TokenKind::Name
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            && is_multiword_keyword_pair(pair[0].text, pair[1].text)
        {
            let first = pair[0].text.to_ascii_lowercase();
            let second = pair[1].text.to_ascii_lowercase();
            let mut replacement = first;
            replacement.push(b' ');
            replacement.extend_from_slice(&second);
            edits.replace(pair[0].span.start..pair[1].span.end, &replacement);
        }
    }

    // Compound keywords are recognized only at the beginning of the physical
    // statement.  In particular, `endif = 1` is an identifier assignment,
    // not an END IF statement.
    if let Some(first) = first_statement_token(&tokens) {
        if let Some(replacement) = vocab::lookup_pair(vocab::COMPOUND_KEYWORDS, first.text) {
            let next = tokens.get(first_statement_index(&tokens) + 1);
            let assignment = next.is_some_and(|token| token.text == b"=");
            if !assignment && !declared_names.suppresses_keyword(line_index, first.text, false) {
                edits.replace(first.span.clone(), replacement.as_bytes());
                // The compound replacement itself changes `elseif(` into
                // `else if(`. The token-local `name(` rule inspected the
                // original `elseif(` token, so it never saw the new `if`.
                if first.is_name(b"elseif") {
                    if let Some(paren) = tokens.get(first_statement_index(&tokens) + 1) {
                        if paren.kind == TokenKind::LParen {
                            edits.replace(first.span.end..paren.span.start, b" ");
                        }
                    }
                }
            }
        }
    }

    for (index, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Name {
            if token.is(b"end") && !declared_names.suppresses_keyword(line_index, token.text, false)
            {
                if let Some(next) = tokens.get(index + 1) {
                    if next.kind == TokenKind::Name
                        && horizontal_gap(line, token.span.end, next.span.start)
                    {
                        edits.replace(token.span.end..next.span.start, b" ");
                        // `end subroutine   name` closes up too, but only when
                        // a *name* follows: `horizontal_gap` is true of an
                        // empty gap, so accepting any token turned this into an
                        // insertion.  Both spellings of the mistake showed up
                        // as non-fixed points on the first `end` a compound
                        // rewrite had just produced — `endif  !! c` became
                        // `end if !! c`, stepping on rule 5's preserved `!!`
                        // spacing, and `enddo; enddo` became `end do ; enddo`.
                        if let Some(after) = tokens.get(index + 2) {
                            if after.kind == TokenKind::Name
                                && horizontal_gap(line, next.span.end, after.span.start)
                            {
                                edits.replace(next.span.end..after.span.start, b" ");
                            }
                        }
                    }
                }
            }
            if token.is(b"do") && !declared_names.suppresses_keyword(line_index, token.text, false)
            {
                if let Some(next) = tokens.get(index + 1) {
                    if next.kind == TokenKind::Name
                        && horizontal_gap(line, token.span.end, next.span.start)
                    {
                        edits.replace(token.span.end..next.span.start, b" ");
                        if next.is_name(b"while") {
                            if let Some(paren) = tokens.get(index + 2) {
                                if paren.kind == TokenKind::LParen
                                    && horizontal_gap(line, next.span.end, paren.span.start)
                                {
                                    edits.replace(next.span.end..paren.span.start, b" ");
                                }
                            }
                        }
                    }
                }
            }
            // `ONLY :` becomes `only:`. Lowercasing happens here, in the
            // spacing rule, and not in `lowercase_keyword` — which preserves it,
            // because `USE, INTRINSIC :: m, ONLY: x` puts the word after a `::`
            // and so inside the declaration-name guard.  Doing the case change
            // in the same place is what makes that statement come out right.
            if token.is(b"only")
                && !declared_names.suppresses_keyword(line_index, token.text, false)
                && tokens.get(index + 1).is_some_and(|next| next.text == b":")
            {
                let colon = tokens[index + 1].span.start;
                if token.text != b"only" || horizontal_gap(line, token.span.end, colon) {
                    edits.replace(token.span.start..colon, b"only");
                }
            }
            // A keyword that introduces a following name is collapsed to one
            // space: `module   mymod`, `use   mymod`, `call   foo`,
            // `subroutine  do_thing`. Unlike `end`/`do` above this is not
            // conditioned on position, so `end module   mymod` is closed up
            // by this rule rather than needing its own case.
            if (token.is(b"module")
                || token.is(b"use")
                || token.is(b"call")
                || token.is(b"subroutine"))
                && !declared_names.suppresses_keyword(line_index, token.text, false)
            {
                if let Some(next) = tokens.get(index + 1) {
                    if next.kind == TokenKind::Name
                        && horizontal_gap(line, token.span.end, next.span.start)
                    {
                        edits.replace(token.span.end..next.span.start, b" ");
                    }
                }
            }
        }

        if token.kind == TokenKind::Name && is_followed_by_lparen(&tokens, index) {
            let next = &tokens[index + 1];
            if !horizontal_gap(line, token.span.end, next.span.start) {
                continue;
            }
            let declared = declared_names.suppresses_keyword(line_index, token.text, false);
            let selected_type = index > 0 && tokens[index - 1].is_name(b"select");
            let no_space = vocab::contains(vocab::PARENTHESIZED_STATEMENT_NAMES, token.text)
                || token.is(b"dimension")
                || token.is(b"associate")
                || token.is(b"result")
                || (token.is(b"type") && !selected_type)
                || (token.is(b"class") && !selected_type);
            let one_space = token.is(b"if") || token.is(b"select");
            if !declared && (no_space || one_space) {
                edits.replace(
                    token.span.end..next.span.start,
                    if no_space { b"" } else { b" " },
                );
            }
        }

        // SELECT TYPE (x) and SELECT TYPE IS (x) are the one keyword family
        // where the space belongs before the opening parenthesis.
        if token.is(b"select") {
            if let (Some(ty), Some(paren)) = (tokens.get(index + 1), tokens.get(index + 2)) {
                if ty.is_name(b"type") && paren.kind == TokenKind::LParen {
                    if horizontal_gap(line, token.span.end, ty.span.start) {
                        edits.replace(token.span.end..ty.span.start, b" ");
                    }
                    if horizontal_gap(line, ty.span.end, paren.span.start) {
                        edits.replace(ty.span.end..paren.span.start, b" ");
                    }
                }
            }
            if let (Some(ty), Some(is), Some(paren)) = (
                tokens.get(index + 1),
                tokens.get(index + 2),
                tokens.get(index + 3),
            ) {
                if ty.is_name(b"type") && is.is_name(b"is") && paren.kind == TokenKind::LParen {
                    if horizontal_gap(line, token.span.end, ty.span.start) {
                        edits.replace(token.span.end..ty.span.start, b" ");
                    }
                    if horizontal_gap(line, ty.span.end, is.span.start) {
                        edits.replace(ty.span.end..is.span.start, b" ");
                    }
                    if horizontal_gap(line, is.span.end, paren.span.start) {
                        edits.replace(is.span.end..paren.span.start, b" ");
                    }
                }
            }
        }

        if token.is(b"change") || token.is(b"form") || token.is(b"select") || token.is(b"sync") {
            if let (Some(rank_or_team), Some(paren)) =
                (tokens.get(index + 1), tokens.get(index + 2))
            {
                if (rank_or_team.is_name(b"rank") || rank_or_team.is_name(b"team"))
                    && paren.kind == TokenKind::LParen
                    && horizontal_gap(line, rank_or_team.span.end, paren.span.start)
                {
                    // The replacement range is the gap before the existing
                    // parenthesis; inserting another `(` here made full mode
                    // grow `change team (x)` on every pass.
                    edits.replace(rank_or_team.span.end..paren.span.start, b" ");
                }
            }
        }
    }

    // Bracket-adjacent whitespace and `) then` are lexical rules, not
    // declaration rules, so they apply to every unquoted code token.
    for pair in tokens.windows(2) {
        if (pair[0].kind == TokenKind::LParen || pair[0].kind == TokenKind::LBracket)
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
            && !is_trailing_continuation_marker(line, pair[1].span.start)
        {
            edits.replace(pair[0].span.end..pair[1].span.start, b"");
        }
        if (pair[1].kind == TokenKind::RParen || pair[1].kind == TokenKind::RBracket)
            && !matches!(pair[0].kind, TokenKind::String | TokenKind::Hollerith)
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
        {
            edits.replace(pair[0].span.end..pair[1].span.start, b"");
        }
        if pair[0].kind == TokenKind::RParen
            && pair[1].is_name(b"then")
            && horizontal_gap(line, pair[0].span.end, pair[1].span.start)
        {
            edits.replace(pair[0].span.end..pair[1].span.start, b" ");
        }
    }

    // The suffix of an arithmetic IF or a one-line IF must not run into the
    // closing condition parenthesis.
    if let Some(close) = if_condition_close(&tokens) {
        if let Some(next) = tokens.get(close + 1) {
            if next.kind != TokenKind::Comment
                && next.text != b"&"
                && !next.is_name(b"then")
                && line[next.span.start..]
                    .iter()
                    .any(|byte| !byte.is_ascii_whitespace())
            {
                edits.replace(tokens[close].span.end..next.span.start, b" ");
            }
        }
    }

    // Empty SUBROUTINE argument lists are the one shortening rule here.  It
    // is anchored at the line's declaration header and therefore cannot
    // remove a call's empty argument list.
    for (index, subroutine) in tokens.iter().enumerate() {
        if !subroutine.is_name(b"subroutine") || index > 0 && tokens[index - 1].is_name(b"end") {
            continue;
        }
        if let (Some(name), Some(open), Some(close)) = (
            tokens.get(index + 1),
            tokens.get(index + 2),
            tokens.get(index + 3),
        ) {
            if name.kind == TokenKind::Name
                && open.kind == TokenKind::LParen
                && close.kind == TokenKind::RParen
            {
                edits.replace(open.span.start..close.span.end, b"");
            }
        }
    }

    let mut output = edits.finish();
    let start = output
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(output.len());
    if output
        .get(start..)
        .is_some_and(|tail| tail.starts_with(b"else if("))
    {
        output.insert(start + b"else if".len(), b' ');
    }
    output
}

fn trailing_ampersand(line: &[u8]) -> bool {
    let mut end = line.len();
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end > 0 && line[end - 1] == b'&'
}

fn trailing_continuation_operand(line: &[u8]) -> bool {
    let mut end = line.len();
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end == 0 || line[end - 1] != b'&' {
        return false;
    }
    let mut previous = end - 1;
    while previous > 0 && line[previous - 1].is_ascii_whitespace() {
        previous -= 1;
    }
    if previous == 0 {
        return false;
    }
    if matches!(
        line[previous - 1],
        b')' | b']' | b'_' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'
    ) {
        return true;
    }
    // A trailing `.` is either a decimal point — `x = 1. &` ends on an operand
    // — or the closing dot of a dotted operator.  `… .or. &` ends on an
    // *operator*, so the next line starts a fresh operand and its leading `-`
    // is unary.  Sniffing the byte alone spaced that minus out, but only on the
    // run after the wrapper had put it there.
    line[previous - 1] == b'.' && !ends_with_dotted_operator(&line[..previous])
}

/// Whether the code ends with a dotted operator such as `.or.`, as opposed to a
/// decimal point or one of the dotted logical *constants*, which are operands.
fn ends_with_dotted_operator(code: &[u8]) -> bool {
    let Some(open) = code[..code.len() - 1]
        .iter()
        .rposition(|byte| !byte.is_ascii_alphabetic())
    else {
        return false;
    };
    if code[open] != b'.' || open + 1 == code.len() - 1 {
        return false;
    }
    let word = &code[open..];
    !word.eq_ignore_ascii_case(b".true.") && !word.eq_ignore_ascii_case(b".false.")
}

fn is_leading_continuation_arithmetic(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    token: &crate::source::Token<'_>,
) -> bool {
    matches!(token.text, b"+" | b"-" | b"*" | b"/")
        && tokens[..index]
            .iter()
            .all(|previous| previous.kind == TokenKind::Ampersand)
}

/// Rule 3: `WRITE(...)item` spacing.
///
/// Port target: `normalize_write_output_spacing`.
pub fn normalize_write_output_spacing(line: &[u8], cx: &PassContext) -> Vec<u8> {
    normalize_write_output_spacing_with_state(line, cx, LexState::default())
}

fn normalize_write_output_spacing_with_state(
    line: &[u8],
    cx: &PassContext,
    incoming: LexState,
) -> Vec<u8> {
    let _ = cx;
    let tokens = tokenize(line, &mut incoming.clone());
    let mut edits = EditBuffer::new(line);
    for (index, token) in tokens.iter().enumerate() {
        if !token.is_name(b"write") || !is_followed_by_lparen(&tokens, index) {
            continue;
        }
        let open = index + 1;
        let Some(close) = matching_close(&tokens, open) else {
            continue;
        };
        let end = tokens[close].span.end;
        if end < line.len()
            && !line[end].is_ascii_whitespace()
            && !matches!(line[end], b'&' | b'!' | b';' | b'\n')
        {
            edits.insert(end, b" ");
        }
    }
    edits.finish()
}

/// Rule 4: delimiter and comma spacing.
///
/// Port target: `normalize_delimiter_spacing` — one space after a comma, none
/// before it, none inside brackets, and the compact behaviour of `*`, `/`,
/// `**`, `//` ([`vocab::COMPACT_ARITHMETIC_OPERATORS`]).
pub fn normalize_delimiter_spacing(line: &[u8], cx: &PassContext) -> Vec<u8> {
    normalize_delimiter_spacing_with_state(line, cx, LexState::default())
}

fn normalize_delimiter_spacing_with_state(
    line: &[u8],
    _cx: &PassContext,
    incoming: LexState,
) -> Vec<u8> {
    let mut text = line.to_vec();
    let tokens = tokenize(&text, &mut incoming.clone());
    if is_declaration_statement(&tokens) {
        if let Some(separator) = top_level_separator(&tokens) {
            text = reorder_optional_attribute(&text, tokens[separator].span.start, incoming);
        } else {
            text = normalize_old_style_declaration(&text, incoming);
        }
    }

    let mut state = incoming;
    let regions = state.regions(&text);
    let mut result = Vec::with_capacity(text.len());
    for (index, region) in regions.iter().enumerate() {
        if region.kind == RegionKind::Code {
            let following_content = regions
                .get(index + 1)
                .is_some_and(|next| next.kind != RegionKind::Comment);
            normalize_delimiters_in_code(
                &text[region.range.clone()],
                &mut result,
                following_content,
            );
        } else {
            result.extend_from_slice(&text[region.range.clone()]);
        }
    }
    result
}

/// Rule 5: comment marker spacing and commented-out assignments.
///
/// Port target: `normalize_comment_spacing` plus `format_comment_operators`,
/// which is the one transform allowed to touch comment text (I3).
pub fn normalize_comment_spacing(line: &[u8], cx: &PassContext) -> Vec<u8> {
    normalize_comment_spacing_with_state(line, cx, LexState::default(), false, 0)
}

/// The width of the code on a line, ignoring indentation and any comment.
fn code_span_len(line: &[u8]) -> usize {
    let end = crate::source::regions::comment_start(line).unwrap_or(line.len());
    line[..end].trim_ascii().len()
}

fn normalize_comment_spacing_with_state(
    line: &[u8],
    cx: &PassContext,
    incoming: LexState,
    preserve_after: bool,
    code_growth: isize,
) -> Vec<u8> {
    let _ = cx;
    let mut state = incoming;
    let mut comment_start = None;
    state.scan(line, |region| {
        if comment_start.is_none() && region.kind == RegionKind::Comment {
            comment_start = Some(region.range.start);
        }
    });
    let Some(start) = comment_start else {
        return line.to_vec();
    };
    let original_comment = &line[start..];
    if original_comment.starts_with(b"!!")
        || is_directive_comment(original_comment)
        || original_comment[1..].iter().all(u8::is_ascii_whitespace)
    {
        return line.to_vec();
    }

    let mut comment = original_comment.to_vec();
    if is_commented_assignment(&comment) {
        comment = format_comment_operators(&comment);
    }
    let before = &line[..start];
    let leading = before.iter().position(|byte| !matches!(byte, b' ' | b'\t'));
    let leading_end = leading.unwrap_or(before.len());
    let mut code = before[leading_end..].to_vec();
    while code.last().is_some_and(u8::is_ascii_whitespace) {
        code.pop();
    }
    let mut out = Vec::with_capacity(line.len() + 2);
    out.extend_from_slice(&before[..leading_end]);
    if !code.is_empty() {
        out.extend_from_slice(&code);
        // Keep the comment where the author put it rather than collapsing the
        // gap to one space: a hand-aligned column of trailing comments is
        // information this pass cannot judge, because it cannot see the
        // neighbouring lines.  Step 17b makes the block-wide decision once
        // layout has settled.
        //
        // The gap is corrected by however much the earlier rules widened this
        // line's code, so what survives is the authored *column*, not the
        // authored gap.  Without that, adding one space inside `i,j, j_ss`
        // moves that comment one column right and a block its author had
        // aligned no longer looks aligned to the pass that must decide.
        // The correction only ever narrows: a line must not leave this pass
        // wider than it arrived, or the wrapper — which measures here, before
        // step 17b compresses anything — would size it against padding that is
        // about to go away.  When the code shrank instead, holding the column
        // would mean widening, so the column moves and the block falls back to
        // a single space, which is the behaviour it had anyway.
        let gap = before.len() - leading_end - code.len();
        let corrected = (gap as isize - code_growth).max(1) as usize;
        out.resize(out.len() + corrected.min(gap.max(1)), b' ');
    }
    out.push(b'!');
    if preserve_after
        && comment
            .get(1)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        out.extend_from_slice(&comment[1..]);
        return out;
    }
    out.push(b' ');
    let mut after = &comment[1..];
    while after
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        after = &after[1..];
    }
    out.extend_from_slice(after);
    out
}

fn preserve_full_comment_spacing(document: &Document, index: usize, cx: &PassContext) -> bool {
    if cx
        .analysis
        .buffer
        .lines
        .get(index)
        .is_none_or(|line| line.kind != PhysicalLineKind::Comment)
    {
        return false;
    }
    let is_full_comment = |line: &[u8]| {
        let start = line
            .iter()
            .position(|byte| !matches!(byte, b' ' | b'\t'))
            .unwrap_or(line.len());
        let comment = &line[start..];
        comment.first() == Some(&b'!')
            && !comment.starts_with(b"!!")
            && !is_directive_comment(comment)
    };
    let current = document
        .lines
        .get(index)
        .is_some_and(|line| is_full_comment(line));
    if !current {
        return false;
    }
    let previous = index
        .checked_sub(1)
        .and_then(|line| document.lines.get(line))
        .is_some_and(|line| is_full_comment(line));
    let next = document
        .lines
        .get(index + 1)
        .is_some_and(|line| is_full_comment(line));
    previous || next
}

fn horizontal_gap(line: &[u8], start: usize, end: usize) -> bool {
    start <= end
        && line[start..end]
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t'))
}

fn is_multiword_keyword_pair(first: &[u8], second: &[u8]) -> bool {
    vocab::MULTIWORD_KEYWORD_PAIRS.iter().any(|(left, right)| {
        first.eq_ignore_ascii_case(left.as_bytes()) && second.eq_ignore_ascii_case(right.as_bytes())
    })
}

fn first_statement_index(tokens: &[crate::source::Token<'_>]) -> usize {
    usize::from(
        tokens
            .first()
            .is_some_and(|token| token.kind == TokenKind::Number),
    )
}

fn first_statement_token<'a>(
    tokens: &'a [crate::source::Token<'a>],
) -> Option<&'a crate::source::Token<'a>> {
    tokens
        .get(first_statement_index(tokens))
        .filter(|token| token.kind == TokenKind::Name)
}

fn is_followed_by_lparen(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.kind == TokenKind::LParen)
}

fn previous_name_is(tokens: &[crate::source::Token<'_>], index: usize, name: &[u8]) -> bool {
    index > 0 && tokens[index - 1].is_name(name)
}

fn matching_close(tokens: &[crate::source::Token<'_>], open: usize) -> Option<usize> {
    let opening = tokens.get(open)?;
    let close_kind = match opening.kind {
        TokenKind::LParen => TokenKind::RParen,
        TokenKind::LBracket => TokenKind::RBracket,
        _ => return None,
    };
    tokens
        .iter()
        .enumerate()
        .skip(open + 1)
        .find(|(_, token)| token.kind == close_kind && token.depth == opening.depth)
        .map(|(index, _)| index)
}

fn if_condition_close(tokens: &[crate::source::Token<'_>]) -> Option<usize> {
    let mut index = first_statement_index(tokens);
    if tokens
        .get(index)
        .is_some_and(|token| token.is_name(b"else"))
    {
        index += 1;
    }
    if !tokens.get(index).is_some_and(|token| token.is_name(b"if")) {
        return None;
    }
    let open = index + 1;
    tokens
        .get(open)
        .filter(|token| token.kind == TokenKind::LParen)?;
    matching_close(tokens, open)
}

fn is_format_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    let index = first_statement_index(tokens);
    tokens
        .get(index)
        .is_some_and(|token| token.is_name(b"format"))
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::LParen)
}

fn common_block_edit(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
) -> Option<(usize, usize, Vec<u8>)> {
    let index = first_statement_index(tokens);
    if !tokens
        .get(index)
        .is_some_and(|token| token.is_name(b"common"))
    {
        return None;
    }
    let slash = tokens.get(index + 1)?;
    let name = tokens.get(index + 2)?;
    let close = tokens.get(index + 3)?;
    if slash.text != b"/"
        || name.kind != TokenKind::Name
        || close.text != b"/"
        || !horizontal_gap(line, slash.span.end, name.span.start)
        || !horizontal_gap(line, name.span.end, close.span.start)
    {
        return None;
    }
    let mut end = close.span.end;
    while end < line.len() && matches!(line[end], b' ' | b'\t') {
        end += 1;
    }
    let mut replacement = b"common /".to_vec();
    replacement.extend_from_slice(name.text);
    replacement.extend_from_slice(b"/");
    if end < line.len() && line[end] != b'!' {
        replacement.push(b' ');
    }
    Some((tokens[index].span.start, end, replacement))
}

fn top_level_separator(tokens: &[crate::source::Token<'_>]) -> Option<usize> {
    tokens.iter().position(|token| {
        token.kind == TokenKind::Operator && token.text == b"::" && token.depth == 0
    })
}

fn is_declaration_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    let index = first_statement_index(tokens);
    let Some(first) = tokens.get(index) else {
        return false;
    };
    if first.kind != TokenKind::Name {
        return false;
    }
    if first.is_name(b"double") {
        return tokens
            .get(index + 1)
            .is_some_and(|token| token.is_name(b"precision"));
    }
    matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"integer"
            | b"real"
            | b"complex"
            | b"logical"
            | b"character"
            | b"type"
            | b"class"
            | b"procedure"
            | b"dimension"
            | b"allocatable"
            | b"pointer"
            | b"target"
            | b"optional"
            | b"parameter"
            | b"save"
            | b"value"
            | b"volatile"
            | b"asynchronous"
            | b"contiguous"
            | b"codimension"
    )
}

fn reorder_optional_attribute(line: &[u8], separator: usize, incoming: LexState) -> Vec<u8> {
    let prefix = &line[..separator];
    let tokens = tokenize(prefix, &mut incoming.clone());
    let commas: Vec<usize> = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Comma && token.depth == 0)
        .map(|token| token.span.start)
        .collect();
    let mut ranges = Vec::with_capacity(commas.len() + 1);
    let mut start = 0;
    for comma in commas {
        ranges.push(start..comma);
        start = comma + 1;
    }
    ranges.push(start..prefix.len());
    let optional: Vec<&[u8]> = ranges
        .iter()
        .filter_map(|range| {
            let attribute = &prefix[range.clone()];
            trim_ascii(attribute)
                .eq_ignore_ascii_case(b"optional")
                .then_some(attribute)
        })
        .collect();
    if optional.is_empty() {
        return line.to_vec();
    }
    let mut attributes: Vec<&[u8]> = ranges
        .iter()
        .map(|range| &prefix[range.clone()])
        .filter(|attribute| !trim_ascii(attribute).eq_ignore_ascii_case(b"optional"))
        .collect();
    attributes.extend(optional);
    let mut replacement = Vec::with_capacity(prefix.len());
    for (index, attribute) in attributes.iter().enumerate() {
        if index > 0 {
            replacement.push(b',');
        }
        replacement.extend_from_slice(attribute);
    }
    let mut edits = EditBuffer::new(line);
    edits.replace(0..separator, &replacement);
    edits.finish()
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn normalize_old_style_declaration(line: &[u8], incoming: LexState) -> Vec<u8> {
    let mut state = incoming;
    let mut result = Vec::with_capacity(line.len());
    state.scan(line, |region| {
        if region.kind == RegionKind::Code {
            let code = &line[region.range];
            let mut one = Vec::new();
            normalize_old_style_code(code, &mut one);
            result.extend_from_slice(&one);
        } else {
            result.extend_from_slice(&line[region.range]);
        }
    });
    result
}

fn normalize_old_style_code(code: &[u8], out: &mut Vec<u8>) {
    let mut source = code.to_vec();
    let tokens = crate::source::tokens::tokens(code);
    let first = first_statement_index(&tokens);
    if let (Some(type_token), Some(next)) = (tokens.get(first), tokens.get(first + 1)) {
        let mut spec_end = type_token.span.end;
        if type_token.is_name(b"double") && next.is_name(b"precision") {
            spec_end = next.span.end;
        } else if matches!(
            type_token.text.to_ascii_lowercase().as_slice(),
            b"integer" | b"real" | b"complex" | b"logical" | b"character" | b"type" | b"class"
        ) && next.kind == TokenKind::LParen
        {
            if let Some(close) = matching_close(&tokens, first + 1) {
                spec_end = tokens[close].span.end;
            }
        }
        if let Some(entity) = tokens.iter().find(|token| {
            token.kind == TokenKind::Name && token.span.start >= spec_end && token.depth == 0
        }) {
            if entity.span.start == spec_end {
                let mut edits = EditBuffer::new(code);
                edits.insert(spec_end, b" ");
                source = edits.finish();
            }
        }
    }
    let leading = code
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(code.len());
    out.extend_from_slice(&source[..leading.min(source.len())]);
    let mut pending = false;
    for byte in &source[leading.min(source.len())..] {
        if matches!(byte, b' ' | b'\t') {
            pending = true;
        } else {
            if pending {
                out.push(b' ');
                pending = false;
            }
            out.push(*byte);
        }
    }
}

fn normalize_delimiters_in_code(code: &[u8], out: &mut Vec<u8>, following_content: bool) {
    let mut index = 0;
    while index < code.len() {
        if code[index] == b',' {
            let mut keep = out.len();
            while keep > 0 && matches!(out[keep - 1], b' ' | b'\t') {
                keep -= 1;
            }
            // Preserve indentation before a leading comma.
            if out[..keep].iter().any(|byte| !matches!(byte, b' ' | b'\t')) {
                out.truncate(keep);
            }
            out.push(b',');
            index += 1;
            while index < code.len() && matches!(code[index], b' ' | b'\t') {
                index += 1;
            }
            if (index < code.len() && code[index] != b'\n')
                || (index == code.len() && following_content)
            {
                out.push(b' ');
            }
            continue;
        }
        if code[index..].starts_with(b"::") {
            out.extend_from_slice(b"::");
            index += 2;
            if index < code.len() && !code[index].is_ascii_whitespace() {
                out.push(b' ');
            }
            continue;
        }
        out.push(code[index]);
        index += 1;
    }
}

pub(crate) fn is_directive_comment(comment: &[u8]) -> bool {
    if comment.len() < 2 || comment[0] != b'!' {
        return false;
    }
    if comment[1] == b'$' {
        return true;
    }
    [b"dir$".as_slice(), b"dec$", b"gcc$"].iter().any(|prefix| {
        comment[1..].len() >= prefix.len()
            && comment[1..1 + prefix.len()].eq_ignore_ascii_case(prefix)
    })
}

fn openmp_clause_body_start(line: &[u8]) -> Option<usize> {
    let start = line.iter().position(|byte| !matches!(byte, b' ' | b'\t'))?;
    if !line[start..].starts_with(b"!$") {
        return None;
    }
    let body_start = start + 2;
    let body = line.get(body_start..)?.trim_ascii_start();
    if body.len() >= 3
        && body[..3].eq_ignore_ascii_case(b"omp")
        && body
            .get(3)
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
    {
        return None;
    }
    Some(body_start)
}

fn is_commented_assignment(comment: &[u8]) -> bool {
    let mut index = 1;
    while comment
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        index += 1;
    }
    let Some(end) = identifier_end(comment, index) else {
        return false;
    };
    if end == index {
        return false;
    }
    index = end;
    loop {
        while comment
            .get(index)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            index += 1;
        }
        if comment.get(index) == Some(&b'%') {
            index += 1;
            let Some(end) = identifier_end(comment, index) else {
                return false;
            };
            if end == index {
                return false;
            }
            index = end;
        } else if comment.get(index) == Some(&b'(') {
            while index < comment.len() && comment[index] != b')' && comment[index] != b'!' {
                index += 1;
            }
            if comment.get(index) != Some(&b')') {
                return false;
            }
            index += 1;
        } else {
            break;
        }
    }
    while comment
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        index += 1;
    }
    comment.get(index) == Some(&b'=')
        && (index == 0 || !matches!(comment[index - 1], b'<' | b'>' | b'=' | b'/'))
        && comment.get(index + 1) != Some(&b'=')
        && comment.get(index + 1) != Some(&b'>')
}

fn identifier_end(bytes: &[u8], start: usize) -> Option<usize> {
    if !bytes.get(start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    Some(end)
}

fn format_comment_operators(comment: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(comment.len() + 8);
    let mut quote = 0u8;
    let mut index = 0;
    while index < comment.len() {
        let byte = comment[index];
        if quote != 0 {
            output.push(byte);
            if byte == quote {
                if comment.get(index + 1) == Some(&quote) {
                    output.push(quote);
                    index += 2;
                    continue;
                }
                quote = 0;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = byte;
            output.push(byte);
            index += 1;
            continue;
        }

        if let Some((length, replacement)) = legacy_operator_at(comment, index) {
            append_comment_operator(&mut output, replacement, true);
            index = skip_horizontal(comment, index + length);
            continue;
        }
        if let Some(length) = spaced_operator_len(comment, index) {
            let named = comment[index] == b'=' && is_named_parameter_at(comment, index);
            append_comment_operator(&mut output, &comment[index..index + length], !named);
            index = skip_horizontal(comment, index + length);
            continue;
        }
        if let Some(length) = arithmetic_operator_len(comment, index) {
            let operator = &comment[index..index + length];
            if operator == b"+" && is_binary_arithmetic_operator(comment, index, operator) {
                append_comment_operator(
                    &mut output,
                    operator,
                    !vocab::contains(vocab::COMPACT_ARITHMETIC_OPERATORS, operator),
                );
            } else {
                output.extend_from_slice(operator);
                index += length;
                continue;
            }
            index = skip_horizontal(comment, index + length);
            continue;
        }
        output.push(byte);
        index += 1;
    }
    output
}

fn append_comment_operator(output: &mut Vec<u8>, operator: &[u8], spaced: bool) {
    if spaced {
        while output.last().is_some_and(u8::is_ascii_whitespace) {
            output.pop();
        }
        if !output.is_empty() {
            output.push(b' ');
        }
        output.extend_from_slice(operator);
        output.push(b' ');
    } else {
        while output.last().is_some_and(u8::is_ascii_whitespace) {
            output.pop();
        }
        output.extend_from_slice(operator);
    }
}

fn skip_horizontal(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    index
}

fn is_named_parameter_at(line: &[u8], index: usize) -> bool {
    let mut end = index;
    while end > 0 && line[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (line[start - 1].is_ascii_alphanumeric() || line[start - 1] == b'_') {
        start -= 1;
    }
    if start == end || !line[start].is_ascii_alphabetic() {
        return false;
    }
    let mut prefix = start;
    while prefix > 0 && line[prefix - 1].is_ascii_whitespace() {
        prefix -= 1;
    }
    if prefix == 0 || !matches!(line[prefix - 1], b'(' | b',') {
        return false;
    }
    let mut depth = 0isize;
    let mut quote = 0u8;
    for &byte in &line[..index] {
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = byte;
        } else if matches!(byte, b'(' | b'[') {
            depth += 1;
        } else if matches!(byte, b')' | b']') {
            depth -= 1;
        }
    }
    depth > 0
}

fn is_specifier_keyword_argument(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    tokens
        .get(index + 1)
        .is_some_and(|token| token.text == b"=" && is_named_parameter_token(tokens, index + 1))
}

/// Whether the name at `index` is being *declared* here, and so keeps its
/// spelling instead of being read as a keyword.
///
/// `continued_entity_list` says the line continues a declaration's entity list
/// at the statement's top level.  The `::` is then on an earlier physical line,
/// and without this the whole line reads as ordinary code: a component actually
/// named `TYPE` was lowercased to `type` — but only after the wrapper had moved
/// it off the first line, so the two runs disagreed.
fn is_contextual_declaration_name(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    index: usize,
    continued_entity_list: bool,
) -> bool {
    // Names nested in an entity's shape or initializer are uses, not
    // declaration entities. In particular, an intrinsic such as SIZE in a
    // dimension bound must still receive its canonical spelling even when a
    // project declares an unrelated symbol named Size.
    if tokens.get(index).is_none_or(|token| token.depth != 0) {
        return false;
    }
    let entities_start = match tokens[..index].iter().rposition(|token| {
        token.kind == TokenKind::Operator && token.text == b"::" && token.depth == 0
    }) {
        Some(separator) => separator + 1,
        // The `::` is on an earlier physical line, so the entity list already
        // covers this one from its first token.
        None if continued_entity_list => 0,
        None => return false,
    };
    // Match `is_contextual_identifier`: a top-level comma starts a new
    // declaration entity, so an initializer on an earlier entity does not
    // affect a later one.  Its initializer scan is character-based rather
    // than depth-filtered, so `=` inside nested parentheses still qualifies.
    let mut item_start = entities_start;
    for (position, token) in tokens.iter().enumerate().take(index).skip(entities_start) {
        if token.kind == TokenKind::Comma && token.depth == 0 {
            item_start = position + 1;
        }
    }
    for token in tokens.iter().take(index).skip(item_start) {
        if token.kind != TokenKind::Operator || token.text != b"=" {
            continue;
        }
        let previous = token.span.start.checked_sub(1).and_then(|at| line.get(at));
        let following = line.get(token.span.end);
        if following == Some(&b'>')
            || (previous != Some(&b'<')
                && previous != Some(&b'>')
                && previous != Some(&b'=')
                && previous != Some(&b'/')
                && following != Some(&b'=')
                && following != Some(&b'>'))
        {
            return false;
        }
    }
    true
}

fn is_named_parameter_token(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].kind == TokenKind::Name
        && (tokens[index - 2].kind == TokenKind::LParen
            || (tokens[index - 2].kind == TokenKind::Comma && tokens[index - 2].depth > 0))
}

fn is_continued_named_parameter(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    inside_paren: bool,
) -> bool {
    inside_paren
        && index > 0
        && tokens[index - 1].kind == TokenKind::Name
        && (index == 1
            || tokens[index - 2].kind == TokenKind::Comma
            || tokens[..index - 1]
                .iter()
                .all(|token| token.kind == TokenKind::Ampersand))
}

/// Track the groups a line opens and closes, innermost last: `true` for a
/// parenthesis, `false` for a bracket.
fn fold_open_groups(line: &[u8], open: &mut Vec<bool>) {
    let mut state = LexState::default();
    for token in tokenize(line, &mut state) {
        match token.kind {
            TokenKind::LParen => open.push(true),
            TokenKind::LBracket => open.push(false),
            TokenKind::RParen | TokenKind::RBracket => {
                open.pop();
            }
            _ => {}
        }
    }
}

/// For each token, whether the innermost group open *at* that token is a
/// parenthesis.
///
/// Named arguments live in `(...)`.  `[...]` is an array constructor: a
/// `name =` after one of its commas is the next entity of a declaration list,
/// not a keyword.  Both matter on the same line — `…, b = &` after a `]`
/// belongs to the declaration list, while `…))], dim=1)` has closed its bracket
/// and is back inside the call — so this is folded per token rather than
/// decided once for the line.
fn inside_paren_at(open_groups: &[bool], tokens: &[crate::source::Token<'_>]) -> Vec<bool> {
    let mut open = open_groups.to_vec();
    let mut result = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token.kind {
            TokenKind::LParen => {
                result.push(open.last().copied().unwrap_or(false));
                open.push(true);
            }
            TokenKind::LBracket => {
                result.push(open.last().copied().unwrap_or(false));
                open.push(false);
            }
            TokenKind::RParen | TokenKind::RBracket => {
                open.pop();
                result.push(open.last().copied().unwrap_or(false));
            }
            _ => result.push(open.last().copied().unwrap_or(false)),
        }
    }
    result
}

fn real_exponent_marker(number: &[u8]) -> Option<usize> {
    let mut index = 0;
    let mut digits = 0;
    while number.get(index).is_some_and(u8::is_ascii_digit) {
        digits += 1;
        index += 1;
    }
    if number.get(index) == Some(&b'.') {
        index += 1;
        while number.get(index).is_some_and(u8::is_ascii_digit) {
            digits += 1;
            index += 1;
        }
    }
    if digits == 0 || !matches!(number.get(index), Some(b'E' | b'e' | b'D' | b'd')) {
        return None;
    }
    let marker = index;
    index += 1;
    if matches!(number.get(index), Some(b'+' | b'-')) {
        index += 1;
    }
    if !number.get(index).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some(marker)
}

fn modern_operator(token: &[u8]) -> Option<&'static [u8]> {
    if token.len() != 4 || token[0] != b'.' || token[3] != b'.' {
        return None;
    }
    match token[1].to_ascii_lowercase() {
        b'e' if token[2].eq_ignore_ascii_case(&b'q') => Some(b"=="),
        b'n' if token[2].eq_ignore_ascii_case(&b'e') => Some(b"/="),
        b'l' if token[2].eq_ignore_ascii_case(&b't') => Some(b"<"),
        b'l' if token[2].eq_ignore_ascii_case(&b'e') => Some(b"<="),
        b'g' if token[2].eq_ignore_ascii_case(&b't') => Some(b">"),
        b'g' if token[2].eq_ignore_ascii_case(&b'e') => Some(b">="),
        _ => None,
    }
}

/// The lowercase spelling of a recognized dotted word, when it
/// differs from what is written.
///
/// `.TRUE.` and `.FALSE.` reach `lowercase_keyword` as the bare word between the
/// dots, so they are lowered through `INTRINSIC_NAMES` like any intrinsic.  A
/// user-defined operator such as `.MYOP.` is in no table and keeps its spelling.
/// Returns `None` when nothing would change, so no edit is recorded.
fn dotted_word_lowering(token: &[u8]) -> Option<Vec<u8>> {
    let word = token.strip_prefix(b".")?.strip_suffix(b".")?;
    if word.is_empty() || !word.iter().any(u8::is_ascii_uppercase) {
        return None;
    }
    let lowered = word.to_ascii_lowercase();
    if !vocab::contains(vocab::INTRINSIC_NAMES, &lowered) {
        return None;
    }
    let mut out = Vec::with_capacity(token.len());
    out.push(b'.');
    out.extend_from_slice(&lowered);
    out.push(b'.');
    Some(out)
}

fn is_spaced_dotted_operator(token: &[u8]) -> bool {
    [b".and.".as_slice(), b".or.", b".not.", b".eqv.", b".neqv."]
        .iter()
        .any(|operator| token.eq_ignore_ascii_case(operator))
}

fn is_spaced_operator_token(
    line: &[u8],
    _tokens: &[crate::source::Token<'_>],
    _index: usize,
    token: &crate::source::Token<'_>,
) -> bool {
    let start = token.span.start;
    let end = token.span.end;
    match token.text {
        b"=>" | b"==" | b"/=" | b"<=" | b">=" => true,
        b"<" => {
            (start == 0 || !matches!(line[start - 1], b'=' | b'<' | b'>'))
                && (end == line.len() || !matches!(line[end], b'<' | b'>'))
        }
        b">" => {
            (start == 0 || !matches!(line[start - 1], b'=' | b'<' | b'>' | b'-'))
                && (end == line.len() || !matches!(line[end], b'<' | b'>'))
        }
        b"=" => {
            (start == 0 || !matches!(line[start - 1], b'<' | b'>' | b'=' | b'/'))
                && (end == line.len() || !matches!(line[end], b'=' | b'>'))
        }
        _ => false,
    }
}

fn is_arithmetic_operator(operator: &[u8]) -> bool {
    matches!(operator, b"+" | b"-" | b"*" | b"/" | b"**" | b"//")
}

fn is_binary_arithmetic_operator(line: &[u8], index: usize, operator: &[u8]) -> bool {
    let mut previous = index;
    while previous > 0 && line[previous - 1].is_ascii_whitespace() {
        previous -= 1;
    }
    let mut following = index + operator.len();
    while following < line.len() && line[following].is_ascii_whitespace() {
        following += 1;
    }
    if operator == b"//" {
        return following < line.len();
    }
    if previous == 0 || following >= line.len() {
        return false;
    }
    let previous_byte = line[previous - 1];
    if !matches!(
        previous_byte,
        b')' | b']' | b'_' | b'.' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'
    ) {
        return false;
    }
    if previous_byte == b'.' && dotted_operator_before(line, previous) {
        return false;
    }
    if matches!(operator, b"+" | b"-") && exponent_before(line, index) {
        return false;
    }
    !matches!(line[following], b')' | b']' | b',')
}

fn dotted_operator_before(line: &[u8], end: usize) -> bool {
    let dot = end.saturating_sub(1);
    let mut start = dot;
    while start > 0 && line[start - 1].is_ascii_alphabetic() {
        start -= 1;
    }
    start < dot && start > 0 && line[start - 1] == b'.'
}

fn exponent_before(line: &[u8], index: usize) -> bool {
    index > 0
        && matches!(line[index - 1], b'e' | b'E' | b'd' | b'D')
        && index > 1
        && (line[index - 2].is_ascii_digit() || line[index - 2] == b'.')
}

/// Carries one bit of left-to-right context between adjacent operator edits.
///
/// The pass builds its output with a single accumulator and asks "did I
/// already write a space?" before padding the next operator.  Span edits have no
/// such accumulator, so two adjacent operators each pad their own side and
/// `a=.not.b` comes out as `a =  .not. b`.  Recording where the previous edit
/// left a trailing space restores the accumulator's answer without giving up the
/// span-edit discipline that keeps protected bytes untouched (I3).
#[derive(Default)]
struct OperatorSpacing {
    /// Exclusive end, in the *original* line, of the previous operator edit.
    /// Walking left past it would overlap that edit, and `EditBuffer` drops
    /// overlapping edits — which is how `.AND. .NOT.` used to lose its second
    /// operator entirely.
    previous_end: Option<usize>,
    /// Whether that edit already wrote the space between the two operators.
    previous_trailing_space: bool,
    /// Whether that edit was a deliberately compact `name=`.  The token that
    /// abuts such an edit opens the argument's value, so a `.not.` there is
    /// unary and must stay against the `=`: `append=.not. new_chains`, never
    /// the lopsided `append= .not. new_chains`.
    previous_compact_named: bool,
}

fn add_operator_edit(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    operator: &[u8],
    spaced: bool,
    spacing: &mut OperatorSpacing,
) {
    let floor = spacing.previous_end.unwrap_or(0);
    let mut left = token.span.start;
    while left > floor && line[left - 1].is_ascii_whitespace() {
        left -= 1;
    }
    let mut right = token.span.end;
    while right < line.len() && line[right].is_ascii_whitespace() {
        right += 1;
    }
    let abuts_previous = spacing.previous_end == Some(left);
    let suppress_leading_space =
        abuts_previous && (spacing.previous_trailing_space || spacing.previous_compact_named);
    let mut replacement = Vec::new();
    if left == 0 {
        replacement.extend_from_slice(&line[..token.span.start]);
    }
    if spaced && left > 0 && !suppress_leading_space {
        replacement.push(b' ');
    }
    replacement.extend_from_slice(operator);
    let trailing = spaced || is_trailing_continuation_marker(line, token.span.end);
    if trailing {
        replacement.push(b' ');
    }
    spacing.previous_end = Some(right);
    spacing.previous_trailing_space = trailing;
    spacing.previous_compact_named = false;
    edits.replace(left..right, &replacement);
}

fn remove_operator_trailing_whitespace(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    spacing: &mut OperatorSpacing,
) {
    let mut end = token.span.end;
    while end < line.len() && line[end].is_ascii_whitespace() {
        end += 1;
    }
    if end > token.span.end && !is_trailing_continuation_marker(line, token.span.end) {
        edits.replace(token.span.end..end, b"");
        spacing.previous_end = Some(end);
    } else {
        spacing.previous_end = Some(token.span.end);
    }
    spacing.previous_trailing_space = false;
    spacing.previous_compact_named = false;
}

fn is_trailing_continuation_marker(line: &[u8], start: usize) -> bool {
    let mut index = start;
    while index < line.len() && line[index].is_ascii_whitespace() {
        index += 1;
    }
    index < line.len()
        && line[index] == b'&'
        && line[index + 1..]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
}

fn legacy_operator_at(line: &[u8], index: usize) -> Option<(usize, &'static [u8])> {
    for (source, replacement) in [
        (b".eq.".as_slice(), b"==".as_slice()),
        (b".ne.", b"/="),
        (b".lt.", b"<"),
        (b".le.", b"<="),
        (b".gt.", b">"),
        (b".ge.", b">="),
    ] {
        if line[index..].len() >= source.len()
            && line[index..index + source.len()].eq_ignore_ascii_case(source)
        {
            return Some((source.len(), replacement));
        }
    }
    None
}

fn spaced_operator_len(line: &[u8], index: usize) -> Option<usize> {
    for operator in [b".and.".as_slice(), b".or.", b".not.", b".eqv.", b".neqv."] {
        if line[index..].len() >= operator.len()
            && line[index..index + operator.len()].eq_ignore_ascii_case(operator)
        {
            return Some(operator.len());
        }
    }
    for operator in [b"=>".as_slice(), b"==", b"/=", b"<=", b">="] {
        if line[index..].starts_with(operator) {
            return Some(operator.len());
        }
    }
    let byte = *line.get(index)?;
    let previous = index.checked_sub(1).and_then(|at| line.get(at)).copied();
    let next = line.get(index + 1).copied();
    let valid = match byte {
        b'<' => !matches!(previous, Some(b'=' | b'<' | b'>')) && !matches!(next, Some(b'<' | b'>')),
        b'>' => {
            !matches!(previous, Some(b'=' | b'<' | b'>' | b'-'))
                && !matches!(next, Some(b'<' | b'>'))
        }
        b'=' => {
            !matches!(previous, Some(b'<' | b'>' | b'=' | b'/'))
                && !matches!(next, Some(b'=' | b'>'))
        }
        _ => false,
    };
    valid.then_some(1)
}

fn arithmetic_operator_len(line: &[u8], index: usize) -> Option<usize> {
    for operator in [b"**".as_slice(), b"//", b"+", b"-", b"*", b"/"] {
        if line[index..].starts_with(operator) {
            return Some(operator.len());
        }
    }
    None
}

/// True when the slice is inside a protected region of `line`.  Rules use this
/// when they cannot express themselves as token edits.
pub fn is_protected(line: &[u8], offset: usize) -> bool {
    let mut protected = false;
    LexState::default().scan(line, |region| {
        if region.range.contains(&offset) && region.kind != RegionKind::Code {
            protected = true;
        }
    });
    protected
}

#[cfg(test)]
mod tests {
    use crate::{
        analysis::{analyze_file, ProjectContext, ScopeTree},
        config::FormatConfig,
        format_source,
        transform::{
            document::Document,
            pipeline::{Changed, PassContext},
        },
        FormatMode,
    };

    fn normalized(source: &[u8]) -> String {
        let mut document = Document::from_bytes(source);
        let project = ProjectContext::empty();
        let local = analyze_file(source).unwrap();
        let config = FormatConfig::default();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        assert_ne!(
            super::run(&mut document, &context).unwrap(),
            Changed::Structure,
            "the per-line chain must not change the line count"
        );
        String::from_utf8_lossy(&document.to_bytes()).into_owned()
    }

    fn full_pipeline(source: &[u8]) -> String {
        let config = FormatConfig {
            mode: FormatMode::Full,
            apply_indent: false,
            ..FormatConfig::default()
        };
        String::from_utf8(format_source(source, &config).unwrap().bytes).unwrap()
    }

    #[test]
    fn keywords_are_lowercased_and_identifiers_are_not() {
        assert_eq!(
            normalized(
                b"PROGRAM Main\nIF (X > 1) THEN\nCALL DoThing(Arg)\nEND IF\nEND PROGRAM Main\n"
            ),
            "program Main\nif (X > 1) then\ncall DoThing(Arg)\nend if\nend program Main\n"
        );
    }

    #[test]
    fn adjacent_operators_are_padded_once_not_twice() {
        // Regression: `=` and `.not.` each padded their own side, because span
        // edits cannot see what a neighbouring edit already wrote.  The
        // the formatter emits exactly one space between them.
        assert_eq!(normalized(b"a = .not. b\n"), "a = .not. b\n");
        assert_eq!(normalized(b"a=.not.b\n"), "a = .not. b\n");
        assert_eq!(normalized(b"a =.not. b\n"), "a = .not. b\n");
        assert_eq!(normalized(b"a=b.and..not.c\n"), "a = b .and. .not. c\n");
        assert_eq!(normalized(b"if (a) c=.not.d\n"), "if (a) c = .not. d\n");
    }

    #[test]
    fn dotted_words_in_the_intrinsic_table_are_lowercased() {
        assert_eq!(
            normalized(b"x = .TRUE.\ny = .FALSE.\n"),
            "x = .true.\ny = .false.\n"
        );
        assert_eq!(normalized(b"a = .NOT. b\n"), "a = .not. b\n");
        assert_eq!(
            normalized(b"a = b .AND. .NOT. c\n"),
            "a = b .and. .not. c\n"
        );
        // A user-defined operator is in no table and keeps its spelling.
        assert_eq!(normalized(b"z = a .MYOP. b\n"), "z = a .MYOP. b\n");
        // Protected bytes are untouched (I3).
        assert_eq!(
            normalized(b"s = '.TRUE.' ! .TRUE.\n"),
            "s = '.TRUE.' ! .TRUE.\n"
        );
    }

    #[test]
    fn only_is_lowercased_even_after_a_double_colon() {
        // `USE, INTRINSIC :: m, ONLY: x` puts the word inside the
        // declaration-name guard, so the spacing rule has to carry the case.
        assert_eq!(
            normalized(b"use, intrinsic :: iso_c_binding, ONLY: A\n"),
            "use, intrinsic :: iso_c_binding, only: A\n"
        );
        assert_eq!(normalized(b"use m, ONLY : A\n"), "use m, only: A\n");
    }

    #[test]
    fn adjacent_operator_padding_is_idempotent() {
        let once = normalized(b"a=.not.b\nx=y.and..not.z\n");
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn a_named_argument_keeps_a_dotted_operator_against_its_equals() {
        // The `=` writes no trailing space on purpose; the `.not.` that opens
        // the value must not push itself off it.  The joined-statement path
        // compacts this shape, so the two disagreed on a wrapped call and the
        // file needed a second run to settle.
        assert_eq!(
            normalized(b"call f(a, append=.not. new_chains)\n"),
            "call f(a, append=.not. new_chains)\n"
        );
        assert_eq!(
            normalized(b"call f(a, append= .not. new_chains)\n"),
            "call f(a, append=.not. new_chains)\n"
        );
        // A real assignment still pads both sides.
        assert_eq!(
            normalized(b"append=.not.new_chains\n"),
            "append = .not. new_chains\n"
        );
    }

    #[test]
    fn an_undeclared_name_that_is_a_keyword_is_lowercased() {
        assert_eq!(normalized(b"CALL sub(x)\nSTOP\n"), "call sub(x)\nstop\n");
    }

    #[test]
    fn context_sensitive_keywords_are_only_keywords_in_their_own_shape() {
        // `BIND(C, name=...)` is not the `bind(c)` language binding.
        assert_eq!(
            normalized(b"real(dl) function f(a) BIND(C, name='exported')\n"),
            "real(dl) function f(a) BIND(C, name='exported')\n"
        );
        assert_eq!(
            normalized(b"subroutine s() BIND(C)\n"),
            "subroutine s bind(C)\n"
        );
        assert_eq!(normalized(b"USE m, ONLY: x\n"), "use m, only: x\n");
        assert_eq!(normalized(b"x = ONLY + 1\n"), "x = ONLY + 1\n");
        // `precision` is an intrinsic and a specifier, not a keyword, so this
        // slice leaves it alone entirely; the guard below is already in place
        // for when the intrinsic table joins the rule.
        assert_eq!(
            normalized(b"DOUBLE PRECISION :: y\n"),
            "double precision :: y\n"
        );
        assert_eq!(normalized(b"z = PRECISION\n"), "z = PRECISION\n");
        assert_eq!(
            normalized(b"integer(KIND=4) :: n\n"),
            "integer(kind=4) :: n\n"
        );
        // `POINTER` is an attribute here and an ordinary word there.
        assert_eq!(
            normalized(b"integer, POINTER :: p\n"),
            "integer, pointer :: p\n"
        );
        assert_eq!(normalized(b"call sub(POINTER)\n"), "call sub(POINTER)\n");
    }

    #[test]
    fn a_declared_name_that_collides_with_a_keyword_is_left_alone() {
        // `Data` is declared here, so the DATA keyword rule must not touch it.
        let source = b"module M\ntype :: Data\nend type Data\nend module M\n";
        assert_eq!(
            normalized(source),
            "module M\ntype :: Data\nend type Data\nend module M\n"
        );
    }

    #[test]
    fn string_literals_and_comments_keep_their_case() {
        // Normalization keeps the authored gap before the comment; the
        // block-wide decision to compress it belongs to
        // `layout_post::trailing_comment_alignment`, which cannot be made from
        // one line. `an_isolated_trailing_comment_keeps_one_space` covers the
        // end-to-end result.
        assert_eq!(
            normalized(b"CALL sub('IF THEN END')  ! IF THEN END\n"),
            "call sub('IF THEN END')  ! IF THEN END\n"
        );
    }

    #[test]
    fn a_component_after_percent_is_not_a_keyword() {
        assert_eq!(normalized(b"X = State%Data\n"), "X = State%Data\n");
    }

    #[test]
    fn preprocessor_lines_are_preserved_byte_for_byte() {
        assert_eq!(
            normalized(b"#define IF_THING 1\n#if defined(IF_THING)\nCALL X\n#endif\n"),
            "#define IF_THING 1\n#if defined(IF_THING)\ncall X\n#endif\n"
        );
    }

    #[test]
    fn a_literal_continued_across_lines_is_not_reinterpreted_as_code() {
        assert_eq!(
            normalized(b"x = 'THEN END &\n  IF' // Y\n"),
            "x = 'THEN END &\n  IF' // Y\n"
        );
    }

    #[test]
    fn keyword_and_delimiter_rules_match_expected_shapes() {
        let source = b"ENDIF\n\
ELSEIF  ( X )\n\
BLOCKDATA\n\
GO   TO 10\n\
DOUBLE   PRECISION :: X\n\
IF( X )THEN\n\
SELECT   TYPE   IS   ( X )\n\
DO    WHILE( X )\n\
COMMON / blk / x\n\
SUBROUTINE s( )\n\
x = (/ 1 , 2 /)\n\
FORMAT((/ 1, 2 /))\n\
WRITE( UNIT = 1 , FMT = 2 )'x'\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "end if\nelse if (X)\nblock data\ngoto 10\ndouble precision :: X\nif (X) then\nselect type is (X)\ndo while (X)\ncommon /blk/ x\nsubroutine s\nx = [1, 2]\nformat((/1, 2 /))\nwrite(unit=1, fmt=2) 'x'\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn keyword_to_name_gaps_collapse_to_one_space() {
        let source = b"module   mymod\n\
use   mymod\n\
call   foo(x)\n\
subroutine   do_thing\n\
end subroutine   do_thing\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "module mymod\nuse mymod\ncall foo(x)\nsubroutine do_thing\nend subroutine do_thing\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn concatenation_spacing_survives_a_continuation_line() {
        let source = b"call MpiStop('SP(k) cannot be combined with HMCode_A_baryon/' &\n\
    // 'HMCode_eta_baryon baryonic corrections in HMCode 2015/2016')\n";
        assert_eq!(normalized(source), String::from_utf8_lossy(source));
    }

    #[test]
    fn go_to_is_compacted_after_a_continuation_join() {
        let source = b"GO &\n  TO 10\n";
        let document = Document::from_bytes(source);
        let project = ProjectContext::empty();
        let local = analyze_file(source).unwrap();
        let config = FormatConfig::default();
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        let declared_names = crate::analysis::scoped_declared_names(&analysis, &scopes);
        assert_eq!(
            super::respace_joined(b"GO TO 10", &context, &declared_names, 0),
            b"goto 10"
        );
    }

    #[test]
    fn post_f2008_keywords_are_lowercased_and_spaced() {
        let source = b"IMPURE  ELEMENTAL FUNCTION f(x)\n\
PURE   ELEMENTAL SUBROUTINE s\n\
CONTIGUOUS :: x\n\
CRITICAL(STAT = istat)\n\
CHANGE   TEAM(newteam)\n\
SELECT  RANK(a)\n\
RANK  DEFAULT\n\
FORM  TEAM(n, team, STAT=istat)\n\
SYNC  ALL(STAT=istat)\n\
SYNC   TEAM(team)\n\
EVENT  POST(event)\n\
EVENT WAIT(event, UNTIL_COUNT =n)\n\
FAIL  IMAGE\n\
LOCK(lockvar, ACQUIRED_LOCK = acquired)\n\
UNLOCK(lockvar)\n\
DO  CONCURRENT(i=1:n) LOCAL_INIT(x) SHARED(y) REDUCE(+:z)\n";
        assert_eq!(
            normalized(source),
            "impure elemental function f(x)\n\
pure elemental subroutine s\n\
contiguous :: x\n\
critical(stat=istat)\n\
change team (newteam)\n\
select rank (a)\n\
rank default\n\
form team (n, team, stat=istat)\n\
sync all(stat=istat)\n\
sync team (team)\n\
event post(event)\n\
event wait(event, until_count=n)\n\
fail image\n\
lock(lockvar, acquired_lock=acquired)\n\
unlock(lockvar)\n\
do concurrent(i=1:n) local_init(x) shared(y) reduce(+:z)\n"
        );
    }

    #[test]
    fn chunk_a_operators_exponents_and_comments_are_narrow() {
        let source =
            b"x=1.0E-3+Y.eq.-1\n! x%value= a.eq.b + 2\nCALL sub('IF ( X )', A , B ) !keep\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "x = 1.0e-3 + Y == -1\n! x%value = a == b + 2\ncall sub('IF ( X )', A, B) ! keep\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn declaration_spacing_and_intrinsic_case_respect_names() {
        assert_eq!(
            normalized(b"INTEGER, OPTIONAL, INTENT(IN) :: X\n"),
            "integer, intent(in), optional:: X\n"
        );
        assert_eq!(
            normalized(b"REAL(KIND=8)X,Y\nX = SIZE + SQRT(Y)\n"),
            "real(kind=8) X, Y\nX = size + sqrt(Y)\n"
        );
        assert_eq!(
            normalized(b"SUBROUTINE s(Write)\nX = Write ( 1 )\nEND SUBROUTINE s\n"),
            "subroutine s(Write)\nX = Write (1)\nend subroutine s\n"
        );
    }

    #[test]
    fn dimension_and_write_output_spacing_matches_expected_shape() {
        let source = b"integer, dimension (:) :: values\nwrite(*, *)'Warning...'\nwrite(unit, '(1I6,4E15.6)')il, value\nwrite(unit, '(1I6,4E15.6)')\nwrite(unit, '(1I6,4E15.6)') &\nwrite(unit, '(1I6,4E15.6)' ) ! no output item\nprint *, \"write(*)'literal'\"\n! write(*)'comment'\n";
        assert_eq!(
            normalized(source),
            "integer, dimension(:) :: values\nwrite(*, *) 'Warning...'\nwrite(unit, '(1I6,4E15.6)') il, value\nwrite(unit, '(1I6,4E15.6)')\nwrite(unit, '(1I6,4E15.6)') &\nwrite(unit, '(1I6,4E15.6)' ) ! no output item\nprint *, \"write(*)'literal'\"\n! write(*)'comment'\n"
        );
    }

    #[test]
    fn parenthesized_statements_lowercase_unless_locally_shadowed() {
        let source = b"WRITE (*, *) value\nREAD (unit, *) value\nOPEN (newunit=unit, file=name)\nBACKSPACE (unit)\nALLOCATED (value)\nC%Write (*, *) value\nsubroutine s\nprocedure :: Write\ncall WRITE()\nend subroutine s\n";
        assert_eq!(
            full_pipeline(source),
            "write(*, *) value\nread(unit, *) value\nopen(newunit=unit, file=name)\nbackspace(unit)\nallocated(value)\nC%Write(*, *) value\nsubroutine s\nprocedure :: Write\ncall Write()\n\nend subroutine s\n"
        );
    }

    #[test]
    fn old_style_declarations_normalize_spacing_and_optional_order() {
        let source = b"    real(dl)  x\n    real(dl)kh, PK\n    real(dp), optional, intent(out) :: sin_k\n    real(dp), intent(in), optional :: cos_k\n";
        assert_eq!(
            full_pipeline(source),
            "    real(dl) x\n    real(dl) kh, PK\n    real(dp), intent(out), optional :: sin_k\n    real(dp), intent(in), optional :: cos_k\n"
        );
    }

    #[test]
    fn a_local_intrinsic_name_is_scoped_to_its_own_procedure() {
        let source = b"SUBROUTINE first()\nINTEGER :: SIZE\nX = SIZE\nEND SUBROUTINE first\n\
SUBROUTINE second()\nX = SIZE\nEND SUBROUTINE second\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "subroutine first\ninteger :: SIZE\nX = SIZE\nend subroutine first\n\
subroutine second\nX = size\nend subroutine second\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn module_declared_names_are_visible_inside_contained_procedures_only() {
        let source = b"MODULE m\nINTEGER :: STATUS\nCONTAINS\nSUBROUTINE s()\nX = STATUS\nEND SUBROUTINE s\nEND MODULE m\n\
X = STATUS\n";
        assert_eq!(
            normalized(source),
            "module m\ninteger :: STATUS\ncontains\nsubroutine s\nX = STATUS\nend subroutine s\nend module m\n\
X = status\n"
        );
    }

    #[test]
    fn a_procedure_name_from_one_module_does_not_shadow_an_intrinsic_in_another() {
        let source =
            b"MODULE a\nCONTAINS\nFUNCTION SIZE()\nSIZE = 1\nEND FUNCTION SIZE\nEND MODULE a\n\
MODULE b\nX = SIZE(1)\nEND MODULE b\n";
        assert_eq!(
            normalized(source),
            "module a\ncontains\nfunction SIZE()\nSIZE = 1\nend function SIZE\nend module a\n\
module b\nX = size(1)\nend module b\n"
        );
    }

    #[test]
    fn local_and_file_names_have_different_keyword_argument_rules() {
        let local =
            normalized(b"SUBROUTINE s(STATUS)\nCALL f(x, STATUS=STATUS)\nEND SUBROUTINE s\n");
        assert_eq!(
            local,
            "subroutine s(STATUS)\ncall f(x, STATUS=STATUS)\nend subroutine s\n"
        );

        let file = normalized(
            b"MODULE m\nINTEGER :: STATUS\nCONTAINS\nSUBROUTINE s()\nCALL f(x, STATUS=STATUS)\nEND SUBROUTINE s\nEND MODULE m\n",
        );
        assert_eq!(
            file,
            "module m\ninteger :: STATUS\ncontains\nsubroutine s\ncall f(x, status=STATUS)\nend subroutine s\nend module m\n"
        );
    }

    #[test]
    fn dollar_sentinel_clause_bodies_follow_fortran_normalization() {
        assert_eq!(normalized(b"!$ USE OMP_LIB\n"), "!$ use OMP_LIB\n");
        assert_eq!(
            normalized(b"!$ IF(X.EQ.1) CALL F( A , B )\n"),
            "!$ if (X == 1) call F(A, B)\n"
        );
    }

    #[test]
    fn dollar_sentinel_boundaries_and_protected_text_are_preserved() {
        let source = b"! USE OMP_LIB\n!$OMP IF(X.EQ.1) CALL F( A , B )\n!$\n  !$ USE OMP_LIB\n!$ CALL F('IF THEN', A)\n";
        let once = normalized(source);
        assert_eq!(
            once,
            "! USE OMP_LIB\n!$OMP IF(X.EQ.1) CALL F( A , B )\n!$\n  !$ use OMP_LIB\n!$ call F('IF THEN', A)\n"
        );
        assert_eq!(normalized(once.as_bytes()), once);
    }

    #[test]
    fn contextual_declaration_names_reset_after_top_level_initializers() {
        assert_eq!(
            normalized(b"INTEGER :: A = 1, SIZE\n"),
            "integer :: A = 1, SIZE\n"
        );
    }

    #[test]
    fn contextual_declaration_initializer_scan_sees_nested_equals() {
        assert_eq!(
            normalized(b"REAL :: X(F(N=1) + SIZE)\n"),
            "real :: X(F(N=1) + size)\n"
        );
    }

    #[test]
    fn uppercase_single_l_is_opt_in_and_protected_bytes_are_untouched() {
        let source = b"x = l + 'l' ! l\n#define L 1\n";
        let mut document = Document::from_bytes(source);
        let project = ProjectContext::empty();
        let local = analyze_file(source).unwrap();
        let config = FormatConfig {
            uppercase_single_l: true,
            ..FormatConfig::default()
        };
        let analysis = document.analyze().unwrap();
        let scopes = ScopeTree::build(&analysis);
        let context = PassContext {
            config: &config,
            project: &project,
            local: &local,
            analysis: &analysis,
            scopes: &scopes,
        };
        super::run(&mut document, &context).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&document.to_bytes()),
            "x = L + 'l' ! l\n#define L 1\n"
        );
    }

    #[test]
    fn joined_named_arguments_keep_compact_equals() {
        assert_eq!(
            super::compact_joined_named_arguments(
                b"call compute(alpha, nested(first, second), named = value)"
            ),
            b"call compute(alpha, nested(first, second), named=value)"
        );
    }
}
