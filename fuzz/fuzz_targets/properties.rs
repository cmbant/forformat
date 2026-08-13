#![no_main]

use forformat::{format_source, FormatConfig, FormatMode};
use forformat::source::{LexState, RegionKind};
use libfuzzer_sys::fuzz_target;

fn protected(source: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut literals = Vec::new();
    let mut hollerith = Vec::new();
    let mut cpp = Vec::new();
    let mut state = LexState::default();
    for line in source.split(|byte| *byte == b'\n') {
        if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
            cpp.push(line.to_vec());
            state = LexState::default();
            continue;
        }
        let continued_literal = state.in_literal();
        for region in state.regions(line) {
            let mut payload = &line[region.range.clone()];
            // Indentation on a continuation line is source layout, not part
            // of the character literal's payload.
            if continued_literal && region.range.start == 0 {
                payload = payload.trim_ascii_start();
            }
            match region.kind {
                RegionKind::StringLiteral => literals.push(payload.to_vec()),
                RegionKind::Hollerith => hollerith.push(payload.to_vec()),
                _ => {}
            }
        }
    }
    (literals, hollerith, cpp)
}

fuzz_target!(|input: &[u8]| {
    // The I2 contract compares two formatter modes. Both modes intentionally
    // normalize an unterminated final physical line differently, so generate
    // complete documents for this property target.
    let mut document = input.to_vec();
    if !document.ends_with(b"\n") {
        document.push(b'\n');
    }
    let mut config = FormatConfig::default();
    config.mode = FormatMode::Full;
    let Ok(once) = format_source(&document, &config) else { return };
    let Ok(twice) = format_source(&once.bytes, &config) else { return };
    assert_eq!(once.bytes, twice.bytes, "I1");

    let mut indent = FormatConfig::default();
    indent.mode = FormatMode::IndentOnly;
    let Ok(indented) = format_source(&once.bytes, &indent) else { return };
    assert_eq!(indented.bytes, once.bytes, "I2");
    assert_eq!(protected(&document), protected(&once.bytes), "I3");
});
