//! One lexical truth: the protected-region walker.
//!
//! Every transform that needs to know "is this byte code, or is it inside a
//! string literal / comment / Hollerith payload?" asks this module.  Before it
//! existed the same quote state machine was reimplemented in `comment_start`,
//! `split_statements`, `is_assignment`, `paren_alignment` and the
//! redundant-whitespace reducer; each copy was a place where doubled quotes or a
//! Hollerith count could be handled slightly differently.  The classifier's
//! copies had both failure modes: `is_assignment` left `'a''b'` half-open, and
//! `single_line_after_paren` forgot to advance its cursor inside a literal, so
//! any `WHERE` or `FORALL` holding one looped forever.  The reducer's was the
//! last to go, and it knew about delimiters but not about Hollerith, so it
//! collapsed a payload's blanks — which are positional data — like any other
//! run.  A recognizer that needs a raw byte scan now takes it from
//! [`for_each_code_byte`].
//!
//! The scanner is byte oriented: it never assumes UTF-8, and it carries its
//! state across physical lines so a character literal continued with `&` is a
//! single protected region rather than two unterminated ones.

use super::syntax::{conditional_compilation_prefix, ConditionalPrefixKind, SourceStream};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Ordinary Fortran source: the only region kind any transform may rewrite.
    Code,
    /// A character literal including both delimiters.  Doubled delimiters are
    /// part of the literal, never a close followed by an open.
    StringLiteral,
    /// From `!` to end of line.
    Comment,
    /// A whole `#`/`??`/`#:` line. Produced by the physical-line classifier
    /// ([`crate::source::PhysicalLineKind::Preprocessor`]), never by the byte
    /// scanner, because being a directive is a property of the line.
    Preprocessor,
    /// `nH` plus the `n` payload bytes.  The payload may be arbitrary bytes,
    /// including quotes and `!`.
    Hollerith,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub range: Range<usize>,
    pub kind: RegionKind,
}

impl Region {
    pub fn is_code(&self) -> bool {
        self.kind == RegionKind::Code
    }
}

/// The carrier that makes a continued character literal one region.
///
/// A default-constructed state means "start of a fresh statement".  Reusing one
/// state across the physical lines of a continuation group is what distinguishes
/// this scanner from the per-line scanners it replaces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LexState {
    /// The active character-literal delimiter, or 0 outside a literal.
    quote: u8,
    /// Hollerith payload bytes still owed from a previous line.
    hollerith: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LineScan {
    /// Offset of an inline comment marker, when present.
    pub comment_start: Option<usize>,
    /// Whether the final significant byte is a semantic continuation marker.
    pub continued: bool,
}

impl LexState {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a character literal is still open at the end of the last
    /// scanned slice.  Wrapping must decline to reflow such a statement (I5).
    pub fn in_literal(&self) -> bool {
        self.quote != 0
    }

    /// True while a Hollerith payload is still owed.
    pub fn in_hollerith(&self) -> bool {
        self.hollerith != 0
    }

    /// Scan one Fortran line body and classify how it ends.
    ///
    /// A trailing `&` is a continuation marker only when that byte belongs to
    /// ordinary code, or when it is the final byte of a still-open character
    /// literal. An ampersand consumed by a Hollerith payload is data, not a
    /// continuation marker. The caller decides whether non-continuation state
    /// should be reset or whether this physical line is stepped over.
    pub(crate) fn scan_line<F: FnMut(Region)>(&mut self, s: &[u8], mut push: F) -> LineScan {
        let mut comment_start = None;
        let mut trailing = None;
        self.scan(s, |region| {
            let range = region.range.clone();
            let kind = region.kind;
            if kind == RegionKind::Comment {
                if comment_start.is_none() {
                    comment_start = Some(range.start);
                }
            } else if let Some(relative) = s[range.clone()]
                .iter()
                .rposition(|byte| !byte.is_ascii_whitespace())
            {
                trailing = Some((range.start + relative, kind));
            }
            push(region);
        });
        let continued = trailing.is_some_and(|(index, kind)| {
            s[index] == b'&'
                && (kind == RegionKind::Code
                    || (kind == RegionKind::StringLiteral && self.in_literal()))
        });
        LineScan {
            comment_start,
            continued,
        }
    }

