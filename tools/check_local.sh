#!/bin/sh
# Run the complete local verification bar in a safe, deterministic order.
# Clippy uses a separate /tmp target directory because its metadata-only
# artifacts must not share Cargo's dev-profile fingerprint slots with cargo test.
set -eu

cd "$(dirname "$0")/.."
. "$(dirname "$0")/target_dir.sh"
target_dir=$(cargo_target_dir)

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo clippy --locked --all-targets -- -D warnings (/tmp/forformat-lint) =="
CARGO_TARGET_DIR=/tmp/forformat-lint cargo clippy --locked --all-targets -- -D warnings

echo "== RUSTDOCFLAGS=-D warnings cargo doc --locked --no-deps =="
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps

echo "== cargo test --locked --all-targets ($target_dir) =="
cargo test --locked --all-targets

echo "== check_fixture_syntax.sh =="
./tools/check_fixture_syntax.sh "$target_dir/debug/forformat"

echo "== check_fuzz_regression.sh =="
./tools/check_fuzz_regression.sh

echo "== check_cli_contract.sh (debug) =="
./tools/check_cli_contract.sh "$target_dir/debug/forformat"

echo "== check_docs.sh =="
./tools/check_docs.sh "$target_dir/debug/forformat"

echo "== cargo build --locked --release =="
cargo build --locked --release

echo "== check_cli_contract.sh (release) =="
./tools/check_cli_contract.sh "$target_dir/release/forformat"

echo "== check_package.sh =="
./tools/check_package.sh

echo "All local checks passed."
