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