    /// Walk `s`, reporting a contiguous partition of it into regions.
    ///
    /// Offsets are relative to `s`.  The regions are emitted in order, are
    /// non-empty, and together cover exactly `0..s.len()`, so a transform can
    /// rebuild the input by concatenating them.
    pub fn scan<F: FnMut(Region)>(&mut self, s: &[u8], mut push: F) {
        let mut i = 0usize;
        let mut code_start = 0usize;

        if self.hollerith > 0 {
            let take = self.hollerith.min(s.len());
            self.hollerith -= take;
            if take > 0 {
                push(Region {
                    range: 0..take,
                    kind: RegionKind::Hollerith,
                });
            }
            i = take;
            code_start = take;
        }

        if self.quote != 0 && i < s.len() {
            let start = i;
            i = self.consume_literal(s, i);
            push(Region {
                range: start..i,
                kind: RegionKind::StringLiteral,
            });
            code_start = i;
        }

        while i < s.len() {
            let c = s[i];
            if c == b'\'' || c == b'"' {
                if i > code_start {
                    push(Region {
                        range: code_start..i,
                        kind: RegionKind::Code,
                    });
                }
                let start = i;
                self.quote = c;
                i += 1;
                i = self.consume_literal(s, i);
                push(Region {
                    range: start..i,
                    kind: RegionKind::StringLiteral,
                });
                code_start = i;
                continue;
            }
            if c == b'!' {
                if i > code_start {
                    push(Region {
                        range: code_start..i,
                        kind: RegionKind::Code,
                    });
                }
                push(Region {
                    range: i..s.len(),
                    kind: RegionKind::Comment,
                });
                return;
            }
            if c.is_ascii_digit()
                && (i == 0 || !(s[i - 1].is_ascii_alphanumeric() || s[i - 1] == b'_'))
            {
                let (hollerith, digit_end) = hollerith_from_digit(s, i);
                if let Some((count, after_h)) = hollerith {
                    if i > code_start {
                        push(Region {
                            range: code_start..i,
                            kind: RegionKind::Code,
                        });
                    }
                    let take = count.min(s.len() - after_h);
                    self.hollerith = count - take;
                    let end = after_h + take;
                    push(Region {
                        range: i..end,
                        kind: RegionKind::Hollerith,
                    });
                    i = end;
                    code_start = end;
                    continue;
                }
                i = digit_end;
                continue;
            }
            i += 1;
        }
        if s.len() > code_start {
            push(Region {
                range: code_start..s.len(),
                kind: RegionKind::Code,
            });
        }
    }

    /// Consume bytes of an open literal starting at `i`, clearing `quote` if the
    /// closing delimiter is found before the end of the slice.
    fn consume_literal(&mut self, s: &[u8], mut i: usize) -> usize {
        let q = self.quote;
        while i < s.len() {
            if s[i] == q {
                if s.get(i + 1) == Some(&q) {
                    i += 2;
                    continue;
                }
                self.quote = 0;
                return i + 1;
            }
            i += 1;
        }
        i
    }

    /// Collect the regions of `s` into a vector.
    pub fn regions(&mut self, s: &[u8]) -> Vec<Region> {
        let mut out = Vec::new();
        self.scan(s, |region| out.push(region));
        out
    }

    /// The offset of the inline comment marker, if this slice starts one.
    ///
    /// This is the single implementation behind `SourceBuffer`'s per-line
    /// comment detection.  `SourceBuffer` threads one state through the whole
    /// file, so a `!` carried inside a continued character literal is literal
    /// text rather than the start of a comment.
    pub fn comment_start(&mut self, s: &[u8]) -> Option<usize> {
        let mut found = None;
        self.scan(s, |region| {
            if found.is_none() && region.kind == RegionKind::Comment {
                found = Some(region.range.start);
            }
        });
        found
    }
}

