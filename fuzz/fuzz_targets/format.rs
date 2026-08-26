#![no_main]

use forformat::{format_source, FormatConfig};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    // The first two input bytes pick the wrapping/whitespace profile, so one
    // corpus explores every combination rather than only the defaults.
    let align_paren_value = input.first().copied().unwrap_or(0) as usize;
    let ws_remred_value = input.get(1).copied().unwrap_or(0) as usize;
    let config = FormatConfig {
        align_paren: align_paren_value.into(),
        ws_remred: ws_remred_value.into(),
        ..FormatConfig::default()
    };
    let _ = format_source(input, &config);
});
