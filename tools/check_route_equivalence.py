#!/usr/bin/env python3
"""Identical input bytes must produce identical output bytes on every route.

The tool can be reached three ways: bytes on stdin, a file named on the command
line with `--isolated` (no project context), and a file inside a repository with
project context.  The first two see exactly the same source and must agree
byte for byte; the third sees more information and may legitimately differ, but
only by identifier case.

Nothing else checks this.  `check_camb_corpus.sh` only ever uses stdin and
`check_project_mode.py` only ever uses the project route, so a divergence that
lives in the route itself is invisible to both.

Usage:
    python3 tools/check_route_equivalence.py [--camb CAMB] [--show N]
"""

from __future__ import annotations

import argparse
import difflib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE / "reference"))

import differential as D  # noqa: E402

SUBDIRECTORIES = ("fortran", "fortran/tests", "forutils", "forutils/tests")


def corpus_files(camb: Path) -> list[Path]:
    return [
        path
        for directory in SUBDIRECTORIES
        for path in sorted((camb / directory).glob("*.f90"))
    ]


def differing_lines(left: str, right: str) -> list[str]:
    return [
        line
        for line in difflib.unified_diff(
            left.splitlines(), right.splitlines(), "stdin", "file", lineterm="", n=0
        )
        if line.startswith(("-", "+")) and not line.startswith(("---", "+++"))
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--camb", type=Path, default=ROOT / "CAMB")
    parser.add_argument("--binary", default=str(ROOT / "target/release/forformat"))
    parser.add_argument("--show", type=int, default=3)
    parser.add_argument(
        "--perturbation",
        choices=D.ALL_PERTURBATIONS,
        help="move the input off the fixed point first; without this the sweep only "
        "exercises rules that CAMB already satisfies, which is most of them",
    )
    args = parser.parse_args()

    files = corpus_files(args.camb)
    if not files:
        print(f"no corpus under {args.camb}", file=sys.stderr)
        return 2

    common = ["--full", *D.FINDENT_ARGS]
    differing = 0
    changed = 0

    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory)
        for path in files:
            # A lone file in a bare directory: same bytes, no project context.
            scratch = workspace / path.name
            if args.perturbation:
                text = path.read_text(errors="surrogateescape")
                source = D.apply(text, args.perturbation, 1).encode(errors="surrogateescape")
                scratch.write_bytes(source)
            else:
                shutil.copyfile(path, scratch)
                source = path.read_bytes()

            piped = subprocess.run(
                [args.binary, *common], input=source, capture_output=True
            )
            named = subprocess.run(
                [args.binary, *common, "--stdout", "--isolated", path.name],
                cwd=workspace,
                capture_output=True,
            )
            scratch.unlink()

            if piped.returncode != 0 or named.returncode != 0:
                print(f"exit {piped.returncode}/{named.returncode}  {path.name}")
                differing += 1
                continue
            if piped.stdout == named.stdout:
                continue

            lines = differing_lines(
                piped.stdout.decode(errors="surrogateescape"),
                named.stdout.decode(errors="surrogateescape"),
            )
            differing += 1
            changed += len(lines)
            print(f"{len(lines):5d} lines  {path.relative_to(args.camb)}")
            for line in lines[: args.show * 2]:
                print("    " + line)

    print()
    print(f"files          {len(files)}")
    print(f"differing      {differing}")
    print(f"changed lines  {changed}")
    return 1 if differing else 0


if __name__ == "__main__":
    raise SystemExit(main())
