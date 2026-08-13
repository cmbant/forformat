#![no_main]

use forformat::source::{regions::regions, SourceBuffer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = regions(input);
    let _ = SourceBuffer::new(input);
});
