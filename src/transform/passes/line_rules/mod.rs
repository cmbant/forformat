//! Step 11: the per-line rule chain.
//!
//! Normalization order, which must not be permuted:
//!
//! 1. `lowercase_line` — keyword case, operator modernization, real exponent
//!    markers, project case application;
//! 2. `normalize_keyword_spacing` — compound keywords, `keyword(`, `) then`;
//! 3. `normalize_write_output_spacing`;
//! 4. `normalize_delimiter_spacing`;
//! 5. `normalize_comment_spacing`.
//!
//! The chain is exposed twice on purpose. [`run`] applies it to the document,
//! and [`respace_joined`] applies rules 1, 2 and 4 to a statement the wrapper
//! has just rejoined.

mod common;

pub use common::{
    case::lowercase_line,
    comment_spacing::normalize_comment_spacing,
    delimiter_spacing::normalize_delimiter_spacing,
    is_protected,
    keyword_spacing::normalize_keyword_spacing,
    write_spacing::normalize_write_output_spacing,
};
pub(crate) use common::{
    case::is_end_construct_keyword,
    comment_spacing::is_directive_comment,
};

use crate::{
    analysis::{scoped_declared_names, DeclaredNameIndex},
    error::FormatError,
    source::{
        regions::LexState,
        tokens::{tokenize, TokenKind},
        PhysicalLineKind,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        pipeline::{Changed, PassContext},
    },
};

#[derive(Clone, Copy, Default)]
struct LineOptions<'a> {
    preserve_comment_after: bool,
    continued_statement: bool,
    continued_infix: bool,
    continued_declaration: bool,
    continued_named_parameter: bool,
    continued_bind_parameter: bool,
    open_groups: &'a [bool],
    continued_format: bool,
    continued_initializer: bool,
}

