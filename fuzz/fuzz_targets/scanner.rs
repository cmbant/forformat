#![no_main]

use forformat::source::scanner::{split_statements, tokens};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = tokens(input);
    let _ = split_statements(input);
});