/// `nH` recognized at `i`, where `s[i]` is already known to be an eligible
/// leading digit. Returns the match plus the end of the digit run so a rejected
/// candidate can be skipped without rechecking each later digit.
fn hollerith_from_digit(s: &[u8], i: usize) -> (Option<(usize, usize)>, usize) {
    let mut j = i;
    let mut count = Some(0usize);
    while j < s.len() && s[j].is_ascii_digit() {
        let digit = usize::from(s[j] - b'0');
        count = count.and_then(|value| value.checked_mul(10)?.checked_add(digit));
        j += 1;
    }
    if !s.get(j).is_some_and(|x| *x == b'h' || *x == b'H') {
        return (None, j);
    }
    (Some((count.unwrap_or(0), j + 1)), j)
}

/// Regions of a standalone slice, starting from a clean lexical state.
pub fn regions(s: &[u8]) -> Vec<Region> {
    LexState::default().regions(s)
}

/// The inline comment marker offset of a standalone slice.
pub fn comment_start(s: &[u8]) -> Option<usize> {
    LexState::default().comment_start(s)
}

/// True when `offset` falls in a rewritable code region of `s`.
pub fn is_code_offset(s: &[u8], offset: usize) -> bool {
    let mut code = false;
    LexState::default().scan(s, |region| {
        if region.range.contains(&offset) && region.kind == RegionKind::Code {
            code = true;
        }
    });
    code
}

/// The comment offset of one physical line of a continuation group, carrying
/// `state` on to the next line.
///
/// A `!` only starts a comment when the scan reaches it outside a protected
/// region, and a character literal continued with `&` keeps that region open
/// across the line break — so the group has to be walked in order with one
/// state.  Only the trailing continuation marker licenses that carry: without
/// it the literal is merely unterminated, and leaking its state forward would
/// swallow every later line's comment.
pub fn line_comment_start(state: &mut LexState, line: &[u8]) -> Option<usize> {
    line_scan(state, line).comment_start
}

/// [`line_comment_start`]'s full result, for a caller that also needs to know
/// whether the line ended in a continuation marker.
pub(crate) fn line_scan(state: &mut LexState, line: &[u8]) -> LineScan {
    scan_group_line(state, line, |_| {})
}

/// [`line_comment_start`]'s sibling for a caller that needs every region of one
/// physical line of a continuation group, with `state` carried on to the next
/// line.
///
/// This is the group-aware counterpart of [`LexState::regions`]. Prefer it
/// whenever the caller walks whole physical lines: [`LexState::regions`] alone
/// keeps an unterminated literal open across the line break, but a free-form
/// literal only survives a line that ends in a continuation marker, so the raw
/// state over-reports protected bytes on malformed input.
pub fn line_regions(state: &mut LexState, line: &[u8]) -> Vec<Region> {
    let mut out = Vec::new();
    scan_group_line(state, line, |region| out.push(region));
    out
}

/// [`line_comment_start`]'s sibling for a pass that needs the code bytes rather
/// than the comment offset: the code spans of one physical line of a
/// continuation group, with `state` carried on to the next line.
pub fn line_code_spans<F: FnMut(usize, &[u8])>(state: &mut LexState, line: &[u8], mut f: F) {
    scan_group_line(state, line, |region| {
        if region.kind == RegionKind::Code {
            f(region.range.start, &line[region.range]);
        }
    });
}

/// The offset before which a line's trailing horizontal whitespace is payload
/// rather than presentation.
///
/// Trailing blanks are normally invisible and always safe to drop, but a blank
/// inside a character literal or a Hollerith payload is a byte of the constant:
/// `x = 3Hab ` promises three characters, and trimming the third leaves a `3H`
/// that no longer has three, which is a different program and not a valid one.
/// Blanks in code, or in a comment, carry nothing at end of line.
///
/// Returns the end offset of the last protected region on the line, or 0 when
/// nothing on it is protected. `state` is carried so a literal or payload
/// continued from an earlier physical line is still protected here.
pub fn protected_trailing_floor(state: &mut LexState, line: &[u8]) -> usize {
    let mut floor = 0;
    scan_group_line(state, line, |region| {
        if matches!(
            region.kind,
            RegionKind::StringLiteral | RegionKind::Hollerith
        ) {
            floor = floor.max(region.range.end);
        }
    });
    floor
}

