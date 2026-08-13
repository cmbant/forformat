#![no_main]

use forformat::analysis::analyze_project;
use std::path::Path;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let sources = [
        (Path::new("one.f90"), input),
        (Path::new("two.f90"), b"module Kinds\ninteger, parameter :: MyReal = 8\nend module Kinds\n"),
    ];
    let _ = analyze_project(sources);
});
