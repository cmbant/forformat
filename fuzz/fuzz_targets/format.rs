#![no_main]

use findent::{format_source, FormatConfig};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let mut config = FormatConfig::default();
    config.align_paren_value = input.first().copied().unwrap_or(0) as usize;
    config.align_paren = config.align_paren_value != 0;
    config.ws_remred_value = input.get(1).copied().unwrap_or(0) as usize;
    config.ws_remred = config.ws_remred_value != 0;
    let _ = format_source(input, &config);
});
