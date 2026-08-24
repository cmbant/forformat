#![no_main]

use forformat::source::{regions::line_regions, LexState, RegionKind};
use forformat::{format_source, FormatConfig, FormatMode};
use libfuzzer_sys::fuzz_target;

/// Did this literal region reach a closing delimiter?
///
/// The region is either a whole literal (`'abc'`) or the tail of one continued
/// from an earlier line (`&def'`), so only the closing byte is a reliable
/// signal; a lone `'` opening an unterminated literal has nothing after it.
fn is_closed_literal(raw: &[u8]) -> bool {
    raw.len() >= 2 && matches!(raw[raw.len() - 1], b'\'' | b'"')
}

/// One literal region's content, without its delimiters or the continuation
/// marker that opens a continued tail.
///
/// A region is a whole literal (`'abc'`), the head of a continued one (`'abc`),
/// or its tail (`&def'`), so both ends are stripped independently rather than
/// assumed to be a matched pair.
fn literal_content(raw: &[u8]) -> &[u8] {
    let raw = raw.strip_prefix(b"&").unwrap_or(raw);
    let raw = match raw.first() {
        Some(b'\'' | b'"') => &raw[1..],
        _ => raw,
    };
    match raw.last() {
        Some(b'\'' | b'"') => &raw[..raw.len() - 1],
        _ => raw,
    }
}

/// The literal bytes full mode must carry through untouched.
///
/// Deliberately *not* a check on preprocessor lines. This walk classifies
/// physical lines on its own, and that is the one job the pipeline is
/// authoritative for: it resolves the continuation marker before deciding what a
/// line is, so ` &#endif c` is a directive to it and a code line here. Every
/// disagreement this property has ever reported was a line-classification
/// difference of that kind, never a corrupted quote. Directive text is pinned by
/// `preprocessor_lines_are_preserved_byte_for_byte`, by
/// `tests/conditional_compilation.rs`, by the `external_macro_define` fixture,
/// and by the `regions` fuzz target -- all of which check it without guessing.
///
/// What is left is the part worth reimplementing independently: if a pass
/// corrupts a character literal, an oracle that shares `regions.rs` would
/// corrupt its own answer identically and see nothing.
#[derive(Debug, PartialEq, Eq)]
struct Protected {
    /// Every literal's content, concatenated in document order -- not a list of
    /// regions. `docs/full-mode.md` documents that a long literal may split at a
    /// whitespace boundary inside its content, which the wrapper emits as
    /// `'two ' // &` / `'spac...'`. That turns one region into two while leaving
    /// every payload byte in place, so a region-for-region comparison reports
    /// documented behaviour as corruption. Concatenated content survives the
    /// split and still catches a changed byte, which is what I3 is for.
    literals: Vec<u8>,
    /// Hollerith stays a region list: `docs/full-mode.md` says a Hollerith
    /// payload is never split, so its regions must match one for one.
    hollerith: Vec<Vec<u8>>,
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
    let mut literals = Vec::new();
    let mut hollerith = Vec::new();
    let mut well_formed = true;
    let mut state = LexState::default();
    for line in source.split(|byte| *byte == b'\n') {
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
        // A literal still open when the line ends is a syntax error, and this
        // walk has to stop asserting there (see `well_formed`). It has to be
        // read off the *regions*, not off the state: `line_regions` has already
        // applied the group reset by the time it returns, and probing the raw
        // `LexState::scan` instead would lex the comment and blank lines a
        // continued statement steps over -- which is how `! don't stop here`
        // inside a continued literal in `continued_literal.f90` gets mistaken
        // for a dangling quote.
        if let Some(last) = regions.last() {
            if last.kind == RegionKind::StringLiteral
                && last.range.end == line.len()
                && !is_closed_literal(&line[last.range.clone()])
                && !line.trim_ascii_end().ends_with(b"&")
            {
                well_formed = false;
            }
        }
        for region in regions {
            let mut payload = &line[region.range.clone()];
            // Indentation on a continuation line is source layout, not part
            // of the character literal's payload.
            if continued_literal && region.range.start == 0 {
                payload = payload.trim_ascii_start();
            }
            // The final whitespace pass removes horizontal whitespace at the
            // end of every physical line. If an unterminated literal reaches
            // that boundary, those bytes are layout whitespace rather than a
            // stable literal payload for this malformed-input property.
            if region.kind == RegionKind::StringLiteral && region.range.end == line.len() {
                payload = payload.trim_ascii_end();
            }
            match region.kind {
                RegionKind::StringLiteral => literals.extend_from_slice(literal_content(payload)),
                RegionKind::Hollerith => hollerith.push(payload.to_vec()),
                _ => {}
            }
        }
    }
    Protected {
        literals,
        hollerith,
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
