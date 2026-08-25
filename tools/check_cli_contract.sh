#!/usr/bin/env bash
set -euo pipefail

. "$(dirname "$0")/target_dir.sh"

binary=${1:-$(cargo_target_dir)/release/forformat}
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
expect_status 2 --continuation=0
expect_status 2 --output-format=fixed
expect_diagnostic 2 'forformat: invalid option: --not-an-option' --not-an-option
expect_diagnostic 2 'forformat: invalid option: --input-format=unknown' --input-format=unknown
expect_diagnostic 2 'forformat: invalid option: expected non-negative integer, got -1' --align_paren=-1
expect_diagnostic 2 'forformat: unsupported: fixed-form input/output is not supported' -ifixed

isolated_error='forformat: invalid option: --isolated requires one or more explicit paths and cannot be combined with --all or --all-files'
expect_diagnostic 2 "$isolated_error" --isolated --all
expect_diagnostic 2 "$isolated_error" --isolated --all-files

# Automatic fixed/free input detection is the default, so the legacy spellings
# that ask for it are accepted rather than rejected.
expect_status 0 -iauto
expect_status 0 --input-format=auto

alias_source=$'program p\nx = 1\nend program\n'
test "$(printf '%s' "$alias_source" | "$binary" --input_format=free)" = "$(printf '%s' "$alias_source" | "$binary" --input-format=free)"

# Numeric compatibility options also accept explicit booleans where their
# configuration representation is boolean/numeric.
expect_status 0 --no-config --align-paren=true
expect_status 0 --no-config --align-paren=false

for standard in f95 f2003 f2008 f2018 f2023; do
    expect_status 0 --no-config --target-standard="$standard"
done

# The F95 target is a formatter-output ceiling: it keeps the legacy constructor
# even when array-bracket modernization is explicitly enabled. F2003 retains
# the existing default modernization.
target_source=$'program p\ninteger :: x(2)\nx = (/1, 2/)\nend program\n'
printf '%s' "$target_source" | "$binary" --no-config --input-format=free \
    --target-standard=f95 --array-brackets=true >"$contract_tmp/f95.out"
grep -F '(/' "$contract_tmp/f95.out" >/dev/null
! grep -F '[' "$contract_tmp/f95.out" >/dev/null
printf '%s' "$target_source" | "$binary" --no-config --input-format=free \
    --target-standard=f2003 >"$contract_tmp/f2003.out"
grep -F '[' "$contract_tmp/f2003.out" >/dev/null

# The expected version comes from cargo, not from a literal, so bumping
# Cargo.toml is the only edit a release needs. `cargo pkgid` prints
# `…#forformat@<version>`.
version=${FORFORMAT_VERSION:-$(cargo pkgid | sed 's/.*[#@]//')}
test -n "$version"
test "$("$binary" --version)" = "forformat $version"
help=$("$binary" --help)
printf '%s\n' "$help" | grep -F 'Usage: forformat [OPTIONS]' >/dev/null
printf '%s\n' "$help" | grep -F -- '--last-indent, -lastindent' >/dev/null
printf '%s\n' "$help" | grep -F -- '--last-usable, -lastusable' >/dev/null
printf '%s\n' "$help" | grep -F -- '--target-standard=' >/dev/null
test "$(printf '' | "$binary" --last-indent)" = 0
test "$(printf 'program p\n' | "$binary" --last-usable)" = 1

broken_pipe_source="$contract_tmp/broken_pipe.f90"
for _ in {1..20000}; do
    printf 'x = 1\n'
done >"$broken_pipe_source"
set +e
"$binary" --stdout "$broken_pipe_source" 2>"$contract_tmp/broken_pipe.stderr" |
    head -n 1 >"$contract_tmp/broken_pipe.stdout"
pipe_status=${PIPESTATUS[0]}
set -e
test "$pipe_status" -eq 0
test ! -s "$contract_tmp/broken_pipe.stderr"

set +e
"$binary" -ifree < tests/fixtures/legacy_free_matrix.f90 | head -c 1 >/dev/null
pipe_status=${PIPESTATUS[0]}
set -e
test "$pipe_status" -eq 0

echo "CLI contract checks passed for $binary"
