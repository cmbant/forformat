use crate::{
    analysis::{DeclaredNameIndex, DeclaredSpelling},
    config::{KeywordCase, StyleConfig},
    source::{
        regions::{LexState, RegionKind},
        syntax::declaration_type_head_len,
        tokens::{join_preserves_boundary, tokenize, TokenKind},
        PhysicalLineKind,
    },
    transform::{document::Document, edit::EditBuffer, pipeline::PassContext, vocab},
};

#[path = "case.rs"]
pub(super) mod case;
#[path = "comment_spacing.rs"]
pub(super) mod comment_spacing;
#[path = "delimiter_spacing.rs"]
pub(super) mod delimiter_spacing;
#[path = "keyword_spacing.rs"]
pub(super) mod keyword_spacing;
#[path = "write_spacing.rs"]
pub(super) mod write_spacing;

fn horizontal_gap(line: &[u8], start: usize, end: usize) -> bool {
    start <= end
        && line[start..end]
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t'))
}

/// Whether the token at `index` is the `(` that opens a legacy `(/ ... /)`
/// array constructor.
///
/// Shared with [`keyword_spacing::Rules::array_constructor_brackets`], which
/// rewrites exactly these to `[`, so that nothing can read the two spellings of
/// one construct differently -- least of all in the pass that turns one into the
/// other. Reading `(/` as a plain `(` opened a keyword-argument group, and `[`
/// does not, so `(/&` / `,n=` kept `n=` compact on the run that rewrote the
/// bracket and spaced it on the run after.
///
/// `/=` lexes as one operator, so a comparison is not a candidate, and the only
/// other thing a `/` can be directly after an open paren is a `FORMAT` record
/// separator -- which is not a keyword-argument group either.
pub(super) fn opens_array_constructor(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    index: usize,
) -> bool {
    tokens[index].kind == TokenKind::LParen
        && tokens.get(index + 1).is_some_and(|next| {
            next.kind == TokenKind::Operator
                && next.text == b"/"
                && horizontal_gap(line, tokens[index].span.end, next.span.start)
        })
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

fn is_labelled_format_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    first_statement_index(tokens) == 1 && is_format_statement(tokens)
}

