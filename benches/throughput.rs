use std::time::Instant;

fn main() {
    let indent_only = forformat::FormatConfig {
        mode: forformat::FormatMode::IndentOnly,
        ..forformat::FormatConfig::default()
    };
    let workloads = [
        (
            "mixed",
            include_bytes!("../tests/fixtures/benchmark.f90").as_slice(),
        ),
        (
            "continuation",
            include_bytes!("../tests/fixtures/benchmark_continuation.f90").as_slice(),
        ),
        (
            "preprocessor",
            include_bytes!("../tests/fixtures/benchmark_preprocessor.f90").as_slice(),
        ),
    ];
    for (name, fixture) in workloads {
        let mut corpus = Vec::with_capacity(fixture.len() * 1_000);
        for _ in 0..1_000 {
            corpus.extend_from_slice(fixture);
        }
        let source = corpus.as_slice();
        let iterations = 100usize;
        let start = Instant::now();
        let mut bytes = 0usize;
        for _ in 0..iterations {
            let output =
                forformat::format_source(source, &indent_only).expect("benchmark fixture formats");
            bytes = bytes.saturating_add(output.bytes.len());
            std::hint::black_box(output);
        }
        let elapsed = start.elapsed();
        let lines = source.iter().filter(|byte| **byte == b'\n').count() * iterations;
        let seconds = elapsed.as_secs_f64();
        println!(
            "{name}: {} lines, {} bytes in {:.3}s ({:.0} lines/s, {:.1} MB/s)",
            lines,
            bytes,
            seconds,
            lines as f64 / seconds,
            bytes as f64 / seconds / 1_000_000.0
        );
    }
}
