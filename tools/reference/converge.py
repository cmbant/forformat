#!/usr/bin/env python3
"""Compose findent 4.3.7 with the frozen Python formatter until output stops changing.

The reference pipeline is two tools run **once** by pre-commit, with no
iteration.  It converges today only because the corpus is already a joint fixed
point.  This driver makes that assumption checkable: for any input it reports
whether `P(R(x))` reaches a fixed point, and of which kind.

Classification, from the port plan §2.4:

  strong          R(x) == x and P(x) == x — what the Rust formatter targets
  composition     P(R(x)) == x but one tool alone changes x, which means the two
                  tools disagree about who owns a formatting decision (almost
                  always continuation layout) and needs a design decision
  cycle           the composition oscillates
  non-convergent  still moving when the iteration cap is reached

Usage:

    tools/reference/converge.py FILE...
    tools/reference/converge.py --json report.json FILE...
    tools/reference/converge.py --no-wrap -D MPI FILE
    tools/reference/converge.py --project CAMB FILE...   # project case context
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
MAX_ITERATIONS = 10

# The CAMB profile, verbatim from findent_fortran.py.  `--indent_contains`
# appears twice on purpose; the last wins.
FINDENT_ARGS = [
    "--indent=4", "--indent_module=0", "--indent_procedure=0", "--start_indent=4",
    "--indent_contains=0", "--openmp=0", "--indent_contains=restart", "--indent_select=4",
    "--indent_case=4", "--indent_interface=0", "--indent_continuation=4", "--indent_ampersand",
]


def load_reference():
    spec = importlib.util.spec_from_file_location(
        "frozen_standardize_fortran", HERE / "standardize_fortran.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def run_findent(text: str, binary: str, args: list[str]) -> str:
    result = subprocess.run(
        [binary, *args], input=text, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(f"{binary} exited {result.returncode}: {result.stderr.strip()}")
    return result.stdout


def digest(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8", "surrogateescape")).hexdigest()[:16]


def first_difference(before: str, after: str) -> tuple[int, str, str] | None:
    b = before.splitlines()
    a = after.splitlines()
    for index in range(max(len(b), len(a))):
        left = b[index] if index < len(b) else "<missing>"
        right = a[index] if index < len(a) else "<missing>"
        if left != right:
            return index + 1, left, right
    return None


def categorize(left: str, right: str) -> str:
    """A coarse label for the first difference, enough to triage a report."""
    if left.strip() == right.strip():
        return "indentation"
    if left.replace(" ", "") == right.replace(" ", ""):
        return "spacing"
    if left.lower() == right.lower():
        return "case"
    if left.rstrip().endswith("&") or right.rstrip().endswith("&"):
        return "continuation"
    if "<missing>" in (left, right):
        return "line-count"
    return "other"


def converge(text: str, module, binary: str, options: dict, keep: Path | None):
    """Iterate P(R(x)) and classify the result."""
    findent_args = list(FINDENT_ARGS)
    format_kwargs = {"wrap": options["wrap"]}

    once_r = run_findent(text, binary, findent_args)
    once_p = module.format_text(text, **format_kwargs)
    strong = once_r == text and once_p == text

    current = text
    seen = {digest(current): 0}
    for iteration in range(1, MAX_ITERATIONS + 1):
        after = module.format_text(run_findent(current, binary, findent_args), **format_kwargs)
        if keep is not None:
            (keep / f"iteration-{iteration}").write_text(after)
        if after == current:
            return {
                "status": "strong" if strong else "composition",
                "iterations": iteration,
                "strong_fixed_point": strong,
                "output_hash": digest(after),
            }, after
        mark = digest(after)
        if mark in seen:
            difference = first_difference(current, after)
            return {
                "status": "cycle",
                "iterations": iteration,
                "cycle_length": iteration - seen[mark],
                "strong_fixed_point": False,
                "first_difference": difference,
                "category": categorize(difference[1], difference[2]) if difference else None,
            }, after
        seen[mark] = iteration
        current = after
    difference = first_difference(current, after)
    return {
        "status": "non-convergent",
        "iterations": MAX_ITERATIONS,
        "strong_fixed_point": False,
        "first_difference": difference,
        "category": categorize(difference[1], difference[2]) if difference else None,
    }, current


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("files", nargs="+", type=Path)
    parser.add_argument("--binary", default="findent", help="findent 4.3.7 binary")
    parser.add_argument("--json", type=Path, help="write a baseline report here")
    parser.add_argument("--no-wrap", dest="wrap", action="store_false")
    parser.add_argument("-D", "--define", action="append", default=[], metavar="NAME[=VALUE]")
    parser.add_argument("--project", type=Path, help="repository root for project case context")
    parser.add_argument("--keep", type=Path, help="directory for intermediate captures")
    parser.add_argument("--diff", action="store_true", help="print the first differing lines")
    args = parser.parse_args()

    module = load_reference()
    if args.project is not None:
        # `standardize_fortran` discovers project sources relative to the
        # repository it is run from, so the context is selected by cwd.
        os.chdir(args.project)

    rows = []
    failures = 0
    for path in args.files:
        text = path.read_text(errors="surrogateescape")
        keep = None
        if args.keep is not None:
            keep = args.keep / path.name
            keep.mkdir(parents=True, exist_ok=True)
        try:
            report, output = converge(text, module, args.binary, {"wrap": args.wrap}, keep)
        except RuntimeError as error:
            report, output = {"status": "error", "message": str(error)}, text
        report |= {
            "name": str(path),
            "input_hash": digest(text),
            "options": {"wrap": args.wrap, "defines": args.define},
        }
        rows.append(report)
        status = report["status"]
        if status not in ("strong", "composition"):
            failures += 1
        print(f"{status:15s} {path}")
        if args.diff and status not in ("strong", "composition"):
            for line in difflib.unified_diff(
                text.splitlines(), output.splitlines(), "input", "converged", lineterm="", n=1
            ):
                print("  " + line)

    if args.json is not None:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(rows, indent=2) + "\n")
        print(f"wrote {args.json}")
    return 1 if failures else 0


if __name__ == "__main__":
    with tempfile.TemporaryDirectory():
        raise SystemExit(main())
