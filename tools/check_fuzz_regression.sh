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

# Keep the libFuzzer targets exercised in CI.  The corpus is deliberately
# bounded: this is a smoke/property pass, while longer campaigns remain a
# developer activity.  Every target gets real input from the checked-in
# fixture corpus.
seed_corpus=$(mktemp -d)
trap 'rm -rf "$seed_corpus"' EXIT
cp tests/fixtures/*.f90 "$seed_corpus/"
for target in regions declarations project wrapper properties; do
    target_corpus="$seed_corpus"
    if test "$target" = properties; then
        # The property target is intentionally single-buffer: project-wide
        # case convergence belongs to the project target and is not available
        # through format_source(&[u8], ...).
        target_corpus=tests/fixtures
    fi
    cargo run --manifest-path fuzz/Cargo.toml --bin "$target" -- \
        -runs=64 -max_len=4096 "$target_corpus"
done
