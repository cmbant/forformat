#!/bin/sh
# Runs the checks performed by the reusable workflow in
# .github/workflows/rust-checks.yml, in their local order, so a local failure
# here means CI would fail too. Keep this in sync with that workflow.
set -eu

cd "$(dirname "$0")/.."
. "$(dirname "$0")/target_dir.sh"
target_dir=$(cargo_target_dir)

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo clippy --locked --all-targets -- -D warnings (/tmp/forformat-lint) =="
CARGO_TARGET_DIR=/tmp/forformat-lint cargo clippy --locked --all-targets -- -D warnings

echo "== cargo test --locked --all-targets =="
cargo test --locked --all-targets

echo "== check_fixture_syntax.sh =="
./tools/check_fixture_syntax.sh "$target_dir/debug/forformat"

echo "== check_fuzz_regression.sh =="
./tools/check_fuzz_regression.sh

echo "== check_cli_contract.sh =="
./tools/check_cli_contract.sh "$target_dir/debug/forformat"

echo "All CI checks passed."
