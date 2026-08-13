#![no_main]

use forformat::classify::classify;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = classify(input);
});
