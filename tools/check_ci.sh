#!/bin/sh
# Runs the same steps as the "checks" job in .github/workflows/ci.yml, in the
# same order, so a local failure here means CI would fail too. Keep this in
# sync with that job.
set -eu

cd "$(dirname "$0")/.."

echo "== cargo fmt --check =="
cargo fmt --check

echo "== cargo clippy --locked --all-targets -- -D warnings (/tmp/forformat-lint) =="
CARGO_TARGET_DIR=/tmp/forformat-lint cargo clippy --locked --all-targets -- -D warnings

echo "== cargo test --locked --all-targets =="
cargo test --locked --all-targets

echo "== check_fixture_syntax.sh =="
./tools/check_fixture_syntax.sh target/debug/forformat

echo "== check_fuzz_regression.sh =="
./tools/check_fuzz_regression.sh

echo "== check_cli_contract.sh =="
./tools/check_cli_contract.sh target/debug/forformat

echo "All CI checks passed."
