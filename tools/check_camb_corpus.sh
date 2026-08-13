#!/bin/sh
# Format every file of the CAMB corpus and explain what changes.
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
#     binary     default ./target/release/forformat
#     mode       full (default) | indent-only
#
# Acceptance (Gate D): zero non-idempotent files, and every long line either
# wrapped or explicitly classified as unwrappable. The `differing` and
# `changed lines` totals are an expected-explained baseline, not a failure
# condition; this route has no project-wide declaration context.
#
# `differing` counts files whose output differs from the *input*, which is a
# statement about CAMB being a fixed point of **stdin** formatting.  That is no
# longer quite true and it is not a correctness claim: a name such as `CP%TCMB`
# is declared in another file, so with no project context neither we nor the
# frozen oracle can resolve its spelling, and both make the same different
# choice.  It stays here as a signal, but it does not gate.
#
# `differing 1` is that signal, and it is expected: Interpolation.f90 has an
# authored comment indented past its construct.  We follow the oracle and
# reindent it, so our output differs from the input while *matching* the
# reference.  Preserving the authored column instead would restate the input at
# the cost of I2, since indent-only reindents the comment on the next pass.
#
# The claims that do gate are measured elsewhere:
#
#     python3 tools/reference/differential.py --perturbation none CAMB/...
#         our unperturbed stdin output against the oracle's.  Two files still
#         differ (halofit.f90, results.f90) and both are the cross-file
#         declaration case above, not an indentation defect.
#     python3 tools/check_invariants.py
#         I1 and I2 over every perturbation; must be zero across the board
#     python3 tools/check_project_mode.py
#         project mode reports only the documented first-run corrections
set -eu

CAMB=${1:-./CAMB}
BIN=${2:-./target/release/forformat}
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

# A nonzero exit keeps this usable in CI once the port is complete.  See the
# header for why `differing` is reported but not gated.  Declines *are* gated:
# AGENTS.md states the corpus wants zero decline diagnostics, and a decline is
# a long line the wrapper refused, so leaving it ungated let the acceptance
# criterion drift without failing anything.
[ "$not_idempotent" -eq 0 ] && [ "$declined" -eq 0 ]
