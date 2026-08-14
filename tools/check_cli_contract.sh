#!/usr/bin/env bash
set -euo pipefail

binary=${1:-target/release/forformat}
test -x "$binary"

contract_tmp=$(mktemp -d)
trap 'rm -rf "$contract_tmp"' EXIT

expect_status() {
    expected=$1
    shift
    set +e
    "$binary" "$@" </dev/null >"$contract_tmp/stdout" 2>"$contract_tmp/stderr"
    actual=$?
    set -e
    test "$actual" -eq "$expected"
}

expect_diagnostic() {
    expected=$1
    expected_message=$2
    shift 2
    expect_status "$expected" "$@"
    test "$(cat "$contract_tmp/stderr")" = "$expected_message"
}

expect_status 2 -ifixed
expect_status 2 -ofixed
expect_status 2 -iauto
expect_status 2 --continuation=0
expect_status 2 --input-format=auto
expect_status 2 --output-format=fixed
expect_diagnostic 2 'forformat: invalid option: --not-an-option' --not-an-option
expect_diagnostic 2 'forformat: invalid option: --input-format=unknown' --input-format=unknown
expect_diagnostic 2 'forformat: invalid option: expected non-negative integer, got -1' --align_paren=-1
expect_diagnostic 2 'forformat: unsupported: fixed-form input/output is not supported' -ifixed

alias_source=$'program p\nx = 1\nend program\n'
test "$(printf '%s' "$alias_source" | "$binary" --input_format=free)" = "$(printf '%s' "$alias_source" | "$binary" --input-format=free)"

# The expected version comes from cargo, not from a literal, so bumping
# Cargo.toml is the only edit a release needs. `cargo pkgid` prints
# `…#forformat@<version>`.
version=${FORFORMAT_VERSION:-$(cargo pkgid | sed 's/.*[#@]//')}
test -n "$version"
test "$("$binary" --version)" = "forformat $version"
"$binary" --help | grep -F 'Usage: forformat [OPTIONS]' >/dev/null
test "$(printf '' | "$binary" --last-indent)" = 0
test "$(printf 'program p\n' | "$binary" --last-usable)" = 1

set +e
"$binary" -ifree < tests/fixtures/legacy_free_matrix.f90 | head -c 1 >/dev/null
pipe_status=${PIPESTATUS[0]}
set -e
test "$pipe_status" -eq 0

echo "CLI contract checks passed for $binary"