/// Apply the whole chain to every line of the document.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let mut changed = Changed::No;
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let mut state = LexState::default();
    let mut continued_statement = false;
    let mut continued_infix = false;
    let mut continued_openmp_infix = false;
    let mut continued_named_parameter = false;
    let mut continued_bind_parameter = false;
    let mut open_groups: Vec<bool> = Vec::new();
    let mut entity_list = EntityListCursor::default();

    for index in 0..document.lines.len() {
        let kind = cx
            .analysis
            .buffer
            .lines
            .get(index)
            .map(|line| line.kind)
            .unwrap_or(PhysicalLineKind::Code);

        if let Some(body_start) = openmp_clause_body_start(&document.lines[index]) {
            let body = apply_with_options(
                &document.lines[index][body_start..],
                cx,
                &declared_names,
                index,
                &mut LexState::default(),
                LineOptions {
                    continued_infix: continued_openmp_infix,
                    ..LineOptions::default()
                },
            );
            let mut rebuilt = document.lines[index][..body_start].to_vec();
            rebuilt.extend_from_slice(&body);
            if rebuilt != document.lines[index] {
                document.lines[index] = rebuilt;
                changed = changed.or(Changed::Text);
            }
            continued_openmp_infix = trailing_continuation_operand(&body);
            state = LexState::default();
            continued_statement = false;
            continued_infix = false;
            continued_named_parameter = false;
            continued_bind_parameter = false;
            open_groups.clear();
            entity_list = EntityListCursor::default();
            continue;
        }

        continued_openmp_infix = false;
        if kind == PhysicalLineKind::Preprocessor {
            state = LexState::default();
            continued_statement = false;
            continued_infix = false;
            continued_named_parameter = false;
            continued_bind_parameter = false;
            open_groups.clear();
            entity_list = EntityListCursor::default();
            continue;
        }

        let preserve_comment_after =
            common::comment_spacing::preserve_full_comment_spacing(document, index, cx);
        let first_statement_tokens = || {
            cx.analysis
                .group_of_line(index)
                .and_then(|group| group.statements.first())
                .map(|statement| crate::source::tokens::tokens(&statement.text))
        };
        let continued_declaration = continued_statement
            && first_statement_tokens()
                .is_some_and(|tokens| common::is_declaration_statement(&tokens));
        let continued_format = continued_statement
            && first_statement_tokens().is_some_and(|tokens| common::is_format_statement(&tokens));

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
                continued_bind_parameter,
                continued_format,
                continued_initializer: entity_list.initializer,
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
                continued_named_parameter = continued_statement && is_call_group(cx, index);
                continued_bind_parameter = continued_statement && is_bind_group(cx, index);
                entity_list.advance(code, open_groups.len());
                fold_open_groups(code, &mut open_groups);
                if !continued_statement {
                    open_groups.clear();
                    entity_list = EntityListCursor::default();
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
            .any(|(index, token)| {
                token.text == b"=" && common::is_named_parameter_token(&tokens, index)
            })
}

fn is_bind_group(cx: &PassContext, line_index: usize) -> bool {
    let Some(statement) = cx
        .analysis
        .group_of_line(line_index)
        .and_then(|group| group.statements.first())
    else {
        return false;
    };
    let tokens = crate::source::tokens::tokens(&statement.text);
    tokens.iter().enumerate().any(|(index, token)| {
        token.is_name(b"bind")
            && tokens
                .get(index + 1)
                .is_some_and(|next| next.kind == TokenKind::LParen)
    })
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
    options: LineOptions<'_>,
) -> Vec<u8> {
    let incoming = *state;

    // The stage order is an architectural invariant. Do not permute it.
    // 1. Case/operator normalization.
    let mut text = common::case::lowercase_line_with_context(
        line,
        cx,
        declared_names,
        line_index,
        state,
        options.continued_statement,
        options.continued_infix,
        options.continued_declaration,
        options.continued_named_parameter,
        options.continued_bind_parameter,
        options.continued_format,
        options.continued_initializer,
        options.open_groups,
        false,
    );
    // 2. Keyword/layout spacing.
    text = common::keyword_spacing::normalize_keyword_spacing_with_state(
        &text,
        declared_names,
        line_index,
        incoming,
        options.continued_format,
        &cx.config.style,
    );
    // 3. WRITE output spacing.
    if cx.config.style.delimiter_spacing {
        text = common::write_spacing::normalize_write_output_spacing_with_state(&text, cx, incoming);
    }
    // 4. Delimiter spacing.
    text = common::delimiter_spacing::normalize_delimiter_spacing_with_state(
        &text,
        cx,
        incoming,
        options.continued_statement,
    );
    // 5. Comment spacing.
    let mut text = common::comment_spacing::normalize_comment_spacing_with_state(
        &text,
        cx,
        incoming,
        options.preserve_comment_after,
        common::comment_spacing::code_span_len(&text) as isize
            - common::comment_spacing::code_span_len(line) as isize,
    );

    if options.continued_statement && options.continued_named_parameter {
        text = compact_continued_named_argument(&text, options.open_groups);
    }
    text
}

/// Rules 1, 2 and 4 for a statement the wrapper has just joined.
pub fn respace_joined(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
) -> Vec<u8> {
    let mut state = LexState::default();
    // Keep this sequence explicit for the same reason as `apply_with_options`.
    let mut text = common::case::lowercase_line_with_context(
        line,
        cx,
        declared_names,
        line_index,
        &mut state,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        &[],
        cx.project.target_local_component_resolution,
    );
    text = common::keyword_spacing::normalize_keyword_spacing_with_state(
        &text,
        declared_names,
        line_index,
        LexState::default(),
        false,
        &cx.config.style,
    );
    text = common::delimiter_spacing::normalize_delimiter_spacing_with_state(
        &text,
        cx,
        LexState::default(),
        false,
    );
    compact_joined_named_arguments(&text)
}

fn compact_joined_named_arguments(line: &[u8]) -> Vec<u8> {
    let tokens = tokenize(line, &mut LexState::default());
    let mut edits = EditBuffer::new(line);
    for (index, token) in tokens.iter().enumerate() {
        if token.text != b"=" || !common::is_named_parameter_token(&tokens, index) {
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
    let inside_paren = common::inside_paren_at(open_groups, &tokens);
    let mut edits = EditBuffer::new(line);
    for (index, token) in tokens.iter().enumerate() {
        if token.text != b"="
            || !common::is_continued_named_parameter(&tokens, index, inside_paren[index])
        {
            continue;
        }
        let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
            continue;
        };
        let Some(next) = tokens.get(index + 1) else {
            continue;
        };
        if next.kind == TokenKind::Ampersand {
            continue;
        }
        edits.replace(previous.span.end..next.span.start, b"=");
    }
    edits.finish()
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
    line[previous - 1] == b'.' && !ends_with_dotted_operator(&line[..previous])
}

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

fn openmp_clause_body_start(line: &[u8]) -> Option<usize> {
    let start = line.iter().position(|byte| !matches!(byte, b' ' | b'\t'))?;
    if !line[start..].starts_with(b"!$") {
        return None;
    }
    let body_start = start + 2;
    if line
        .get(body_start)
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t'))
    {
        return None;
    }
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

#[derive(Clone, Copy, Default)]
struct EntityListCursor {
    separator: bool,
    initializer: bool,
}

impl EntityListCursor {
    fn advance(&mut self, line: &[u8], depth: usize) {
        let mut state = LexState::default();
        let mut depth = depth;
        for token in tokenize(line, &mut state) {
            match token.kind {
                TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBracket => depth = depth.saturating_sub(1),
                _ if depth > 0 => {}
                TokenKind::Operator if token.text == b"::" => self.separator = true,
                TokenKind::Operator
                    if self.separator && (token.text == b"=" || token.text == b"=>") =>
                {
                    self.initializer = true;
                }
                TokenKind::Comma if self.separator => self.initializer = false,
                _ => {}
            }
        }
    }
}

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
