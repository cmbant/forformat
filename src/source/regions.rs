//! One lexical truth: the protected-region walker.
//!
//! Every transform that needs to know "is this byte code, or is it inside a
//! string literal / comment / Hollerith payload?" asks this module.  Before it
//! existed the same quote state machine was reimplemented in `comment_start`,
//! `split_statements`, `is_assignment`, `paren_alignment` and `reduce_line_into`;
//! each copy was a place where doubled quotes or a Hollerith count could be
//! handled slightly differently.
//!
//! The scanner is byte oriented: it never assumes UTF-8, and it carries its
//! state across physical lines so a character literal continued with `&` is a
//! single protected region rather than two unterminated ones.

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
            if let Some((count, after_h)) = hollerith_at(s, i) {
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
    /// comment detection; a stateless call reproduces findent's per-physical
    /// line rule exactly.
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

/// `nH` recognized at `i`: returns the payload length and the offset just past
/// the `H`.
fn hollerith_at(s: &[u8], i: usize) -> Option<(usize, usize)> {
    if !s[i].is_ascii_digit() {
        return None;
    }
    if i > 0 && (s[i - 1].is_ascii_alphanumeric() || s[i - 1] == b'_') {
        return None;
    }
    let mut j = i;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
    }
    if !s.get(j).is_some_and(|x| *x == b'h' || *x == b'H') {
        return None;
    }
    let count = std::str::from_utf8(&s[i..j])
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    Some((count, j + 1))
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
