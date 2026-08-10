#![no_main]

use findent::source::{LogicalGroup, SourceBuffer};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    if let Ok(buffer) = SourceBuffer::new(input) {
        let _ = LogicalGroup::assemble(&buffer);
    }
});
