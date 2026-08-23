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

# Keep the libFuzzer targets exercised in CI. Each target gets its own disposable
# copy so one fuzzer cannot grow the next target's corpus or write into checked-in
# fixtures. A fixed seed makes this bounded smoke run reproducible.
corpus_root=$(mktemp -d)
trap 'rm -rf "$corpus_root"' EXIT
mkdir "$corpus_root/seed"
cp tests/fixtures/*.f90 "$corpus_root/seed/"
for target in regions declarations project wrapper properties; do
    target_corpus="$corpus_root/$target"
    mkdir "$target_corpus"
    cp "$corpus_root"/seed/* "$target_corpus/"
    if test "$target" = wrapper; then
        # Permanent seed for the separator-only case that previously exposed a
        # Document/SourceBuffer physical-line mismatch after semicolon removal.
        printf '\n;' > "$target_corpus/separator-only"
    fi
    cargo run --manifest-path fuzz/Cargo.toml --bin "$target" -- \
        -seed=1 -runs=64 -max_len=4096 "$target_corpus"
done
