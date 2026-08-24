#![no_main]

use forformat::source::{
    regions::line_regions, LexState, PhysicalLineKind, RegionKind, SourceBuffer,
};
use forformat::{format_source, FormatConfig, FormatMode};
use libfuzzer_sys::fuzz_target;

/// One character literal's bytes. The delimiter is part of the invariant: a
/// formatter may split one literal for wrapping, but it must not silently change
/// `'abc'` into `"abc"` or move bytes between unrelated literals.
#[derive(Debug, PartialEq, Eq)]
struct Literal {
    delimiter: u8,
    content: Vec<u8>,
}

/// Drop the trailing `&` that continues an open character literal, plus any
/// horizontal whitespace after it. Whitespace before the marker is still
/// literal payload and must remain protected.
fn without_trailing_continuation(raw: &[u8]) -> &[u8] {
    let end = raw
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |index| index + 1);
    if end > 0 && raw[end - 1] == b'&' {
        &raw[..end - 1]
    } else {
        raw
    }
}

/// Extract one region's literal payload without its syntactic delimiters or
/// continuation markers. `continued` means the opening delimiter was on an
/// earlier physical line; `continues` means this region leaves the literal open
/// for the next line.
fn literal_piece(raw: &[u8], delimiter: u8, continued: bool, continues: bool) -> (&[u8], bool) {
    let mut raw = raw;
    if continued {
        raw = raw.trim_ascii_start();
        raw = raw.strip_prefix(b"&").unwrap_or(raw);
    } else if raw.first() == Some(&delimiter) {
        raw = &raw[1..];
    }

    let closed = raw.last() == Some(&delimiter);
    if closed {
        raw = &raw[..raw.len() - 1];
    } else if continues {
        raw = without_trailing_continuation(raw);
    }
    (raw, closed)
}

/// One document's preprocessor directive lines, trimmed the same way the
/// emitter trims them (`emitter.rs`: "Preprocessor spelling is preserved, but
/// its source indentation is always structural noise and trailing horizontal
/// whitespace is normalized"), in document order.
///
/// Classification comes from `SourceBuffer` itself, not a hand-rolled `#`
/// check: an earlier version of this walk classified lines on its own and
/// disagreed with the pipeline about ` &#endif c` (a continuation marker
/// immediately followed by a directive), which is a directive to `SourceBuffer`
/// and looked like code here -- a false I3 failure with nothing wrong in the
/// formatter. Reusing the real classifier removes that whole class of
/// disagreement; `None` means `SourceBuffer::new` itself failed, which the
/// caller treats as not well-formed.
fn preprocessor_lines(source: &[u8]) -> Option<Vec<Vec<u8>>> {
    let buffer = SourceBuffer::new(source).ok()?;
    Some(
        buffer
            .lines
            .iter()
            .filter(|line| line.kind == PhysicalLineKind::Preprocessor)
            .map(|line| {
                source[line.span.start as usize..line.span.end as usize]
                    .trim_ascii()
                    .to_vec()
            })
            .collect(),
    )
}

/// The literal and directive bytes full mode must carry through untouched.
///
/// What is left out is the part worth reimplementing independently: if a pass
/// corrupts a character literal, an oracle that shares `regions.rs` would
/// corrupt its own answer identically and see nothing. Directive lines don't
/// have that hazard -- `SourceBuffer`'s classification is reused rather than
/// reimplemented (see `preprocessor_lines`), and only the resulting bytes are
/// compared, so a pass that corrupts directive spelling still shows up here.
#[derive(Debug, PartialEq, Eq)]
struct Protected {
    /// Logical character literals in document order. A physical continuation
    /// remains one entry, and the exact `'<piece>' // &` shape emitted by the
    /// wrapper is folded back into that same entry on the following line. Other
    /// literal boundaries stay visible, so bytes cannot migrate between
    /// unrelated constants without I3 noticing.
    literals: Vec<Literal>,
    /// Hollerith stays a region list: `docs/full-mode.md` says a Hollerith
    /// payload is never split, so its regions must match one for one.
    hollerith: Vec<Vec<u8>>,
    /// See `preprocessor_lines`.
    preprocessor: Vec<Vec<u8>>,
    /// False when a line ends inside a character literal that has neither a
    /// closing delimiter nor a trailing continuation marker.
    ///
    /// Such a literal is a syntax error, and "which bytes are its payload" has
    /// no answer this walk and the pipeline are obliged to agree on: the
    /// formatter decides from `SourceBuffer`, which also knows statement
    /// membership and unresolved preprocessor branches. I3 is therefore
    /// asserted only where every literal is delimited or properly continued.
    /// I1 and I2 still cover the rest, and `tests/expected/align_legacy_full.out`
    /// pins the formatter's actual behaviour on dangling quotes as a golden.
    well_formed: bool,
}

