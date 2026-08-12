#![no_main]

use findent::{format_source_with_context, analysis::ProjectContext, FormatConfig, FormatMode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let mut config = FormatConfig::default();
    config.mode = FormatMode::Full;
    config.wrap.line_length = 40 + input.first().copied().unwrap_or(0) as usize;
    let _ = format_source_with_context(input, &ProjectContext::empty(), &config);
});