/// Advance `state` over one physical line of a continuation group, reporting
/// nothing.
///
/// For a caller that writes the line's bytes through unchanged and so has no
/// other reason to scan it, but still owes the next physical line of the group
/// the lexical state this one leaves behind.
pub fn advance_group_line(state: &mut LexState, line: &[u8]) {
    scan_group_line(state, line, |_| {});
}

/// One `T` per source stream, for a caller that walks whole physical lines
/// rather than one statement.
///
/// Conditional-compilation code and ordinary code carry independent
/// protected-region state, because a literal continued in one stream steps over
/// the other stream's physical lines without being closed by them.  Every
/// line-walking pass needs that pair, and each one used to declare its own:
/// `SourceBuffer`, the inline-comment detacher, declaration alignment and
/// trailing-whitespace trimming had four separately-written copies of the same
/// two fields.  `T` differs — some carry a bare [`LexState`], some pair it with
/// the pass's own carry — so it is the *pairing* that is shared here, not the
/// state.
#[derive(Debug, Default)]
pub(crate) struct StreamStates<T> {
    pub(crate) ordinary: T,
    pub(crate) conditional: T,
}

impl<T> StreamStates<T> {
    pub(crate) fn select_mut(&mut self, stream: SourceStream) -> &mut T {
        match stream {
            SourceStream::Ordinary => &mut self.ordinary,
            SourceStream::Conditional => &mut self.conditional,
        }
    }
}

/// The common case: one [`LexState`] per stream.
pub(crate) type StreamLexStates = StreamStates<LexState>;

impl StreamLexStates {
    /// [`protected_trailing_floor`] for `line`, advancing whichever stream's
    /// state the line belongs to.
    ///
    /// A compact `!$&` prefix is contextual, and this walker has no statement
    /// context to resolve it with, so it is left to the ordinary stream exactly
    /// as [`stepped_over_by_continuation`] leaves it.
    pub(crate) fn protected_trailing_floor(&mut self, line: &[u8]) -> usize {
        let body_start = match conditional_compilation_prefix(line) {
            Some(prefix) if prefix.kind == ConditionalPrefixKind::BlankSeparated => {
                prefix.body_start
            }
            _ => return protected_trailing_floor(&mut self.ordinary, line),
        };
        match protected_trailing_floor(&mut self.conditional, &line[body_start..]) {
            0 => 0,
            floor => body_start + floor,
        }
    }
}

/// Does a protected region open on an earlier physical line survive onto this
/// one, given that the caller has already decided the two lines belong to the
/// same continuation group?
///
/// A free-form character literal can only resume on a physical line whose first
/// nonblank body byte is `&`.  When malformed or inactive source puts some
/// other code-looking line in between, the safe reading is that the line is
/// transparent: it is stepped over without consuming or closing the open
/// literal, exactly as `SourceBuffer` treats it.
///
/// This is the strict sibling of [`stepped_over_by_continuation`], which
/// answers the *shape* question — blank, comment or directive — for a caller
/// that has no group structure to lean on.  Both are needed: this one is for a
/// caller walking a known group, that one for a caller walking raw lines.
pub(crate) fn resumes_protected_region(state: &LexState, body: &[u8]) -> bool {
    !state.in_literal() || body.trim_ascii_start().starts_with(b"&")
}

/// What one source stream made of a code line, and how its state moved.
pub(crate) enum StreamLine {
    /// The stream lexed the line: its regions, comment span and continuation
    /// all come from this scan, and `state` has been advanced over it.
    Lexed(LineScan),
    /// The line is transparent to this stream. It is code-shaped but cannot
    /// resume the protected region the stream is carrying, so the state is
    /// left untouched and only the line's own continuation syntax counts.
    Transparent { continued: bool },
}