fn protected(source: &[u8]) -> Protected {
    let mut literals: Vec<Literal> = Vec::new();
    let mut hollerith = Vec::new();
    let mut well_formed = true;
    let mut state = LexState::default();
    let mut merge_wrapped_literal = false;
    for line in source.split(|byte| *byte == b'\n') {
        // A wrapper split is always resumed on the very next physical line.
        // Taking this before classifying the line prevents comments/directives
        // from accidentally carrying the merge forward.
        let merge_from_previous = std::mem::take(&mut merge_wrapped_literal);
        if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
            // A directive is *stepped over*, not lexed: it neither closes nor
            // resets an open literal, and `SourceBuffer` carries the ordinary
            // stream's state straight across it. Clearing the state here would
            // make the property disagree with the pipeline about where the
            // literal that opened before the `#if` ends.
            continue;
        }
        let continued_literal = state.in_literal();
        // Walk the line as part of a continuation group, not as a standalone
        // slice: a literal left unterminated on a line with no continuation
        // marker ends there, and `LexState::scan` alone would keep it open
        // and claim every later line as protected payload.
        let regions = line_regions(&mut state, line);
        let literal_continues = state.in_literal();
        let mut first_literal = true;
        for region in regions {
            let raw = &line[region.range.clone()];
            match region.kind {
                RegionKind::StringLiteral => {
                    let mut may_split_wrapped_literal = false;
                    let closed = if continued_literal && region.range.start == 0 {
                        if !raw.trim_ascii_start().starts_with(b"&") {
                            well_formed = false;
                        }
                        let Some(literal) = literals.last_mut() else {
                            well_formed = false;
                            continue;
                        };
                        let (content, closed) =
                            literal_piece(raw, literal.delimiter, true, literal_continues);
                        literal.content.extend_from_slice(content);
                        closed
                    } else {
                        let Some(&delimiter) = raw.first() else {
                            continue;
                        };
                        if !matches!(delimiter, b'\'' | b'"') {
                            well_formed = false;
                            continue;
                        }
                        let (content, closed) =
                            literal_piece(raw, delimiter, false, literal_continues);
                        may_split_wrapped_literal = closed;
                        let prefix = line[..region.range.start].trim_ascii();
                        let resumes_wrapped = prefix.is_empty() || prefix == b"&";
                        let merge = first_literal
                            && merge_from_previous
                            && resumes_wrapped
                            && literals
                                .last()
                                .is_some_and(|literal| literal.delimiter == delimiter);
                        if merge {
                            literals
                                .last_mut()
                                .expect("merge requires a previous literal")
                                .content
                                .extend_from_slice(content);
                        } else {
                            literals.push(Literal {
                                delimiter,
                                content: content.to_vec(),
                            });
                        }
                        first_literal = false;
                        closed
                    };
                    if !closed && !literal_continues {
                        well_formed = false;
                    }

                    // `literal_wrap_split` emits exactly `'<piece>' // &` and
                    // resumes the same-delimiter literal after indentation (and
                    // an optional authored leading `&`) on the next line. Fold
                    // only that shape; ordinary neighboring
                    // literals remain separate entries in the invariant.
                    if may_split_wrapped_literal && line[region.range.end..].trim_ascii() == b"// &"
                    {
                        merge_wrapped_literal = true;
                    }
                }
                RegionKind::Hollerith => hollerith.push(raw.to_vec()),
                _ => {}
            }
        }
    }
    let preprocessor = preprocessor_lines(source).unwrap_or_else(|| {
        well_formed = false;
        Vec::new()
    });
    Protected {
        literals,
        hollerith,
        preprocessor,
        well_formed,
    }
}

fuzz_target!(|input: &[u8]| {
    // The I2 contract compares two formatter modes. Both modes intentionally
    // normalize an unterminated final physical line differently, so generate
    // complete documents for this property target.
    let mut document = input.to_vec();
    if !document.ends_with(b"\n") {
        document.push(b'\n');
    }
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let Ok(once) = format_source(&document, &config) else {
        return;
    };
    let Ok(twice) = format_source(&once.bytes, &config) else {
        return;
    };
    assert_eq!(once.bytes, twice.bytes, "I1");

    let indent = FormatConfig {
        mode: FormatMode::IndentOnly,
        ..FormatConfig::default()
    };
    let Ok(indented) = format_source(&once.bytes, &indent) else {
        return;
    };
    assert_eq!(indented.bytes, once.bytes, "I2");

    let before = protected(&document);
    if before.well_formed {
        assert_eq!(before, protected(&once.bytes), "I3");
    }
});
