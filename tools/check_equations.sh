#!/bin/sh
set -eu

binary=${1:-target/release/findent}
input=${2:-equations.f90}
expected=${3:-equations.f90.indented}
work=$(mktemp -d "${TMPDIR:-/tmp}/findent-equations.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

"$binary" -ifree < "$input" > "$work/rust.out"
cmp -s "$work/rust.out" "$expected" || {
    echo "findent: equations output differs from $expected" >&2
    diff -u "$expected" "$work/rust.out" | sed -n '1,120p' >&2 || true
    exit 1
}

"$binary" -ifree < "$work/rust.out" > "$work/again.out"
cmp -s "$work/rust.out" "$work/again.out" || {
    echo "findent: equations output is not idempotent" >&2
    exit 1
}

if [ -x /opt/findent/src/findent ]; then
    env -u FINDENT_FLAGS LC_ALL=C /opt/findent/src/findent -ifree < "$input" > "$work/oracle.out"
    cmp -s "$work/rust.out" "$work/oracle.out" || {
        echo "findent: equations output differs from findent 4.3.7" >&2
        diff -u "$work/oracle.out" "$work/rust.out" | sed -n '1,120p' >&2 || true
        exit 1
    }
fi

lines=$(wc -l < "$input")
bytes=$(wc -c < "$input")
echo "equations check passed: $lines lines, $bytes bytes"
