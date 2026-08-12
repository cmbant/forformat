use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

fn camb_sources() -> Vec<(PathBuf, Vec<u8>)> {
    let mut paths = Vec::new();
    for directory in [
        Path::new("CAMB/fortran"),
        Path::new("CAMB/fortran/tests"),
        Path::new("CAMB/forutils"),
        Path::new("CAMB/forutils/tests"),
    ] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("f90"))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| fs::read(&path).ok().map(|source| (path, source)))
        .collect()
}

fn main() {
    let indent_only = findent::FormatConfig {
        mode: findent::FormatMode::IndentOnly,
        ..findent::FormatConfig::default()
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
                findent::format_source(source, &indent_only).expect("benchmark fixture formats");
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

    let sources = camb_sources();
    if sources.is_empty() {
        println!("Gate G corpus: CAMB not present");
        return;
    }
    let lines: usize = sources
        .iter()
        .map(|(_, source)| source.iter().filter(|byte| **byte == b'\n').count())
        .sum();
    let full = findent::FormatConfig {
        mode: findent::FormatMode::Full,
        ..findent::FormatConfig::default()
    };
    // One iteration is the same unit as the CLI: one project analysis followed
    // by one parallel formatting pass over the selected targets.  The old
    // harness analyzed ten times and formatted every file serially, while the
    // CLI analyzes once and formats targets concurrently; its "total" was
    // therefore neither a CLI invocation nor a useful throughput number.
    let iterations = 10usize;
    let mut analysis_seconds = 0.0;
    let mut format_seconds = 0.0;
    let mut bytes = 0usize;
    for _ in 0..iterations {
        let start = Instant::now();
        let mut context = findent::analyze_project(
            sources
                .iter()
                .map(|(path, source)| (path.as_path(), source.as_slice())),
        )
        .expect("CAMB project analyzes");
        context.enable_target_local_component_resolution();
        std::hint::black_box(&context);
        analysis_seconds += start.elapsed().as_secs_f64();

        let start = Instant::now();
        let outputs = std::thread::scope(|scope| {
            sources
                .iter()
                .map(|(_, source)| {
                    scope.spawn(|| {
                        findent::format_source_with_context(source, &context, &full)
                            .expect("CAMB full-formats")
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("format worker panicked"))
                .collect::<Vec<_>>()
        });
        format_seconds += start.elapsed().as_secs_f64();
        for output in outputs {
            bytes = bytes.saturating_add(output.bytes.len());
            std::hint::black_box(output);
        }
    }
    let analysis_per_iteration = analysis_seconds / iterations as f64;
    let format_per_iteration = format_seconds / iterations as f64;
    println!(
        "Gate G full: {} files, {} lines; analysis {:.3}s/{} ({:.3}s), formatting {:.3}s/{} ({:.3}s), total {:.3}s ({:.0} lines/s)",
        sources.len(), lines, analysis_seconds, iterations, analysis_per_iteration,
        format_seconds, iterations, format_per_iteration,
        analysis_per_iteration + format_per_iteration,
        lines as f64 / format_per_iteration,
    );
    std::hint::black_box(bytes);
}
