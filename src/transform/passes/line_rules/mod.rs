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
//! [`run`] applies the physical-line chain to the document, while
//! [`respace_joined`] deliberately enables only rules 1, 2 and 4 for a
//! statement the wrapper has rejoined. Both paths go through the same internal
//! sequencer so the order has one definition. Canonicalization-only follows the
//! same sequence but lets rules 1-2 make spelling edits without enabling the
//! whitespace-only portions of the chain.

mod common;

pub use common::{
    case::lowercase_line, comment_spacing::normalize_comment_spacing,
    delimiter_spacing::normalize_delimiter_spacing, is_protected,
    keyword_spacing::normalize_keyword_spacing, write_spacing::normalize_write_output_spacing,
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

/// Immutable facts about the line currently passing through the rule chain.
///
/// These are deliberately separated from [`LineState`]: a stage may inspect
/// context, but only `run` advances continuation state between physical lines.
#[derive(Clone, Copy, Default)]
struct LineContext<'a> {
    preserve_comment_after: bool,
    continued_statement: bool,
    continued_infix: bool,
    continued_declaration: bool,
    continued_named_parameter: bool,
    continued_bind_parameter: bool,
    open_groups: &'a [bool],
    multiple_subscript_depths: &'a [usize],
    continued_format: bool,
    continued_initializer: bool,
    /// The statement so far carried a top-level `::`, so this line continues
    /// an entity list even when the head's type token is unrecognizable — a
    /// preprocessor template expansion, say.
    continued_separator: bool,
    /// The previous line ended on `%`, so this line opens with a component
    /// name that no token on the line itself identifies as one.
    continued_component: bool,
}

/// State carried from one physical line to the next while step 11 runs.
#[derive(Default)]
struct LineState {
    lex: LexState,
    /// The conditional-compilation stream's own lexical state.
    ///
    /// A statement continues only within its own sentinel stream, so the two
    /// carry literal and Hollerith state independently: consecutive `!$ ` lines
    /// splice with each other, and an ordinary literal spans an intervening
    /// `!$ ` line because that line is a comment under the only reading of the
    /// source that compiles.
    omp_lex: LexState,
    continued_statement: bool,
    continued_infix: bool,
    continued_openmp_infix: bool,
    continued_named_parameter: bool,
    continued_bind_parameter: bool,
    continued_component: bool,
    open_groups: Vec<bool>,
    multiple_subscript_depths: Vec<usize>,
    entity_list: EntityListCursor,
}

impl LineState {
    fn context<'a>(
        &self,
        open_groups: &'a [bool],
        multiple_subscript_depths: &'a [usize],
        document: &Document,
        index: usize,
        cx: &PassContext,
    ) -> LineContext<'a> {
        let preserve_comment_after =
            common::comment_spacing::preserve_full_comment_spacing(document, index, cx);
        let first_statement_tokens = || {
            cx.analysis
                .group_of_line(index)
                .and_then(|group| group.statements.first())
                .map(|statement| crate::source::tokens::tokens(&statement.text))
        };
        let continued_declaration = self.continued_statement
            && first_statement_tokens()
                .is_some_and(|tokens| common::is_declaration_statement(&tokens));
        let continued_format = self.continued_statement
            && first_statement_tokens().is_some_and(|tokens| common::is_format_statement(&tokens));

        LineContext {
            preserve_comment_after,
            continued_statement: self.continued_statement,
            continued_infix: self.continued_infix,
            continued_declaration,
            continued_named_parameter: self.continued_named_parameter,
            continued_bind_parameter: self.continued_bind_parameter,
            open_groups,
            multiple_subscript_depths,
            continued_format,
            continued_initializer: self.entity_list.initializer,
            continued_separator: self.continued_statement && self.entity_list.separator,
            continued_component: self.continued_component,
        }
    }

    fn reset_statement(&mut self) {
        self.lex = LexState::default();
        // `omp_lex` is deliberately untouched: it belongs to the other stream,
        // which an ordinary statement boundary does not end.
        self.continued_statement = false;
        self.continued_infix = false;
        self.continued_named_parameter = false;
        self.continued_bind_parameter = false;
        self.continued_component = false;
        self.open_groups.clear();
        self.multiple_subscript_depths.clear();
        self.entity_list = EntityListCursor::default();
    }

    fn advance(&mut self, code: &[u8], incoming: LexState, cx: &PassContext, line_index: usize) {
        self.continued_statement = trailing_ampersand(code);
        self.continued_infix = trailing_continuation_operand(code);
        self.continued_named_parameter = self.continued_statement && is_call_group(cx, line_index);
        self.continued_bind_parameter = self.continued_statement && is_bind_group(cx, line_index);
        self.continued_component =
            self.continued_statement && common::trailing_component_selector(code);
        common::delimiter_spacing::advance_multiple_subscript_depths(
            code,
            incoming,
            self.open_groups.len(),
            &mut self.multiple_subscript_depths,
        );
        self.entity_list
            .advance(code, self.open_groups.len(), incoming);
        fold_open_groups(code, &mut self.open_groups, incoming);
        if !self.continued_statement {
            self.open_groups.clear();
            self.multiple_subscript_depths.clear();
            self.entity_list = EntityListCursor::default();
        }
    }
}

