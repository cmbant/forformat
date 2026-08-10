#![no_main]

use findent::{format_source, FormatConfig};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = format_source(input, &FormatConfig::default());
});
