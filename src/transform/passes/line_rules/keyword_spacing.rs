use super::*;

use crate::source::Token;

/// Rule 2: keyword and layout spacing.
///
/// This stage owns the statement-level rewrites (array constructors, compound
/// and multiword keywords, `goto`) before the token-local spacing rules.
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
        true,
        &StyleConfig::default(),
    )
}

pub(crate) fn normalize_keyword_spacing_with_state(
    line: &[u8],
    declared_names: &DeclaredNameIndex,
    line_index: usize,
    incoming: LexState,
    continued_format: bool,
    normalize_whitespace: bool,
    style: &StyleConfig,
) -> Vec<u8> {
    let tokens = tokenize(line, &mut incoming.clone());
    let rules = Rules {
        line,
        tokens: &tokens,
        declared_names,
        line_index,
        style,
        normalize_whitespace,
        continued_format,
    };
    let mut edits = EditBuffer::new(line);

    // The order is the rule order and is load-bearing: a later rule replaces a
    // range the earlier one has already rewritten.
    if normalize_whitespace {
        rules.common_block(&mut edits);
    }
    rules.array_constructor_brackets(&mut edits);
    rules.join_goto(&mut edits);
    rules.multiword_keywords(&mut edits);
    rules.split_compound_keywords(&mut edits);
    rules.token_local(&mut edits);
    if normalize_whitespace {
        rules.delimiter_adjacency(&mut edits);
        rules.if_condition_gap(&mut edits);
    }
    rules.strip_empty_args(&mut edits);

    let mut output = edits.finish();
    let start = output
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(output.len());
    if output
        .get(start..)
        .is_some_and(|tail| tail.starts_with(b"else if("))
    {
        // Splitting ELSEIF creates a language-level separator; this is part of
        // the canonical replacement spelling, not a whitespace-only cleanup.
        output.insert(start + b"else if".len(), b' ');
    }
    output
}

/// One physical line's tokens, plus the settings every rule below consults.
///
/// The rules divide in two, and which half a rule is in is the answer to
/// "does canonicalize-only run it?".
///
/// Presentation rules — [`Rules::common_block`], everything gated inside
/// [`Rules::token_local`], [`Rules::delimiter_adjacency`],
/// [`Rules::if_condition_gap`] — only ever move blanks, and are skipped
/// wholesale when the authored spacing is the author's.
///
/// The rest own a *spelling*: `(/` becomes `[`, `go to` becomes `goto`, `end
/// module` gets its one canonical space, a compound keyword splits, `only` is
/// cased, an empty argument list goes away. Those apply in every normalizing
/// mode. Four of them are mixed — they carry a spelling *and* a gap around it —
/// and each says so where it consults [`Rules::normalize_whitespace`].
struct Rules<'a> {
    line: &'a [u8],
    tokens: &'a [Token<'a>],
    declared_names: &'a DeclaredNameIndex,
    line_index: usize,
    style: &'a StyleConfig,
    normalize_whitespace: bool,
    continued_format: bool,
}