/// Advance one stream's lexical state across one code line.
///
/// The two streams differ only in which gate decides that a line is stepped
/// over rather than lexed. The ordinary stream walks raw lines, so it uses the
/// shape gate inside [`scan_group_line`] — blank, comment, directive. The
/// conditional stream's body is always code-shaped, so only the strict "a
/// literal resumes on `&`" rule can apply to it.
///
/// Every reader that needs to know what is protected on a given line has to
/// advance the same way or it disagrees about which bytes are literal text.
/// [`SourceBuffer`] and step 11 both go through here for exactly that reason:
/// while step 11 carried a state of its own, a literal left open by a line that
/// did not continue leaked into every statement after it, and the `!` that the
/// wrapper detached as a trailing comment was one step 11 still believed was
/// inside a literal — so the hoisted comment gained its space a pass late.
///
/// [`SourceBuffer`]: super::SourceBuffer
/// [`scan_group_line`]: self::scan_group_line
pub(crate) fn advance_stream_line(
    state: &mut LexState,
    code: &[u8],
    conditional: bool,
) -> StreamLine {
    if !conditional {
        return StreamLine::Lexed(line_scan(state, code));
    }
    if resumes_protected_region(state, code) {
        let scan = state.scan_line(code, |_| {});
        if !scan.continued {
            *state = LexState::default();
        }
        return StreamLine::Lexed(scan);
    }
    // A code-looking line that cannot resume the carried literal is transparent
    // to that protected state. It still owns its *own* physical continuation
    // syntax, though: the malformed-string compatibility fixture has exactly
    // such a line ending in `&`. Classify only that fact from a clean state,
    // without consuming the carried literal or changing its comment spans.
    StreamLine::Transparent {
        continued: LexState::default().scan_line(code, |_| {}).continued,
    }
}

/// Scan one line of a continuation group, then decide whether lexical state
/// survives it. Protected bytes that merely happen to be `&` never license a
/// carry into the next physical line.
fn scan_group_line<F: FnMut(Region)>(state: &mut LexState, line: &[u8], push: F) -> LineScan {
    if *state != LexState::default() && stepped_over_by_continuation(line) {
        return LineScan::default();
    }
    let scan = state.scan_line(line, push);
    if !scan.continued {
        *state = LexState::default();
    }
    scan
}

/// Does a continued statement step over this line rather than absorb it?
///
/// A blank, a comment and a preprocessor directive are not part of the
/// statement they sit inside: [`LogicalGroup::visit`] skips exactly these three
/// kinds when it joins a group's continuation lines, and findent copies them to
/// the output without ever lexing them.  So while a character literal or a
/// Hollerith payload is open, such a line contributes no regions and cannot
/// close it -- the apostrophe in prose like `! don't stop here` is not a
/// delimiter, and the literal resumes at the `&` on the next code line.
///
/// A blank-separated conditional-compilation sentinel (`!$ ` / `!$\t`) is
/// unconditionally code-shaped and is therefore not in the skipped set. Compact
/// `!$&` is different: it is conditional code only with proven incoming
/// continuation context, so this context-free helper treats a raw compact line
/// as comment-like. Semantic callers use `SourceBuffer` when that distinction
/// matters.
///
/// A `!findentfix:` line *is* in the set, and this is the one place the set is
/// deliberately wider than [`LogicalGroup::visit`]'s, which treats
/// [`PhysicalLineKind::FindentFix`] as a group boundary rather than a skipped
/// line.  The two answer different questions.  To gfortran a findentfix
/// directive is an ordinary comment, so lexically the literal around it stays
/// open; ending it here would reopen the exact bug this function exists to
/// prevent, rewriting `&def!ghi')` as code plus a comment.  The cost of the
/// asymmetry is bounded and visible: a directive between the halves of a
/// continued statement truncates the *statement*, so a construct head split
/// that way is not recognized and its body is under-indented against findent.
/// Content is preserved either way; do not "align" the two sets by dropping
/// findentfix from this one.
///
/// [`LogicalGroup::visit`]: crate::source::LogicalGroup::visit
/// [`PhysicalLineKind::FindentFix`]: crate::source::PhysicalLineKind::FindentFix
pub fn stepped_over_by_continuation(line: &[u8]) -> bool {
    let trimmed = line.trim_ascii_start();
    let blank_conditional = conditional_compilation_prefix(line)
        .is_some_and(|prefix| prefix.kind == ConditionalPrefixKind::BlankSeparated);
    trimmed.is_empty()
        || trimmed.starts_with(b"#")
        || trimmed.starts_with(b"??")
        || (trimmed.starts_with(b"!") && !blank_conditional)
}

