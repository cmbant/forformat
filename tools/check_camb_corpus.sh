#!/bin/sh
# Format every file of the CAMB corpus and report what changes.
#
# The corpus is a *developer verification target*, not test data: CAMB sources
# are never vendored, never committed as goldens and never wired into
# `cargo test`.  The workflow on any difference is to reduce it to a minimal
# snippet, add a fixture and a manifest row, fix it, and rerun this.
#
# This script never writes into the CAMB tree.
#
#   tools/check_camb_corpus.sh [camb-path] [binary] [mode]
#
#     camb-path  default ./CAMB
#     binary     default ./target/release/findent
#     mode       full (default) | indent-only
#
# Acceptance (Gate D): zero differing files, and every long line either wrapped
# or explicitly classified as unwrappable.
set -eu

CAMB=${1:-./CAMB}
BIN=${2:-./target/release/findent}
MODE=${3:-full}

# The CAMB profile, duplicated verbatim from scripts/findent_fortran.py.
PROFILE="--indent=4 --indent_module=0 --indent_procedure=0 --start_indent=4 \
--indent_contains=0 --openmp=0 --indent_contains=restart --indent_select=4 \
--indent_case=4 --indent_interface=0 --indent_continuation=4 --indent_ampersand"

case "$MODE" in
    full) ARGS="--full $PROFILE" ;;
    indent-only) ARGS="--indent-only $PROFILE" ;;
    *) echo "unknown mode: $MODE" >&2; exit 2 ;;
esac

[ -x "$BIN" ] || { echo "no binary at $BIN (cargo build --release)" >&2; exit 2; }
[ -d "$CAMB/fortran" ] || { echo "no corpus at $CAMB/fortran" >&2; exit 2; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

files=0
differing=0
changed_lines=0
not_idempotent=0
declined=0
longest=0
longest_file=""

# `fortran/` and `forutils/` are analyzed together: forutils supplies the shared
# modules and types whose declarations determine case resolution in fortran/.
for f in "$CAMB"/fortran/*.f90 "$CAMB"/fortran/tests/*.f90 \
         "$CAMB"/forutils/*.f90 "$CAMB"/forutils/tests/*.f90; do
    [ -f "$f" ] || continue
    files=$((files + 1))
    out="$WORK/out"
    twice="$WORK/twice"
    diag="$WORK/diag"
    # shellcheck disable=SC2086
    $BIN $ARGS < "$f" > "$out" 2> "$diag"
    if [ -s "$diag" ]; then
        count=$(wc -l < "$diag")
        declined=$((declined + count))
        while IFS= read -r message; do
            printf 'DECLINED         %s  %s\n' "$f" "$message"
        done < "$diag"
    fi
    # shellcheck disable=SC2086
    $BIN $ARGS < "$out" > "$twice" 2> "$diag"

    if ! cmp -s "$out" "$twice"; then
        not_idempotent=$((not_idempotent + 1))
        echo "NOT IDEMPOTENT  $f"
    fi
    if ! cmp -s "$f" "$out"; then
        differing=$((differing + 1))
        n=$(diff "$f" "$out" | grep -c '^[<>]' || true)
        changed_lines=$((changed_lines + n))
        printf 'DIFFERS %6s lines  %s\n' "$n" "$f"
    fi
    width=$(awk '{ if (length($0) > m) m = length($0) } END { print m + 0 }' "$out")
    if [ "$width" -gt "$longest" ]; then
        longest=$width
        longest_file=$f
    fi
done

echo
echo "files              $files"
echo "differing          $differing"
echo "changed lines      $changed_lines"
echo "non-idempotent     $not_idempotent"
echo "decline diagnostics $declined"
echo "longest line       $longest  ($longest_file)"

# Gate D wants zero differing files; a nonzero exit keeps this usable in CI once
# the port is complete.
[ "$differing" -eq 0 ] && [ "$not_idempotent" -eq 0 ]