impl Rules<'_> {
    /// Is there authored horizontal whitespace between these two offsets?
    fn gap(&self, end: usize, start: usize) -> bool {
        horizontal_gap(self.line, end, start)
    }

    /// Does a declaration on this line shadow this keyword, making it the
    /// user's name rather than the language's word?
    fn shadowed(&self, text: &[u8]) -> bool {
        self.declared_names
            .suppresses_keyword(self.line_index, text, false)
    }

    fn cased(&self, text: &[u8]) -> Vec<u8> {
        apply_case(text, self.style.keyword_case)
    }

    fn common_block(&self, edits: &mut EditBuffer) {
        if let Some((start, end, replacement)) = common_block_edit(self.line, self.tokens) {
            edits.replace(start..end, &replacement);
        }
    }

    /// `(/ ... /)` becomes `[ ... ]`. Mixed: the bracket is the spelling, the
    /// blanks it swallows are presentation.
    fn array_constructor_brackets(&self, edits: &mut EditBuffer) {
        if !self.style.array_brackets || is_format_statement(self.tokens) || self.continued_format {
            return;
        }
        for pair in self.tokens.windows(2) {
            if pair[0].kind == TokenKind::LParen
                && pair[1].kind == TokenKind::Operator
                && pair[1].text == b"/"
                && self.gap(pair[0].span.end, pair[1].span.start)
            {
                let mut end = pair[1].span.end;
                if self.normalize_whitespace {
                    while end < self.line.len() && matches!(self.line[end], b' ' | b'\t') {
                        end += 1;
                    }
                }
                edits.replace(pair[0].span.start..end, b"[");
            }
        }
        for pair in self.tokens.windows(2) {
            if pair[0].kind == TokenKind::Operator
                && pair[0].text == b"/"
                && pair[1].kind == TokenKind::RParen
                && self.gap(pair[0].span.end, pair[1].span.start)
            {
                let mut start = pair[0].span.start;
                if self.normalize_whitespace {
                    while start > 0 && matches!(self.line[start - 1], b' ' | b'\t') {
                        start -= 1;
                    }
                }
                edits.replace(start..pair[1].span.end, b"]");
            }
        }
    }

    /// `go to` becomes `goto`. Mixed: joining the two words is the spelling,
    /// and the space that has to reappear after `)` is presentation.
    fn join_goto(&self, edits: &mut EditBuffer) {
        if !self.style.join_goto {
            return;
        }
        for pair in self.tokens.windows(2) {
            if pair[0].is_name(b"go")
                && pair[1].is_name(b"to")
                && self.gap(pair[0].span.end, pair[1].span.start)
            {
                let mut replacement = self.cased(pair[0].text);
                replacement.extend_from_slice(&self.cased(pair[1].text));
                if self.normalize_whitespace
                    && if_condition_close(self.tokens)
                        .is_some_and(|close| self.tokens[close].span.end == pair[0].span.start)
                {
                    replacement.insert(0, b' ');
                }
                edits.replace(pair[0].span.start..pair[1].span.end, &replacement);
            }
        }
    }

    /// A language-level multiword token has one canonical internal space. This
    /// stays active when presentation whitespace is preserved: `end   module`
    /// and `endmodule` are spelling choices, not incidental layout gaps.
    fn multiword_keywords(&self, edits: &mut EditBuffer) {
        for pair in self.tokens.windows(2) {
            if pair[0].kind == TokenKind::Name
                && pair[1].kind == TokenKind::Name
                && self.gap(pair[0].span.end, pair[1].span.start)
                && is_multiword_keyword_pair(pair[0].text, pair[1].text)
            {
                let mut replacement = self.cased(pair[0].text);
                replacement.push(b' ');
                replacement.extend_from_slice(&self.cased(pair[1].text));
                edits.replace(pair[0].span.start..pair[1].span.end, &replacement);
            }
        }
    }

    /// `endif` becomes `end if`. Mixed: the split is the spelling, the gap
    /// before what follows is presentation — except after `elseif`, where the
    /// separator the split creates is itself part of the spelling.
    fn split_compound_keywords(&self, edits: &mut EditBuffer) {
        if !self.style.split_compound_keywords {
            return;
        }
        let Some(first) = first_statement_token(self.tokens) else {
            return;
        };
        let Some(replacement) = vocab::lookup_pair(vocab::COMPOUND_KEYWORDS, first.text) else {
            return;
        };
        let next = self.tokens.get(first_statement_index(self.tokens) + 1);
        let assignment = next.is_some_and(|token| token.text == b"=");
        if assignment || self.shadowed(first.text) {
            return;
        }
        let mut replacement = compound_spelling(first.text, replacement);
        if self.style.keyword_case != KeywordCase::Preserve {
            replacement = case_compound_words(
                &replacement,
                self.style.keyword_case,
                self.declared_names,
                self.line_index,
            );
        }
        edits.replace(first.span.clone(), &replacement);
        if self.normalize_whitespace {
            if let Some(next) = next {
                if next.kind == TokenKind::Name && self.gap(first.span.end, next.span.start) {
                    edits.replace(first.span.end..next.span.start, b" ");
                }
            }
        }
        if first.is_name(b"elseif") {
            if let Some(paren) = self.tokens.get(first_statement_index(self.tokens) + 1) {
                if paren.kind == TokenKind::LParen {
                    edits.replace(first.span.end..paren.span.start, b" ");
                }
            }
        }
    }

    /// The rules that look at one token and its immediate neighbours.
    fn token_local(&self, edits: &mut EditBuffer) {
        for (index, token) in self.tokens.iter().enumerate() {
            if token.kind == TokenKind::Name {
                if self.normalize_whitespace {
                    self.keyword_then_name_gap(index, token, edits);
                }
                self.only_colon(index, token, edits);
            }
            if self.normalize_whitespace {
                self.name_then_paren_gap(index, token, edits);
                self.select_type_gap(index, token, edits);
                self.rank_or_team_gap(index, token, edits);
            }
        }
    }

    /// One space between a keyword and the name after it: `end module m`,
    /// `do while (...)`, `use m`, `call s`.
    fn keyword_then_name_gap(&self, index: usize, token: &Token<'_>, edits: &mut EditBuffer) {
        if token.is(b"end") && !self.shadowed(token.text) {
            if let Some(next) = self.tokens.get(index + 1) {
                if next.kind == TokenKind::Name && self.gap(token.span.end, next.span.start) {
                    edits.replace(token.span.end..next.span.start, b" ");
                    if let Some(after) = self.tokens.get(index + 2) {
                        if after.kind == TokenKind::Name
                            && self.gap(next.span.end, after.span.start)
                        {
                            edits.replace(next.span.end..after.span.start, b" ");
                        }
                    }
                }
            }
        }
        if token.is(b"do") && !self.shadowed(token.text) {
            if let Some(next) = self.tokens.get(index + 1) {
                if next.kind == TokenKind::Name && self.gap(token.span.end, next.span.start) {
                    edits.replace(token.span.end..next.span.start, b" ");
                    if next.is_name(b"while") {
                        if let Some(paren) = self.tokens.get(index + 2) {
                            if paren.kind == TokenKind::LParen
                                && self.gap(next.span.end, paren.span.start)
                            {
                                edits.replace(next.span.end..paren.span.start, b" ");
                            }
                        }
                    }
                }
            }
        }
        if (token.is(b"module") || token.is(b"use") || token.is(b"call") || token.is(b"subroutine"))
            && !self.shadowed(token.text)
        {
            if let Some(next) = self.tokens.get(index + 1) {
                if next.kind == TokenKind::Name && self.gap(token.span.end, next.span.start) {
                    edits.replace(token.span.end..next.span.start, b" ");
                }
            }
        }
    }

    /// `only :` becomes `only:`. Mixed: the casing is the spelling and applies
    /// either way, the gap before the colon is presentation.
    fn only_colon(&self, index: usize, token: &Token<'_>, edits: &mut EditBuffer) {
        if !token.is(b"only")
            || self.shadowed(token.text)
            || self
                .tokens
                .get(index + 1)
                .is_none_or(|next| next.text != b":")
        {
            return;
        }
        let colon = self.tokens[index + 1].span.start;
        let keyword = self.cased(token.text);
        if self.normalize_whitespace {
            if token.text != keyword || self.gap(token.span.end, colon) {
                edits.replace(token.span.start..colon, &keyword);
            }
        } else if token.text != keyword {
            edits.replace(token.span.clone(), &keyword);
        }
    }

    /// The gap between a name and the `(` after it: none for a statement that
    /// owns its parentheses, one space for `if` and `select`.
    fn name_then_paren_gap(&self, index: usize, token: &Token<'_>, edits: &mut EditBuffer) {
        if token.kind != TokenKind::Name || !is_followed_by_lparen(self.tokens, index) {
            return;
        }
        let next = &self.tokens[index + 1];
        if !self.gap(token.span.end, next.span.start) {
            return;
        }
        let selected_type = index > 0 && self.tokens[index - 1].is_name(b"select");
        let no_space = vocab::contains(vocab::PARENTHESIZED_STATEMENT_NAMES, token.text)
            || token.is(b"dimension")
            || token.is(b"associate")
            || token.is(b"result")
            || (token.is(b"type") && !selected_type)
            || (token.is(b"class") && !selected_type);
        let one_space = token.is(b"if") || token.is(b"select");
        if !self.shadowed(token.text) && (no_space || one_space) {
            edits.replace(
                token.span.end..next.span.start,
                if no_space { b"" } else { b" " },
            );
        }
    }

    /// `select type (x)` and `type is (t)`: one space at each seam.
    fn select_type_gap(&self, index: usize, token: &Token<'_>, edits: &mut EditBuffer) {
        if !token.is(b"select") {
            return;
        }
        if let (Some(ty), Some(paren)) = (self.tokens.get(index + 1), self.tokens.get(index + 2)) {
            if ty.is_name(b"type") && paren.kind == TokenKind::LParen {
                if self.gap(token.span.end, ty.span.start) {
                    edits.replace(token.span.end..ty.span.start, b" ");
                }
                if self.gap(ty.span.end, paren.span.start) {
                    edits.replace(ty.span.end..paren.span.start, b" ");
                }
            }
        }
        if let (Some(ty), Some(is), Some(paren)) = (
            self.tokens.get(index + 1),
            self.tokens.get(index + 2),
            self.tokens.get(index + 3),
        ) {
            if ty.is_name(b"type") && is.is_name(b"is") && paren.kind == TokenKind::LParen {
                if self.gap(token.span.end, ty.span.start) {
                    edits.replace(token.span.end..ty.span.start, b" ");
                }
                if self.gap(ty.span.end, is.span.start) {
                    edits.replace(ty.span.end..is.span.start, b" ");
                }
                if self.gap(is.span.end, paren.span.start) {
                    edits.replace(is.span.end..paren.span.start, b" ");
                }
            }
        }
    }

    /// `select rank (x)`, `change team (t)`, `form team (...)`, `sync team`.
    fn rank_or_team_gap(&self, index: usize, token: &Token<'_>, edits: &mut EditBuffer) {
        if !(token.is(b"change") || token.is(b"form") || token.is(b"select") || token.is(b"sync")) {
            return;
        }
        if let (Some(rank_or_team), Some(paren)) =
            (self.tokens.get(index + 1), self.tokens.get(index + 2))
        {
            if (rank_or_team.is_name(b"rank") || rank_or_team.is_name(b"team"))
                && paren.kind == TokenKind::LParen
                && self.gap(rank_or_team.span.end, paren.span.start)
            {
                edits.replace(rank_or_team.span.end..paren.span.start, b" ");
            }
        }
    }

    /// Nothing sits between a delimiter and what it encloses, and one space
    /// sits between a closing `)` and `then`.
    fn delimiter_adjacency(&self, edits: &mut EditBuffer) {
        for pair in self.tokens.windows(2) {
            if (pair[0].kind == TokenKind::LParen || pair[0].kind == TokenKind::LBracket)
                && self.gap(pair[0].span.end, pair[1].span.start)
                && !is_trailing_continuation_marker(self.line, pair[1].span.start)
            {
                edits.replace(pair[0].span.end..pair[1].span.start, b"");
            }
            if (pair[1].kind == TokenKind::RParen || pair[1].kind == TokenKind::RBracket)
                && !matches!(pair[0].kind, TokenKind::String | TokenKind::Hollerith)
                && self.gap(pair[0].span.end, pair[1].span.start)
            {
                edits.replace(pair[0].span.end..pair[1].span.start, b"");
            }
            if pair[0].kind == TokenKind::RParen
                && pair[1].is_name(b"then")
                && self.gap(pair[0].span.end, pair[1].span.start)
            {
                edits.replace(pair[0].span.end..pair[1].span.start, b" ");
            }
        }
    }

    /// One space between an `if` condition's `)` and the statement it guards.
    fn if_condition_gap(&self, edits: &mut EditBuffer) {
        let Some(close) = if_condition_close(self.tokens) else {
            return;
        };
        let Some(next) = self.tokens.get(close + 1) else {
            return;
        };
        if next.kind != TokenKind::Comment
            && next.text != b"&"
            && !next.is_name(b"then")
            && self.line[next.span.start..]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace())
        {
            edits.replace(self.tokens[close].span.end..next.span.start, b" ");
        }
    }

    /// `subroutine s()` becomes `subroutine s`.
    fn strip_empty_args(&self, edits: &mut EditBuffer) {
        if !self.style.strip_empty_args {
            return;
        }
        for (index, subroutine) in self.tokens.iter().enumerate() {
            if !subroutine.is_name(b"subroutine")
                || index > 0 && self.tokens[index - 1].is_name(b"end")
            {
                continue;
            }
            if let (Some(name), Some(open), Some(close)) = (
                self.tokens.get(index + 1),
                self.tokens.get(index + 2),
                self.tokens.get(index + 3),
            ) {
                if name.kind == TokenKind::Name
                    && open.kind == TokenKind::LParen
                    && close.kind == TokenKind::RParen
                {
                    edits.replace(open.span.start..close.span.end, b"");
                }
            }
        }
    }
}
