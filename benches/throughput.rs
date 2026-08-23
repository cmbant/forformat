//! Throughput and peak-memory benchmark.
//!
//! Two halves, because they cover different engines. `lexical_throughput`
//! measures the per-byte work on single buffers; `project_throughput` measures
//! the project-aware half — scope analysis, the USE graph, visibility queries
//! and both case passes — which only runs in the normalizing modes and only
//! when a `ProjectContext` spans more than one file.
//!
//! Peak RSS is reported alongside time because the two have already come apart
//! once: a change that left wall time flat grew peak RSS by 80% on a real
//! corpus, and nothing in a time-only benchmark could see it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use forformat::analysis::analyze_project_with_includes;

fn main() {
    lexical_throughput();
    println!();
    project_throughput();
}

fn lexical_throughput() {
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
            "{name}: {} lines, {} bytes in {:.3}s ({:.0} lines/s, {:.1} MB/s){}",
            lines,
            bytes,
            seconds,
            lines as f64 / seconds,
            bytes as f64 / seconds / 1_000_000.0,
            peak_suffix()
        );
    }
}

/// Modules in the synthetic project.
///
/// Both costs this half exists to watch scale with the project rather than
/// with any one file: the evidence map holds an entry per name token across
/// every source, and the include lookup holds facts per supplied file. A
/// single large file would show neither.
const MODULES: usize = 800;
const PROJECT_ROOT: &str = "/forformat-bench";
const INCLUDE_NAME: &str = "shared_kinds.inc";

/// The shared INCLUDE. One fragment reached by every module is the shape that
/// makes fragment caching and include lookup worth measuring at all; it also
/// gives the self-check below something to prove.
const INCLUDE_SOURCE: &[u8] = b"integer, parameter :: bench_kind_wp = kind(1.0d0)\n";

fn project_throughput() {
    let files = generate_project();
    let sources: Vec<(&Path, &[u8])> = files
        .iter()
        .map(|(path, source)| (path.as_path(), source.as_slice()))
        .collect();
    let include_path = PathBuf::from(PROJECT_ROOT).join(INCLUDE_NAME);
    let includes = [(include_path.as_path(), INCLUDE_SOURCE)];

    let lines = files
        .iter()
        .map(|(_, source)| source.iter().filter(|byte| **byte == b'\n').count())
        .sum::<usize>();

    let start = Instant::now();
    let context = analyze_project_with_includes(sources.iter().copied(), includes.iter().copied())
        .expect("benchmark project analyzes");
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "project analyze: {} files, {} lines in {:.3}s ({:.0} lines/s){}",
        files.len(),
        lines,
        seconds,
        lines as f64 / seconds,
        peak_suffix()
    );

    for (name, mode) in [
        ("normalize-only", forformat::FormatMode::NormalizeOnly),
        ("full", forformat::FormatMode::Full),
    ] {
        let config = forformat::FormatConfig {
            mode,
            ..forformat::FormatConfig::default()
        };
        let start = Instant::now();
        let mut bytes = 0usize;
        for (_, source) in &files {
            let output = forformat::format_source_with_context(source, &context, &config)
                .expect("benchmark project formats");
            bytes = bytes.saturating_add(output.bytes.len());
            std::hint::black_box(output);
        }
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "project {name}: {} lines, {} bytes in {:.3}s ({:.0} lines/s, {:.1} MB/s){}",
            lines,
            bytes,
            seconds,
            lines as f64 / seconds,
            bytes as f64 / seconds / 1_000_000.0,
            peak_suffix()
        );
    }

    check_include_resolved(&context, &files);
}

/// A benchmark that silently stops exercising what it claims to exercise is
/// worse than no benchmark. `bench_kind_wp` is declared only in the INCLUDE
/// and written in upper case in every module body, so it comes back lowercase
/// exactly when the fragment was found, analyzed, and carried into the
/// project's declared names. If lookup regresses to a filesystem miss, the
/// spelling stays as written and this fails loudly.
fn check_include_resolved(context: &forformat::ProjectContext, files: &[(PathBuf, Vec<u8>)]) {
    let config = forformat::FormatConfig {
        mode: forformat::FormatMode::Full,
        ..forformat::FormatConfig::default()
    };
    let (_, source) = files.last().expect("project has modules");
    let output = forformat::format_source_with_context(source, context, &config)
        .expect("benchmark project formats");
    let text = String::from_utf8(output.bytes).expect("formatted output is UTF-8");
    assert!(
        text.contains("bench_kind_wp") && !text.contains("BENCH_KIND_WP"),
        "the shared INCLUDE no longer reaches project name resolution; \
         this benchmark is measuring the wrong thing"
    );
}

/// Peak resident set size in MiB, from the kernel's own high-water mark.
///
/// `VmHWM` is a high-water mark over the whole process, so it never falls and
/// each line reports the peak as of its own end rather than a per-stage
/// figure. That is the right shape for the thing being watched: the question
/// is what the process had to hold at once, not what any one stage allocated.
fn peak_rss_mib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmHWM:")?;
        Some(value.split_whitespace().next()?.parse::<u64>().ok()? / 1024)
    })
}

