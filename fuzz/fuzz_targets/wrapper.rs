#![no_main]

use forformat::{analysis::ProjectContext, format_source_with_context, FormatConfig, FormatMode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let default = FormatConfig::default();
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 40 + input.first().copied().unwrap_or(0) as usize,
            ..default.wrap
        },
        ..default
    };
    let _ = format_source_with_context(input, &ProjectContext::empty(), &config);
});
