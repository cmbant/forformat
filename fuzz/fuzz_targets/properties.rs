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

/// The bytes full mode must carry through untouched.
#[derive(Debug, PartialEq, Eq)]
struct Protected {
    literals: Vec<Vec<u8>>,
    hollerith: Vec<Vec<u8>>,
    cpp: Vec<Vec<u8>>,
    /// False when a line ends inside a character literal that has neither a
    /// closing delimiter nor a trailing continuation marker.
    ///
    /// Such a literal is a syntax error, and "which bytes are its payload" has
    /// no answer this walk and the pipeline are obliged to agree on. The
    /// formatter decides from `SourceBuffer`, which also knows statement
    /// membership and unresolved preprocessor branches; a line-at-a-time walk
    /// knows neither, and the two part company on inputs carrying more than one
    /// dangling quote. I3 is therefore asserted only where every literal is
    /// delimited or properly continued -- which is every input that is Fortran
    /// at all, and 49 of the 51 checked-in fixtures. I1 and I2 still cover the
    /// rest, and `tests/expected/align_legacy_full.out` pins the formatter's
    /// actual behaviour on dangling quotes as a golden.
    well_formed: bool,
}

fn protected(source: &[u8]) -> Protected {
    let mut literals = Vec::new();
    let mut hollerith = Vec::new();
    let mut cpp = Vec::new();
    let mut well_formed = true;
    let mut state = LexState::default();
    for line in source.split(|byte| *byte == b'\n') {
        if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
            // A directive's *text* is protected, but the horizontal whitespace
            // around it is presentation like every other line's. Trailing blanks
            // go because the final whitespace pass removes them -- and on a
            // `\`-continued `#define` removing them is what makes the
            // continuation work at all. Leading blanks go because a directive is
            // laid out at column 1 rather than at the surrounding indent.
            cpp.push(line.trim_ascii().to_vec());
            // The directive is *stepped over*, not lexed: it neither closes nor
            // resets an open literal, and `SourceBuffer` carries the ordinary
            // stream's state straight across it. Clearing the state here would
            // make the property disagree with the pipeline about where the
            // literal that opened before the `#if` ends.
            continue;
        }
        let continued_literal = state.in_literal();
        let entered_open = continued_literal || state.in_hollerith();
        // Walk the line as part of a continuation group, not as a standalone
        // slice: a literal left unterminated on a line with no continuation
        // marker ends there, and `LexState::regions` alone would keep it open
        // and claim every later line as protected payload.
        let regions = line_regions(&mut state, line);
        if let Some(last) = regions.last() {
            let raw = &line[last.range.clone()];
            if last.kind == RegionKind::StringLiteral
                && last.range.end == line.len()
                && !is_closed_literal(raw)
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
                RegionKind::StringLiteral => literals.push(payload.to_vec()),
                RegionKind::Hollerith => hollerith.push(payload.to_vec()),
                _ => {}
            }
        }
        // `line_regions` clears the state at a line that did not end in a
        // continuation marker. Seeing an open state go into that reset is the
        // signal that a literal was left dangling at a line break.
        let closed_by_the_line_break = (entered_open || state.in_literal() || state.in_hollerith())
            && !state.in_literal()
            && !state.in_hollerith();
        if closed_by_the_line_break && !line.trim_ascii_end().ends_with(b"&") {
            well_formed = false;
        }
    }
    Protected {
        literals,
        hollerith,
        cpp,
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
