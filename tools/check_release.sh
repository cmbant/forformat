#!/bin/sh
set -eu

binary=${1:-target/release/forformat}
test -x "$binary"

size=$(wc -c < "$binary")
test "$size" -lt 2097152

checksum=unavailable
if command -v sha256sum >/dev/null 2>&1; then
    checksum=$(sha256sum "$binary" | awk '{print $1}')
fi

if command -v file >/dev/null 2>&1; then
    file "$binary"
fi

iterations=${FORFORMAT_STARTUP_ITERATIONS:-50}
start=$(date +%s%N)
i=0
while test "$i" -lt "$iterations"; do
    "$binary" </dev/null >/dev/null
    i=$((i + 1))
done
finish=$(date +%s%N)
elapsed=$((finish - start))
average=$((elapsed / iterations))

rss=unavailable
time_bin=${FORFORMAT_TIME_BIN:-/usr/bin/time}
if test -x "$time_bin"; then
    time_output=$(mktemp)
    trap 'rm -f "$time_output"' EXIT HUP INT TERM
    "$time_bin" -f '%M' "$binary" </dev/null >/dev/null 2>"$time_output" || true
    rss=$(tail -n 1 "$time_output")
fi

echo "release check: bytes=$size sha256=$checksum startup_average_ns=$average peak_rss_kb=$rss"
