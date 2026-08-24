#!/bin/sh
set -eu

# Deterministic, dependency-free smoke coverage for the same malformed and
# truncated corpus that seeds the cargo-fuzz targets. Normal CI uses this
# short regression; cargo-fuzz remains an optional deeper campaign.
cargo test --test properties fixture_prefixes_are_total_and_idempotent -- --exact
cargo test --test properties arbitrary_byte_inputs_are_total_without_utf8_assumptions -- --exact
cargo test --test properties arbitrary_non_ascii_bytes_in_comments_and_strings_are_transparent -- --exact
cargo test --test properties unknown_statements_do_not_invent_structural_depth -- --exact
cargo test --test properties keyword_case_mutations_preserve_fixture_indent_depth -- --exact
cargo test --test properties source_and_logical_group_spans_stay_inside_the_input -- --exact
cargo test format::stack::tests -- --nocapture
cargo test classify::recognizers::tests -- --nocapture
cargo test tests::malformed_digit_prefixes_do_not_mutate_label_or_construct_state -- --exact

# Keep the libFuzzer targets exercised in CI. libFuzzer is a coverage-guided
# fuzzer, so the targets have to be built with sanitizer coverage instrumentation
# -- without it the mutation loop runs blind, never grows a corpus, and reports
# "no interesting inputs were found so far".
#
# The flags below are the coverage subset of what `cargo fuzz` passes; they need
# no nightly toolchain. `--target` on the host triple is required, not cosmetic:
# it stops cargo from applying these flags to build scripts and proc macros,
# which have no libFuzzer runtime to link against.
#
# `-Coverflow-checks` is what turns a silently wrapping subtraction into a
# reported crash, so it stays on even though the rest of the build is a debug
# profile already.
host=$(rustc -vV | sed -n 's/^host: //p')
fuzz_target_dir=${FUZZ_TARGET_DIR:-${TMPDIR:-/tmp}/forformat-fuzz-target}

# Two modes, because they answer different questions.
#
# The default, `-runs=0`, means "execute the corpus once and stop": every fixture
# goes through every target and the answer is the same on every machine and every
# run. That reproducibility is what makes it safe as a required check.
#
# Setting FUZZ_TIME (seconds) or FUZZ_RUNS instead turns the mutation loop on.
# Which inputs it reaches varies run to run, so it can pass on one run and fail
# on the next -- but a failure is never a false alarm, and the crashing input is
# written to $FUZZ_ARTIFACTS, which reproduces it exactly. Treat a red mutating
# run as a bug report with the repro attached.
#
# `TODO.md` records what the last campaign found and has not been fixed yet;
# until those are fixed a mutating run rediscovers them at random, which is why
# CI runs this mode non-blocking.
runs=${FUZZ_RUNS:-0}
max_total_time=${FUZZ_TIME:-0}

# Crash artifacts land here rather than in the working tree: libFuzzer writes
# `crash-<sha>` next to its prefix, and the repository root is not the place for
# it. The directory is gitignored and the path is stable so CI can upload it.
artifacts=${FUZZ_ARTIFACTS:-$PWD/fuzz/artifacts}
mkdir -p "$artifacts"

CARGO_TARGET_DIR="$fuzz_target_dir" \
RUSTFLAGS="--cfg fuzzing \
-Cpasses=sancov-module \
-Cllvm-args=-sanitizer-coverage-level=4 \
-Cllvm-args=-sanitizer-coverage-inline-8bit-counters \
-Cllvm-args=-sanitizer-coverage-pc-table \
-Cllvm-args=-sanitizer-coverage-trace-compares \
-Cdebug-assertions \
-Coverflow-checks" \
    cargo build --locked --manifest-path fuzz/Cargo.toml --target "$host" --bins

# Each target gets its own disposable copy of the seed corpus so one fuzzer
# cannot grow the next target's corpus or write into checked-in fixtures.
corpus_root=$(mktemp -d)
trap 'rm -rf "$corpus_root"' EXIT
mkdir "$corpus_root/seed"
cp tests/fixtures/*.f90 "$corpus_root/seed/"
for target in scanner assembler classifier engine format regions declarations project wrapper properties; do
    target_corpus="$corpus_root/$target"
    mkdir "$target_corpus"
    cp "$corpus_root"/seed/* "$target_corpus/"
    if test "$target" = wrapper; then
        # Permanent seed for the separator-only case that previously exposed a
        # Document/SourceBuffer physical-line mismatch after semicolon removal.
        printf '\n;' > "$target_corpus/separator-only"
    fi
    if test "$runs" -eq 0 && test "$max_total_time" -eq 0; then
        echo "corpus sweep: $target"
    else
        echo "fuzzing $target (runs=$runs, max_total_time=${max_total_time}s)"
    fi
    "$fuzz_target_dir/$host/debug/$target" \
        -seed=1 -runs="$runs" -max_total_time="$max_total_time" \
        -max_len=4096 -print_final_stats=1 \
        -artifact_prefix="$artifacts/" "$target_corpus"
done
