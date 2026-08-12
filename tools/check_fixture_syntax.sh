#!/bin/sh
# G3: compile self-contained fixtures before and after full formatting.
set -eu

BIN=${1:-./target/debug/findent}
FIXTURES=${2:-tests/fixtures}

if ! command -v gfortran >/dev/null 2>&1; then
    echo "gfortran unavailable; G3 skipped"
    exit 0
fi
[ -x "$BIN" ] || { echo "no formatter binary at $BIN" >&2; exit 2; }
[ -d "$FIXTURES" ] || { echo "no fixture directory at $FIXTURES" >&2; exit 2; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

failed=0
checked=0
skipped=0

for source in "$FIXTURES"/*.f90; do
    [ -f "$source" ] || continue
    name=$(basename "$source")
    before_log="$WORK/$name.before.log"
    after="$WORK/$name.after.f90"
    after_log="$WORK/$name.after.log"

    if ! gfortran -ffree-form -ffree-line-length-none -fopenmp -fsyntax-only \
        -J"$WORK" "$source" > /dev/null 2> "$before_log"; then
        skipped=$((skipped + 1))
        echo "SKIP $source"
        continue
    fi

    if ! "$BIN" --full < "$source" > "$after"; then
        echo "FAIL formatter $source"
        failed=$((failed + 1))
        continue
    fi
    if ! gfortran -ffree-form -ffree-line-length-none -fopenmp -fsyntax-only \
        -J"$WORK" "$after" > /dev/null 2> "$after_log"; then
        echo "FAIL formatted syntax $source"
        sed 's/^/  /' "$after_log"
        failed=$((failed + 1))
        continue
    fi
    checked=$((checked + 1))
    echo "OK   $source"
done

echo
echo "G3 checked  $checked"
echo "G3 skipped  $skipped"
echo "G3 failed   $failed"
[ "$failed" -eq 0 ]
