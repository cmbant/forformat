#!/usr/bin/env bash
# Install a built wheel into a clean environment and exercise the CLI contract
# through it.
#
#   tools/check_wheel.sh [wheel-directory]
#
#     wheel-directory  default ./dist
#
# Nothing downstream of the build re-tests the artifact, so the wheel that gets
# uploaded — and published — is the one exercised here: installed from the
# file, into an interpreter that has never seen this checkout. The checks are
# the ones a user would notice immediately if the packaged binary were missing,
# stale, or the wrong platform: the version string, stdin formatting, and the
# exit statuses `--check` promises.
set -euo pipefail

dist=${1:-dist}
test -d "$dist"

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Only the bootstrap needs an interpreter from outside; the venv always
# provides `python`.
python_bin=${PYTHON:-python}
command -v "$python_bin" >/dev/null 2>&1 || python_bin=python3
"$python_bin" -m venv "$work/venv"
# Windows interpreters put the scripts in `Scripts`, everything else in `bin`.
if [ -d "$work/venv/Scripts" ]; then
    venv_bin="$work/venv/Scripts"
else
    venv_bin="$work/venv/bin"
fi
export PATH="$venv_bin:$PATH"

# `--no-index` keeps the resolver off the network: the wheel under test is the
# only thing that can satisfy this install.
python -m pip install --quiet --no-index --find-links "$dist" forformat

version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
test -n "$version"
test "$(forformat --version)" = "forformat $version"

printf 'program p\nx=1\nend program p\n' > "$work/smoke.f90"
printf 'program p\nx=1\nend program p\n' | forformat > "$work/stdin.out"
grep -q '^   x = 1$' "$work/stdin.out"

status=0
forformat --isolated --check "$work/smoke.f90" >/dev/null || status=$?
test "$status" -eq 1
forformat --isolated "$work/smoke.f90" >/dev/null
forformat --isolated --check "$work/smoke.f90" >/dev/null

status=0
forformat --not-an-option </dev/null >/dev/null 2>"$work/stderr.txt" || status=$?
test "$status" -eq 2
grep -q 'invalid option: --not-an-option' "$work/stderr.txt"

echo "wheel check: forformat $version installed from $dist passed the CLI contract"
