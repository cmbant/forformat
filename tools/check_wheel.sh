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

# Licence files are part of the binary-distribution contract. Check the wheel
# itself rather than relying on setuptools' default discovery rules.
"$python_bin" - "$dist" <<'PY'
import sys
import zipfile
from pathlib import Path, PurePosixPath

wheels = sorted(Path(sys.argv[1]).glob("*.whl"))
if len(wheels) != 1:
    raise SystemExit(f"wheel check: expected exactly one wheel, found {len(wheels)}")
with zipfile.ZipFile(wheels[0]) as archive:
    basenames = {PurePosixPath(name).name for name in archive.namelist()}
required = {"LICENSE", "LICENSE-THIRD-PARTY", "NOTICE"}
missing = sorted(required - basenames)
if missing:
    raise SystemExit(f"wheel check: wheel is missing legal files: {', '.join(missing)}")
PY

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
status=0
forformat --isolated --check "$work/smoke.f90" >/dev/null || status=$?
test "$status" -eq 0

status=0
forformat --not-an-option </dev/null >/dev/null 2>"$work/stderr.txt" || status=$?
test "$status" -eq 2
grep -q 'invalid option: --not-an-option' "$work/stderr.txt"

mkdir "$work/repo"
git -C "$work/repo" init -q
printf 'indent = 8\n' > "$work/repo/.forformat.toml"
printf 'module SharedName\nend module SharedName\n' > "$work/repo/project.f90"
git -C "$work/repo" add -A
printf 'indent = 5\n' > "$work/explicit.toml"

(
cd "$work"
FORFORMAT_WHEEL_WORK="$work" python - <<'PY'
import os
import subprocess
import warnings
from pathlib import Path

from forformat import ForformatError, ForformatWarning, format_source

work = Path(os.environ["FORFORMAT_WHEEL_WORK"])
repo = work / "repo"
os.chdir(repo)

source = "program p\nx=1\nend program p\n"
formatted = format_source(source)
assert isinstance(formatted, str)
assert "\n   x = 1\n" in formatted

configured = format_source(
    source,
    options=("--config", str(work / "explicit.toml")),
)
assert "\n     x = 1\n" in configured

target = repo / "target.f90"
target.write_text("program p\ninteger :: StaleName\nend program p\n")
subprocess.run(["git", "add", "target.f90"], check=True)
contextual = format_source(
    "program p\nuse sharedname\nprint *, stalename\nend program p\n",
    filename=target,
    repo_context_path=repo,
)
assert "\n   use SharedName\n" in contextual
assert "stalename" in contextual
assert "StaleName" not in contextual

raw = b"program p\nprint *, 'x' ! \xff\nend program p\n"
raw_formatted = format_source(raw)
assert isinstance(raw_formatted, bytes)
assert b"\xff" in raw_formatted

with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    format_source(
        "program p\naaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nend program p\n",
        options=("--line-length=20",),
    )
assert any(issubclass(item.category, ForformatWarning) for item in caught)

try:
    format_source(source, options=("--not-an-option",))
except ForformatError as error:
    assert error.returncode == 2
    assert "invalid option" in error.stderr
else:
    raise AssertionError("invalid options must raise ForformatError")
PY
)

echo "wheel check: forformat $version installed from $dist passed the CLI and import contracts"