#[derive(Clone, Copy)]
enum RuleMode<'a> {
    Physical(LineContext<'a>),
    Rejoined,
}

/// Apply the whole chain to every line of the document.
pub fn run(document: &mut Document, cx: &PassContext) -> Result<Changed, FormatError> {
    let mut changed = Changed::No;
    let declared_names = scoped_declared_names(cx.analysis, cx.scopes);
    let mut state = LineState::default();

    for index in 0..document.lines.len() {
        let kind = cx
            .analysis
            .buffer
            .lines
            .get(index)
            .map(|line| line.kind)
            .unwrap_or(PhysicalLineKind::Code);

        if let Some(body_start) = openmp_clause_body_start(&document.lines[index]) {
            let context = LineContext {
                continued_infix: state.continued_openmp_infix,
                ..LineContext::default()
            };
            let body = apply_rules(
                &document.lines[index][body_start..],
                cx,
                &declared_names,
                index,
                &mut state.omp_lex,
                RuleMode::Physical(context),
            );
            let mut rebuilt = document.lines[index][..body_start].to_vec();
            rebuilt.extend_from_slice(&body);
            if rebuilt != document.lines[index] {
                document.lines[index] = rebuilt;
                changed = changed.or(Changed::Text);
            }
            state.continued_openmp_infix = trailing_continuation_operand(&body);
            // The ordinary stream steps over this line, so its lexical state
            // survives; only the ordinary statement context ends here.
            let lex = state.lex;
            state.reset_statement();
            state.lex = lex;
            continue;
        }

        // A normal physical line cannot continue an expression through a
        // conditional-compilation sentinel; only adjacent `!$` body lines
        // share this state.
        state.continued_openmp_infix = false;
        if kind == PhysicalLineKind::Preprocessor {
            // A directive is stepped over by a continued statement, so it
            // cannot close a character literal the previous line left open;
            // only the statement context goes.
            let lex = state.lex;
            state.reset_statement();
            state.lex = lex;
            continue;
        }

        let multiple_subscript_depths = state.multiple_subscript_depths.clone();
        let context = state.context(
            &state.open_groups,
            &multiple_subscript_depths,
            document,
            index,
            cx,
        );
        // A comment or blank line is stepped over too. It is still normalized
        // on its own terms, but through a scratch state: reading the group's
        // state would make the `!` of a comment inside an open literal look
        // like literal text, and writing it back would let the apostrophe in
        // prose like `! don't` close the literal, so the `!` in `&def!ghi'` on
        // the resumed line would be rewritten as a comment marker.
        let incoming_lex = state.lex;
        let mut scratch = LexState::default();
        let lex = if matches!(kind, PhysicalLineKind::Comment | PhysicalLineKind::Blank) {
            &mut scratch
        } else {
            &mut state.lex
        };
        let line = apply_rules(
            &document.lines[index],
            cx,
            &declared_names,
            index,
            lex,
            RuleMode::Physical(context),
        );

        if let Some(physical) = cx.analysis.buffer.lines.get(index) {
            if matches!(
                physical.kind,
                PhysicalLineKind::Code | PhysicalLineKind::FindentFix
            ) {
                state.advance(
                    cx.analysis.buffer.code_bytes(physical),
                    incoming_lex,
                    cx,
                    index,
                );
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
        || tokens.iter().enumerate().any(|(index, token)| {
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
    let context = LineContext::default();
    apply_rules(
        line,
        cx,
        declared_names,
        line_index,
        state,
        RuleMode::Physical(context),
    )
}

/// The one definition of the line-rule sequence.
///
/// `RuleMode` controls which stages are enabled, but never their relative
/// order. A physical line gets all five stages; a rejoined statement gets the
/// deliberate 1/2/4 subset and then the joined named-argument cleanup.
fn apply_rules(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    state: &mut LexState,
    mode: RuleMode<'_>,
) -> Vec<u8> {
    let incoming = *state;
    let context = match mode {
        RuleMode::Physical(context) => context,
        RuleMode::Rejoined => LineContext::default(),
    };
    let physical = matches!(mode, RuleMode::Physical(_));
    let normalize_whitespace = cx.config.style.normalize_whitespace;

    // The stage order is an architectural invariant. Do not permute it.
    // 1. Case/operator normalization. The case rule itself distinguishes
    // token replacement from operator-spacing edits by style.
    let mut text = common::case::lowercase_line_with_context(
        line,
        cx,
        declared_names,
        line_index,
        state,
        &context,
    );
    // 2. Keyword spelling and, when enabled, layout spacing.
    text = common::keyword_spacing::normalize_keyword_spacing_with_state(
        &text,
        declared_names,
        line_index,
        incoming,
        context.continued_format,
        normalize_whitespace,
        &cx.config.style,
    );
    // 3. WRITE output spacing (physical lines only).
    if normalize_whitespace && physical && cx.config.style.delimiter_spacing {
        text =
            common::write_spacing::normalize_write_output_spacing_with_state(&text, cx, incoming);
    }
    // 4. Delimiter spacing.
    if normalize_whitespace {
        text = common::delimiter_spacing::normalize_delimiter_spacing_with_context(
            &text,
            cx,
            incoming,
            context.continued_statement,
            context.open_groups.len(),
            context.multiple_subscript_depths,
        );
    }
    // 5. Comment spacing (physical lines only).
    if normalize_whitespace && physical {
        text = common::comment_spacing::normalize_comment_spacing_with_state(
            &text,
            cx,
            incoming,
            context.preserve_comment_after,
            common::comment_spacing::code_span_len(&text) as isize
                - common::comment_spacing::code_span_len(line) as isize,
        );
    }

    match mode {
        RuleMode::Physical(context) => {
            if normalize_whitespace
                && context.continued_statement
                && context.continued_named_parameter
            {
                compact_continued_named_argument(&text, context.open_groups)
            } else {
                text
            }
        }
        // Rejoined statements only exist inside full-mode wrapping, where the
        // joined named-argument spelling is part of the wrapper's established
        // canonical output.
        RuleMode::Rejoined => compact_joined_named_arguments(&text),
    }
}

/// Rules 1, 2 and 4 for a statement the wrapper has just joined.
pub fn respace_joined(
    line: &[u8],
    cx: &PassContext,
    declared_names: &DeclaredNameIndex,
    line_index: usize,
) -> Vec<u8> {
    apply_rules(
        line,
        cx,
        declared_names,
        line_index,
        &mut LexState::default(),
        RuleMode::Rejoined,
    )
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
    fn advance(&mut self, line: &[u8], depth: usize, incoming: LexState) {
        let mut state = incoming;
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

fn fold_open_groups(line: &[u8], open: &mut Vec<bool>, incoming: LexState) {
    let mut state = incoming;
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

include!("tests.rs");

#[cfg(test)]
mod rejoined_tests {
    use crate::{
        analysis::{analyze_file, scoped_declared_names, ProjectContext, ScopeTree},
        config::FormatConfig,
        transform::{document::Document, pipeline::PassContext},
    };

    #[test]
    fn rejoined_component_names_keep_authored_case() {
        let source = b"X=State%Data\n";
        let document = Document::from_bytes(source);
        let mut project = ProjectContext::empty();
        project.enable_target_local_component_resolution();
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
        let declared_names = scoped_declared_names(&analysis, &scopes);

        assert_eq!(
            super::respace_joined(b"X=State%Data", &context, &declared_names, 0),
            b"X = State%Data"
        );
    }
}
