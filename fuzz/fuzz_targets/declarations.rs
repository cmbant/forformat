#![no_main]

use forformat::analysis::analyze_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = analyze_file(input);
});