fn peak_suffix() -> String {
    let mut suffix = String::new();
    if let Some(peak) = peak_rss_mib() {
        let _ = write!(suffix, ", peak RSS {peak} MB");
    }
    suffix
}

fn generate_project() -> Vec<(PathBuf, Vec<u8>)> {
    let root = PathBuf::from(PROJECT_ROOT);
    let mut files = Vec::with_capacity(MODULES + 1);
    files.push((root.join("shared_types.f90"), shared_types_source()));
    for index in 0..MODULES {
        files.push((
            root.join(format!("{}.f90", module_name(index))),
            module_source(index),
        ));
    }
    files
}

fn module_name(index: usize) -> String {
    format!("geom_{index:03}")
}

/// The project's common vocabulary.
///
/// `geom_000` is deliberately both a module in this project and a derived type
/// here. Resolving that token means deciding which namespace it belongs to
/// rather than matching a spelling, which is the work being measured.
fn shared_types_source() -> Vec<u8> {
    let mut source = String::new();
    source.push_str(
        "module shared_types\n\
         implicit none\n\
         private\n\
         public :: point_t, record_t, geom_000, register\n\
         type :: point_t\n\
         real :: x\n\
         real :: y\n\
         real :: z\n\
         end type point_t\n\
         type :: record_t\n\
         character(len=32) :: label\n\
         type(point_t) :: at\n\
         end type record_t\n\
         type :: geom_000\n\
         integer :: tag\n\
         end type geom_000\n\
         contains\n\
         subroutine register(entry)\n\
         type(record_t), intent(in) :: entry\n\
         print *, entry%label\n\
         end subroutine register\n",
    );
    source.push_str("end module shared_types\n");
    source.into_bytes()
}

fn module_source(index: usize) -> Vec<u8> {
    let name = module_name(index);
    let mut source = String::new();
    let _ = writeln!(source, "module {name}");
    source.push_str("use shared_types, only: point_t, record_t, register\n");
    // A chain rather than a star: every module but the first resolves a name
    // through an alias of a public entity in another module, so the USE graph
    // has depth as well as breadth.
    if index > 0 {
        let previous = module_name(index - 1);
        let _ = writeln!(
            source,
            "use {previous}, only: neighbour_{index:03} => shell_{:03}",
            index - 1
        );
    }
    source.push_str("implicit none\n");
    let _ = writeln!(source, "include '{INCLUDE_NAME}'");
    source.push_str("private\n");
    let _ = writeln!(
        source,
        "public :: shell_{index:03}, build_{index:03}, measure_{index:03}"
    );
    let _ = writeln!(
        source,
        "type :: inner_{index:03}\n\
         real(BENCH_KIND_WP) :: alpha\n\
         real(BENCH_KIND_WP) :: beta\n\
         type(point_t) :: origin\n\
         end type inner_{index:03}"
    );
    let _ = writeln!(
        source,
        "type :: shell_{index:03}\n\
         type(inner_{index:03}) :: core\n\
         type(record_t) :: entry\n\
         integer :: count\n\
         end type shell_{index:03}"
    );
    source.push_str("contains\n");
    // Component chains are the expensive lookup: each link resolves a type in
    // another module before the member itself can be spelled.
    let _ = writeln!(
        source,
        "subroutine build_{index:03}(self, seed)\n\
         type(shell_{index:03}), intent(inout) :: self\n\
         real(bench_kind_wp), intent(in) :: seed\n\
         self%core%alpha = seed\n\
         self%core%beta = seed * 2.0_bench_kind_wp\n\
         self%core%origin%x = seed + 1.0_bench_kind_wp\n\
         self%core%origin%y = self%core%origin%x * 2.0_bench_kind_wp\n\
         self%core%origin%z = self%core%origin%y - self%core%alpha\n\
         self%entry%at%z = self%core%origin%z\n\
         self%entry%label = 'built'\n\
         self%count = self%count + 1\n\
         call register(self%entry)\n\
         end subroutine build_{index:03}"
    );
    let _ = writeln!(
        source,
        "function measure_{index:03}(self) result(total)\n\
         type(shell_{index:03}), intent(in) :: self\n\
         real(bench_kind_wp) :: total\n\
         integer :: i\n\
         total = 0.0_bench_kind_wp\n\
         do i = 1, self%count\n\
         if (self%core%alpha > self%core%beta) then\n\
         total = total + self%core%origin%x\n\
         else\n\
         total = total + self%core%origin%y\n\
         end if\n\
         end do\n\
         end function measure_{index:03}"
    );
    if index > 0 {
        let _ = writeln!(
            source,
            "subroutine link_{index:03}(self, other)\n\
             type(shell_{index:03}), intent(inout) :: self\n\
             type(neighbour_{index:03}), intent(in) :: other\n\
             self%core%alpha = other%core%alpha\n\
             self%entry%at%x = other%entry%at%x\n\
             end subroutine link_{index:03}"
        );
    }
    let _ = writeln!(source, "end module {name}");
    source.into_bytes()
}