pub(super) fn is_format_statement(tokens: &[crate::source::Token<'_>]) -> bool {
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

/// True when the statement carries a `::` outside every bracket: the token
/// that separates a modern declaration's attributes from its entity list.
pub(super) fn has_top_level_separator(tokens: &[crate::source::Token<'_>]) -> bool {
    top_level_separator(tokens).is_some()
}

fn top_level_separator(tokens: &[crate::source::Token<'_>]) -> Option<usize> {
    tokens.iter().position(|token| {
        token.kind == TokenKind::Operator && token.text == b"::" && token.depth == 0
    })
}

pub(super) fn is_declaration_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    let index = first_statement_index(tokens);
    let Some(first) = tokens.get(index) else {
        return false;
    };
    if first.kind != TokenKind::Name {
        return false;
    }
    if declaration_type_head_len(tokens, index).is_some() {
        return true;
    }
    matches!(
        first.text.to_ascii_lowercase().as_slice(),
        b"procedure"
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

/// Whether the name at `index` is the *name* half of a `keyword=value` pair.
///
/// A continuation line carries no statement context, so the `(` or `,` that
/// makes the pair a keyword argument may be on an earlier line: the shape has
/// to be recognized from the threaded context as well, or a keyword argument
/// is read as an ordinary name on exactly the lines the wrapper creates, and
/// the two readings disagree from one run to the next (I1).
fn is_specifier_keyword_argument(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    inside_paren: &[bool],
    context: &super::LineContext<'_>,
) -> bool {
    tokens.get(index + 1).is_some_and(|token| {
        token.text == b"=" && is_keyword_argument_equals(tokens, index + 1, inside_paren, context)
    })
}

/// The `=` at `index` separates a keyword argument from its value, on this
/// line or through the continuation context threaded into it.
pub(super) fn is_keyword_argument_equals(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    inside_paren: &[bool],
    context: &super::LineContext<'_>,
) -> bool {
    is_named_parameter_token(tokens, index)
        || context.continued_statement
            && (!context.continued_declaration && context.continued_named_parameter
                || context.continued_bind_parameter)
            && is_continued_named_parameter(tokens, index, inside_paren[index])
}

/// Whether the line's first token continues a `%` component selector left
/// open by the previous line, as in `ptr % &` / `data % simple_int`.
pub(super) fn continues_component_selector(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    context: &super::LineContext<'_>,
) -> bool {
    context.continued_component
        && tokens[..index]
            .iter()
            .all(|token| token.kind == TokenKind::Ampersand)
}

/// Whether `line`'s last significant token is the `%` of a component
/// selector, so the statement's next line opens with a component name.
pub(super) fn trailing_component_selector(line: &[u8]) -> bool {
    let mut state = LexState::default();
    let tokens = tokenize(line, &mut state);
    tokens
        .iter()
        .rev()
        .find(|token| !matches!(token.kind, TokenKind::Ampersand | TokenKind::Comment))
        .is_some_and(|token| token.kind == TokenKind::Operator && token.text == b"%")
}

fn is_contextual_declaration_name(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    index: usize,
    continued_entity_list: bool,
) -> bool {
    if tokens.get(index).is_none_or(|token| token.depth != 0) {
        return false;
    }
    let entities_start = match tokens[..index].iter().rposition(|token| {
        token.kind == TokenKind::Operator && token.text == b"::" && token.depth == 0
    }) {
        Some(separator) => separator + 1,
        None if continued_entity_list => 0,
        None => return false,
    };
    let mut item_start = entities_start;
    for (position, token) in tokens.iter().enumerate().take(index).skip(entities_start) {
        if token.kind == TokenKind::Comma && token.depth == 0 {
            item_start = position + 1;
        }
    }
    for token in tokens.iter().take(index).skip(item_start) {
        if token.kind != TokenKind::Operator {
            continue;
        }
        // A pointer initialization ends the entity name and opens an ordinary
        // expression, so `vecR(:, :) => null()` references the intrinsic. The
        // lexer spells `=>` as one token, which is why the byte test below
        // never saw it.
        if token.text == b"=>" {
            return false;
        }
        if token.text != b"=" {
            continue;
        }
        let previous = token.span.start.checked_sub(1).and_then(|at| line.get(at));
        let following = line.get(token.span.end);
        if previous != Some(&b'<')
            && previous != Some(&b'>')
            && previous != Some(&b'=')
            && previous != Some(&b'/')
            && following != Some(&b'=')
        {
            return false;
        }
    }
    true
}

fn is_old_style_declaration_entity(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    if !is_declaration_statement(tokens) || top_level_separator(tokens).is_some() {
        return false;
    }
    let first = first_statement_index(tokens);
    let mut entity_start = first + declaration_type_head_len(tokens, first).unwrap_or(1);
    if tokens
        .get(entity_start)
        .is_some_and(|token| token.text == b"*")
    {
        entity_start += 1;
        if tokens
            .get(entity_start)
            .is_some_and(|token| token.kind == TokenKind::Number)
        {
            entity_start += 1;
        }
    } else if tokens
        .get(entity_start)
        .is_some_and(|token| token.kind == TokenKind::LParen)
    {
        let Some(close) = matching_close(tokens, entity_start) else {
            return false;
        };
        entity_start = close + 1;
    }
    if !tokens
        .get(entity_start)
        .is_some_and(|token| token.kind == TokenKind::Name)
    {
        return false;
    }
    let mut item_start = entity_start;
    for (position, token) in tokens.iter().enumerate().take(index).skip(entity_start) {
        if token.kind == TokenKind::Comma && token.depth == 0 {
            item_start = position + 1;
        }
        if token.kind == TokenKind::Operator && token.text == b"=" && token.depth == 0 {
            return false;
        }
    }
    index == item_start
}

pub(super) fn is_named_parameter_token(tokens: &[crate::source::Token<'_>], index: usize) -> bool {
    index >= 2
        && tokens[index - 1].kind == TokenKind::Name
        && (tokens[index - 2].kind == TokenKind::LParen
            || (tokens[index - 2].kind == TokenKind::Comma && tokens[index - 2].depth > 0))
}

/// A `DATA` statement's slashes delimit its value lists — `DATA EIGHT/8.0D0/`
/// — and a data-stmt-constant is a literal, not an expression, so no top-level
/// slash in one is a division.
///
/// Fortran keywords are not reserved, so the leading spelling is not enough:
/// `data = a/b` and `data(i) = a/b` are assignments to a variable that happens
/// to be called `data`, and their slashes are ordinary divisions. What
/// separates them is the assignment itself. A `DATA` statement is a list of
/// objects and slash-delimited values; the only `=` it can contain belongs to
/// an implied-do control, which is inside the parentheses of the implied-do.
/// So a depth-zero `=` — or `=>`, for a pointer assignment — means the
/// statement is an assignment, whatever its first word says.
pub(super) fn is_data_statement(tokens: &[crate::source::Token<'_>]) -> bool {
    let first = first_statement_index(tokens);
    if !tokens
        .get(first)
        .is_some_and(|token| token.is_name(b"data"))
    {
        return false;
    }
    !tokens.iter().skip(first + 1).any(is_top_level_assignment)
}

/// Whether `token` is the `=` or `=>` of an assignment rather than part of a
/// larger operator: `==`, `/=`, `<=` and `>=` are comparisons, and anything
/// inside parentheses belongs to an argument or an implied-do control.
fn is_top_level_assignment(token: &crate::source::Token<'_>) -> bool {
    token.depth == 0
        && token.kind == TokenKind::Operator
        && (token.text == b"=" || token.text == b"=>")
}

/// The statement's I/O keyword, when it opens the statement or follows an
/// `IF (...)` condition on the same token list.
pub(super) fn io_statement_head(tokens: &[crate::source::Token<'_>]) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(io, candidate)| {
        if !(candidate.is_name(b"print")
            || candidate.is_name(b"read")
            || candidate.is_name(b"write"))
        {
            return None;
        }
        let first = first_statement_index(tokens);
        let is_head =
            io == first || if_condition_close(tokens).is_some_and(|close| io == close + 1);
        is_head.then_some(io)
    })
}

fn is_io_keyword(token: &crate::source::Token<'_>) -> bool {
    token.is_name(b"print") || token.is_name(b"read") || token.is_name(b"write")
}

fn is_io_specifier_star(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    token: &crate::source::Token<'_>,
    context: &super::LineContext<'_>,
) -> bool {
    if token.text != b"*" {
        return false;
    }
    let Some(io) = io_statement_head(tokens) else {
        // `IF (...) PRINT *, …` is one statement, and the wrapper breaks it
        // inside the condition: the continuation line then opens mid-condition
        // with no head in sight, and this line's `*` read as a multiplication.
        // Pass one wrote `print *` and pass two compacted it to `print*`
        // (WRF `module_sf_ruclsm.F`), so the head has to be asked of the
        // statement rather than of the line.
        return context.io_statement
            && index
                .checked_sub(1)
                .and_then(|previous| tokens.get(previous))
                .is_some_and(is_io_keyword);
    };
    let open = io + 1;
    if tokens
        .get(open)
        .is_some_and(|next| next.kind == TokenKind::LParen)
    {
        let Some(close) = matching_close(tokens, open) else {
            return false;
        };
        return index > open
            && index < close
            && matches!(tokens[index - 1].kind, TokenKind::LParen | TokenKind::Comma);
    }
    index == io + 1
}

pub(super) fn is_continued_named_parameter(
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

pub(super) fn inside_paren_at(
    line: &[u8],
    open_groups: &[bool],
    tokens: &[crate::source::Token<'_>],
) -> Vec<bool> {
    let mut open = open_groups.to_vec();
    let mut result = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen if opens_array_constructor(line, tokens, index) => {
                result.push(open.last().copied().unwrap_or(false));
                open.push(false);
            }
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

/// Where in `line` the exponent marker of the real literal at `index` sits —
/// including the spelling this pass has not finished assembling.
///
/// [`real_exponent_marker`] answers for a literal that is already one token. But
/// `1D+ 1` is one literal with a blank in it, and this same pass closes that
/// blank: the operator rules read the `+` as an exponent sign (`exponent_before`)
/// and decline to space it, and whitespace reduction then pulls the digits up
/// against it. Split, the literal is not a number token at all -- it lexes as
/// `1`, the *name* `D`, `+`, `1` -- so the case rule had nothing to case and
/// waited for the token the blank's closing left behind. That is one run too
/// late, and `a = 1D+ 1` needed two to settle. Both halves now decide from
/// `exponent_before`, so neither can see an exponent the other does not.
fn exponent_marker(
    line: &[u8],
    tokens: &[crate::source::Token<'_>],
    index: usize,
) -> Option<usize> {
    let token = &tokens[index];
    if let Some(marker) = real_exponent_marker(token.text) {
        return Some(token.span.start + marker);
    }
    // `1` `D` `+` `1`, each token hard against the last. The gap the pass is
    // about to close is the one *after* the sign, so only these joins are
    // required here; `exponent_before` supplies the rest of the shape, which is
    // the marker letter and a digit or `.` before it.
    let mut marker_index = index + 1;
    // The mantissa is not always one token: a trailing `.` with no digits after
    // it is not part of the number, so `1.E-` lexes as `1` `.` `E` `-`. Only a
    // `.` may intervene. Anything else -- a `.and.` between the number and a
    // name, say -- is not a literal being assembled, and casing its `E` would
    // rename an identifier rather than spell a marker.
    if tokens
        .get(marker_index)
        .is_some_and(|dot| dot.text == b"." && dot.span.start == token.span.end)
    {
        marker_index += 1;
    }
    let marker = tokens.get(marker_index)?;
    let sign = tokens.get(marker_index + 1)?;
    let digits = tokens.get(marker_index + 2)?;
    if marker.kind != TokenKind::Name
        || marker.text.len() != 1
        || marker.span.start != tokens[marker_index - 1].span.end
        || !matches!(sign.text, b"+" | b"-")
        || sign.span.start != marker.span.end
        || !exponent_before(line, sign.span.start)
    {
        return None;
    }
    // The exponent's own digits, which `real_exponent_marker` demands of the
    // joined spelling too. Without them there is no exponent and no blank to
    // close: `1D+ x` is a `D`-suffixed literal plus a name, and stays as written.
    if digits.kind != TokenKind::Number || !digits.text.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some(marker.span.start)
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

/// Spell `bytes` according to the configured keyword case.
///
/// `Preserve` is a real setting, not a default: a caller that rewrites keyword
/// spelling must route it through here rather than hard-coding a case.
pub(crate) fn apply_case(bytes: &[u8], case: KeywordCase) -> Vec<u8> {
    match case {
        KeywordCase::Lower => bytes.to_ascii_lowercase(),
        KeywordCase::Upper => bytes.to_ascii_uppercase(),
        KeywordCase::Preserve => bytes.to_vec(),
    }
}

/// Case a split compound keyword one word at a time.
///
/// A word that names something the file declares is not this rule's to case:
/// Wannier90's `berry.F90` has an `INTEGER :: if`, so the declared-case pass
/// settles every free-standing `if` in it as `if`. Casing the whole `ELSE IF`
/// replacement wrote `ELSE IF` on the first run and left the declared-case pass
/// to rewrite it as `ELSE if` on the second.
pub(super) fn case_compound_words(
    replacement: &[u8],
    case: KeywordCase,
    declared_names: &DeclaredNameIndex,
    line: usize,
) -> Vec<u8> {
    // An END statement is the exception the declared-case pass itself makes:
    // it leaves `END IF` alone in the same file where it rewrites `ELSE IF`,
    // so keyword case owns every word of an `end …` split outright.
    let end_statement = replacement
        .split(|byte| *byte == b' ')
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case(b"end"));
    let mut out = Vec::with_capacity(replacement.len());
    for (index, word) in replacement.split(|byte| *byte == b' ').enumerate() {
        if index > 0 {
            out.push(b' ');
        }
        if !end_statement && declared_names.suppresses_keyword(line, word, false) {
            out.extend_from_slice(declared_word_spelling(declared_names, line, word));
        } else {
            out.extend_from_slice(&apply_case(word, case));
        }
    }
    out
}

/// The spelling the declared-case pass gives `word`, so the split lands on that
/// pass's answer rather than one run ahead of it.
fn declared_word_spelling<'a>(
    declared_names: &'a DeclaredNameIndex,
    line: usize,
    word: &'a [u8],
) -> &'a [u8] {
    for declared in [
        declared_names.governing_local_case(line, word),
        declared_names.file_declared_case(line, word),
    ] {
        if let DeclaredSpelling::Spelling(spelling) = declared {
            return spelling;
        }
    }
    word
}

fn compound_spelling(source: &[u8], canonical: &str) -> Vec<u8> {
    let first_len = canonical
        .split_once(' ')
        .map_or(canonical.len(), |(first, _)| first.len());
    if source.len() < first_len {
        return canonical.as_bytes().to_vec();
    }
    let mut result = source[..first_len].to_vec();
    result.push(b' ');
    result.extend_from_slice(&source[first_len..]);
    result
}

fn dotted_case(token: &[u8], case: KeywordCase) -> Vec<u8> {
    let mut result = token.to_vec();
    if result.len() > 2 {
        let interior = apply_case(&result[1..result.len() - 1], case);
        result.splice(1..result.len() - 1, interior);
    }
    result
}

fn dotted_word_case(token: &[u8], case: KeywordCase) -> Option<Vec<u8>> {
    let word = token.strip_prefix(b".")?.strip_suffix(b".")?;
    if word.is_empty() {
        return None;
    }
    let canonical = word.to_ascii_lowercase();
    if !vocab::contains(vocab::INTRINSIC_NAMES, &canonical) {
        return None;
    }
    let mut out = Vec::with_capacity(token.len());
    out.push(b'.');
    out.extend_from_slice(&apply_case(word, case));
    out.push(b'.');
    Some(out)
}

fn is_spaced_dotted_operator(token: &[u8]) -> bool {
    [b".and.".as_slice(), b".or.", b".not.", b".eqv.", b".neqv."]
        .iter()
        .any(|operator| token.eq_ignore_ascii_case(operator))
}

/// Whether a relational operator token is written with a space on each side.
///
/// A single-character operator glued to another operator byte is left alone: the
/// pair is more likely to be one thing this rule does not model than two things
/// it does, and `a<<b` is better left as written than pulled apart a character at
/// a time.
///
/// That guard reads the bytes beside the token, which is the whitespace the
/// *previous* run of this pass wrote. It is therefore only stable while the pass
/// leaves those bytes alone, and `run_separates` is how the caller says it will
/// not: when any other operator in the same glued run is being spaced, the run is
/// coming apart on this pass whatever this token decides, and a token that
/// declined would be spaced by the next run once its neighbour had moved away.
/// See [`super::case::operator_verdicts`], which computes the flag.
fn is_spaced_operator_token(
    line: &[u8],
    token: &crate::source::Token<'_>,
    run_separates: bool,
) -> bool {
    let start = token.span.start;
    let end = token.span.end;
    let unglued = |before: &[u8], after: &[u8]| {
        run_separates
            || ((start == 0 || !before.contains(&line[start - 1]))
                && (end == line.len() || !after.contains(&line[end])))
    };
    match token.text {
        b"=>" | b"==" | b"/=" | b"<=" | b">=" => true,
        b"<" => unglued(b"=<>", b"<>"),
        b">" => unglued(b"=<>-", b"<>"),
        b"=" => unglued(b"<>=/", b"=>"),
        _ => false,
    }
}

fn is_arithmetic_operator(operator: &[u8]) -> bool {
    matches!(operator, b"+" | b"-" | b"*" | b"/" | b"**" | b"//")
}

fn binary_operator_spaced(operator: &[u8], compact_multiplicative: bool) -> bool {
    !(compact_multiplicative && matches!(operator, b"*" | b"/" | b"**"))
}

fn is_declaration_type_star(
    tokens: &[crate::source::Token<'_>],
    index: usize,
    operator: &[u8],
) -> bool {
    operator == b"*"
        && index > 0
        && index - 1 == first_statement_index(tokens)
        && tokens[index - 1].kind == TokenKind::Name
        && matches!(
            tokens[index - 1].text.to_ascii_lowercase().as_slice(),
            b"character" | b"integer" | b"real" | b"complex" | b"logical"
        )
        && tokens[index].depth == 0
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

/// Whether the `+` or `-` at `index` is a real literal's exponent sign, and so
/// belongs to the literal rather than to the expression around it.
fn exponent_before(line: &[u8], index: usize) -> bool {
    if index < 2 || !matches!(line[index - 1], b'e' | b'E' | b'd' | b'D') {
        return false;
    }
    if line[index - 2].is_ascii_digit() {
        return true;
    }
    // A mantissa may end in `.`, but the literal still needs a digit in it:
    // `1.E-3` is a real and `.E-3` is not. The `.` that closes a dotted operator
    // is not a mantissa at all -- `1.and.E-3` is `.and.` between a number and
    // the name `E` -- and reading it as one made the sign an exponent's while
    // the operator was still glued and an ordinary subtraction once it had been
    // spaced, so the line took two runs to settle.
    line[index - 2] == b'.' && index >= 3 && line[index - 3].is_ascii_digit()
}

#[derive(Default)]
struct OperatorSpacing {
    previous_end: Option<usize>,
    previous_trailing_space: bool,
    previous_compact_named: bool,
    /// What the last operator was written as, for [`join_preserves_boundary`].
    previous_operator: Vec<u8>,
}

fn add_operator_edit(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    operator: &[u8],
    spaced: bool,
    next: Option<&[u8]>,
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
    // The blank after a compact operator is this edit's to swallow, and
    // swallowing it must not spell a third token: `a/ /b` became `a//b`, which
    // the next run lexes as one `//` and spaces. Leave the blank alone.
    if !spaced && !join_preserves_boundary(operator, next.unwrap_or_default()) {
        right = token.span.end;
    }
    let abuts_previous = spacing.previous_end == Some(left);
    let suppress_leading_space =
        abuts_previous && (spacing.previous_trailing_space || spacing.previous_compact_named);
    // The same rule on the other side of the gap: closing up to the operator
    // *before* this one must not spell a third either. `f(a= = 1)` was written
    // `f(a== 1)`, because the keyword argument's `=` is compact and this edit
    // reaches back over the blank behind it, and the run after that lexes `==`
    // as one operator and spaces it. A space is the smallest thing that keeps
    // the two tokens two tokens.
    let would_join = abuts_previous
        && !spacing.previous_trailing_space
        && !join_preserves_boundary(&spacing.previous_operator, operator);
    let mut replacement = Vec::new();
    if left == 0 {
        replacement.extend_from_slice(&line[..token.span.start]);
    }
    if (spaced && left > 0 && !suppress_leading_space) || would_join {
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
    spacing.previous_operator = operator.to_vec();
    edits.replace(left..right, &replacement);
}

fn remove_operator_trailing_whitespace(
    line: &[u8],
    edits: &mut EditBuffer<'_>,
    token: &crate::source::Token<'_>,
    next: Option<&[u8]>,
    spacing: &mut OperatorSpacing,
) {
    let mut end = token.span.end;
    while end < line.len() && line[end].is_ascii_whitespace() {
        end += 1;
    }
    // As in `add_operator_edit`: the blank this would remove is the only thing
    // keeping two tokens from being read as one.
    if !join_preserves_boundary(token.text, next.unwrap_or_default()) {
        end = token.span.end;
    }
    if end > token.span.end && !is_trailing_continuation_marker(line, token.span.end) {
        edits.replace(token.span.end..end, b"");
        spacing.previous_end = Some(end);
    } else {
        spacing.previous_end = Some(token.span.end);
    }
    spacing.previous_trailing_space = false;
    spacing.previous_compact_named = false;
    spacing.previous_operator = token.text.to_vec();
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
mod declaration_head_tests {
    use super::is_declaration_statement;
    use crate::source::tokens::tokens;

    #[test]
    fn continued_declarations_share_type_head_recognition() {
        for source in [
            b"INTEGER*1 i".as_slice(),
            b"DOUBLE PRECISION x",
            b"DOUBLEPRECISION x",
            b"TYPEOF(x) y",
            b"CLASSOF(x) y",
            b"DOUBLE COMPLEX z",
        ] {
            assert!(is_declaration_statement(&tokens(source)), "{source:?}");
        }
    }
}