/// Visit every code region of `s` as `(start offset, bytes)`, skipping string
/// literals, Hollerith payloads and any trailing comment.
///
/// This is the allocation-free entry point for the recognizers, which need a
/// byte scan with their own bracket bookkeeping rather than a token stream.
/// Each of them used to carry a private copy of the quote state machine, and
/// those copies were where doubled delimiters and missing loop increments went
/// wrong; walking the shared regions removes the state machine from the caller
/// entirely.
pub fn for_each_code_span<F: FnMut(usize, &[u8])>(s: &[u8], mut f: F) {
    LexState::default().scan(s, |region| {
        if region.kind == RegionKind::Code {
            f(region.range.start, &s[region.range]);
        }
    });
}

/// Visit every code byte of `s` as `(offset, byte)`.  Offsets index `s`, so a
/// caller may still look at the neighbouring protected bytes deliberately.
pub fn for_each_code_byte<F: FnMut(usize, u8)>(s: &[u8], mut f: F) {
    for_each_code_span(s, |start, span| {
        for (index, byte) in span.iter().enumerate() {
            f(start + index, *byte);
        }
    });
}

/// Apply `f` to every code region of `s`, copying protected regions through
/// byte-for-byte.  This is the safe default for a text normalization rule that
/// does not need a token stream: I3 holds by construction because `f` is never
/// shown a protected byte.
pub fn map_code<F>(s: &[u8], state: &mut LexState, mut f: F) -> Vec<u8>
where
    F: FnMut(&[u8], &mut Vec<u8>),
{
    let mut out = Vec::with_capacity(s.len());
    state.scan(s, |region| {
        let slice = &s[region.range.clone()];
        if region.kind == RegionKind::Code {
            f(slice, &mut out);
        } else {
            out.extend_from_slice(slice);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{comment_start, is_code_offset, map_code, regions, LexState, RegionKind};

    fn kinds(s: &[u8]) -> Vec<(RegionKind, &[u8])> {
        regions(s)
            .into_iter()
            .map(|region| (region.kind, &s[region.range]))
            .collect()
    }

    #[test]
    fn trailing_continuation_respects_region_ownership() {
        let scan = |line: &[u8]| {
            let mut state = LexState::default();
            state.scan_line(line, |_| {}).continued
        };

        assert!(scan(b"x = a &"));
        assert!(scan(b"x = 'abc &"));
        assert!(!scan(b"x = 1H&"));
        assert!(!scan(b"x = 2H&&"));
        assert!(scan(b"x = 1H&&"));
        assert!(!scan(b"x = 1 ! comment &"));
    }

    #[test]
    fn regions_partition_the_input_exactly() {
        for source in [
            b"x = 'a''b' ! tail".as_slice(),
            b"x = 3Habc + 1".as_slice(),
            b"".as_slice(),
            b"!".as_slice(),
            b"'unterminated".as_slice(),
            b"\xff\xfe 'q' \xff".as_slice(),
        ] {
            let mut end = 0;
            for region in regions(source) {
                assert_eq!(region.range.start, end, "gap in {source:?}");
                assert!(region.range.start < region.range.end, "empty region");
                end = region.range.end;
            }
            assert_eq!(end, source.len(), "short coverage of {source:?}");
        }
    }

    #[test]
    fn doubled_quotes_stay_inside_one_literal() {
        assert_eq!(
            kinds(b"x = 'a''b' + y"),
            [
                (RegionKind::Code, b"x = ".as_slice()),
                (RegionKind::StringLiteral, b"'a''b'".as_slice()),
                (RegionKind::Code, b" + y".as_slice()),
            ]
        );
    }

    #[test]
    fn a_literal_continues_across_physical_lines() {
        let mut state = LexState::default();
        let first = state.regions(b"call sub('hello &");
        assert_eq!(first.last().unwrap().kind, RegionKind::StringLiteral);
        assert!(state.in_literal());
        let second = state.regions(b"world', x)");
        assert_eq!(second[0].kind, RegionKind::StringLiteral);
        assert_eq!(&b"world'"[..], &b"world', x)"[second[0].range.clone()]);
        assert!(!state.in_literal());
    }

    #[test]
    fn hollerith_payload_is_protected_and_may_span_lines() {
        assert_eq!(
            kinds(b"x = 3H!'; + 1"),
            [
                (RegionKind::Code, b"x = ".as_slice()),
                (RegionKind::Hollerith, b"3H!';".as_slice()),
                (RegionKind::Code, b" + 1".as_slice()),
            ]
        );
        let mut state = LexState::default();
        state.regions(b"x = 6Hab");
        assert!(state.in_hollerith());
        let next = state.regions(b"cd! y");
        assert_eq!(next[0].kind, RegionKind::Hollerith);
        assert_eq!(next[0].range, 0..4);
    }

    #[test]
    fn overflowing_hollerith_count_preserves_zero_count_behavior() {
        let source = format!("{}0Habc", usize::MAX).into_bytes();
        let prefix_end = source.iter().position(|byte| *byte == b'H').unwrap() + 1;
        let parsed = regions(&source);
        assert_eq!(parsed[0].kind, RegionKind::Hollerith);
        assert_eq!(parsed[0].range, 0..prefix_end);
        assert_eq!(parsed[1].kind, RegionKind::Code);
        assert_eq!(&source[parsed[1].range.clone()], b"abc");
    }

    #[test]
    fn comment_detection_matches_the_previous_hand_written_scanner() {
        assert_eq!(comment_start(b"x='!'; y=\"!\" ! real"), Some(13));
        assert_eq!(comment_start(b"x=3H;!; ! real"), Some(8));
        assert_eq!(comment_start(b"x='a''!b' ! real"), Some(10));
        assert_eq!(comment_start(b"x='a''!b'\xff ! real"), Some(11));
        assert_eq!(comment_start(b"x=\"a\"\xff ! real"), Some(7));
        assert_eq!(comment_start(b"x=3h\xff! real"), None);
        assert_eq!(comment_start(b"x = 1"), None);
    }

    #[test]
    fn a_group_walk_keeps_a_continued_literal_out_of_comment_detection() {
        let mut state = LexState::default();
        assert_eq!(
            super::line_comment_start(&mut state, b"if (s == 'abc &"),
            None
        );
        assert!(state.in_literal());
        assert_eq!(
            super::line_comment_start(&mut state, b"&def!ghi') then ! tail"),
            Some(16)
        );
        assert!(!state.in_literal());

        // No trailing marker, so the unterminated literal ends with its line
        // rather than swallowing the next one's comment.
        let mut broken = LexState::default();
        assert_eq!(super::line_comment_start(&mut broken, b"x = 'abc"), None);
        assert_eq!(
            super::line_comment_start(&mut broken, b"y = 1 ! note"),
            Some(6)
        );
    }

    #[test]
    fn a_skipped_line_neither_lexes_into_nor_closes_an_open_literal() {
        // A continued statement steps over blanks, comments and directives, so
        // the odd apostrophe in `! don't` must not invert the literal state and
        // the `!` on the resumed line must stay literal text.
        for separator in [
            &b""[..],
            b"   ",
            b"! don't stop here",
            b"      ! tail",
            b"#ifdef FOO",
            b"??cpp",
        ] {
            let mut state = LexState::default();
            assert_eq!(
                super::line_comment_start(&mut state, b"if (s == 'abc &"),
                None
            );
            assert!(state.in_literal());
            assert_eq!(super::line_comment_start(&mut state, separator), None);
            assert!(
                state.in_literal(),
                "separator {:?} closed the literal",
                String::from_utf8_lossy(separator)
            );
            assert_eq!(
                super::line_comment_start(&mut state, b"&def!ghi') then ! tail"),
                Some(16)
            );
            assert!(!state.in_literal());
        }
    }

    #[test]
    fn a_findentfix_directive_does_not_close_the_literal_around_it() {
        // gfortran reads a findentfix directive as an ordinary comment, so the
        // literal spans it. `LogicalGroup::visit` still breaks the *statement*
        // there, which is a deliberate asymmetry: it costs indentation on a
        // split construct head, while lexing the directive would corrupt the
        // literal's bytes.
        let mut state = LexState::default();
        assert_eq!(
            super::line_comment_start(&mut state, b"if (s == 'abc &"),
            None
        );
        assert!(state.in_literal());
        assert_eq!(
            super::line_comment_start(&mut state, b"!findentfix: free"),
            None
        );
        assert!(state.in_literal());
        assert_eq!(
            super::line_comment_start(&mut state, b"&def!ghi') then"),
            None
        );
    }

    #[test]
    fn conditional_sentinel_transparency_respects_compact_context() {
        for line in [b"!$ x = 1".as_slice(), b"!$\tx = 1"] {
            assert!(!super::stepped_over_by_continuation(line), "{line:?}");
        }
        for line in [
            b"!$& x = 1".as_slice(),
            b"  !$&x = 1",
            b"!$OMP parallel",
            b"!$OMPX vendor",
        ] {
            assert!(super::stepped_over_by_continuation(line), "{line:?}");
        }
    }

    #[test]
    fn a_skipped_line_is_still_lexed_outside_a_literal() {
        // The step-over applies only while a context is open; an ordinary
        // comment line keeps reporting its own start.
        let mut state = LexState::default();
        assert_eq!(super::line_comment_start(&mut state, b"! plain"), Some(0));
    }

    #[test]
    fn code_walkers_report_only_rewritable_bytes() {
        let source = b"a('x!y') = 3Hz!q + b ! tail";
        let mut spans = Vec::new();
        super::for_each_code_span(source, |start, span| spans.push((start, span.to_vec())));
        assert_eq!(
            spans
                .iter()
                .map(|(start, span)| (*start, span.as_slice()))
                .collect::<Vec<_>>(),
            [
                (0, b"a(".as_slice()),
                (7, b") = ".as_slice()),
                (16, b" + b ".as_slice()),
            ]
        );
        let mut seen = Vec::new();
        super::for_each_code_byte(b"x = 'a''b' + y", |i, byte| seen.push((i, byte)));
        assert_eq!(seen.first(), Some(&(0, b'x')));
        assert!(!seen.iter().any(|(i, _)| (4..10).contains(i)));
        assert_eq!(seen.last(), Some(&(13, b'y')));
    }

    #[test]
    fn code_mapping_leaves_protected_bytes_alone() {
        let mut state = LexState::default();
        let out = map_code(b"x  =  'a  b'  !  c", &mut state, |code, out| {
            let mut space = false;
            for byte in code {
                if *byte == b' ' {
                    space = true;
                    continue;
                }
                if space {
                    out.push(b' ');
                    space = false;
                }
                out.push(*byte);
            }
            if space {
                out.push(b' ');
            }
        });
        assert_eq!(out, b"x = 'a  b' !  c");
        assert!(is_code_offset(b"x = 'a' + y", 0));
        assert!(!is_code_offset(b"x = 'a' + y", 5));
    }
}
