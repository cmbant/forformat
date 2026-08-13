#!/usr/bin/env python3
"""Seed or refresh the traceability table from the frozen Python test suite.

Every one of the 90 reference tests needs a row with a terminal status before
Gate B can pass.  The rows are generated so the list cannot silently drift from
the suite; the *contents* of the last three columns are maintained by hand and
preserved across regenerations.

    python3 tools/gen_traceability.py            # create or refresh
    python3 tools/gen_traceability.py --check    # fail if a test has no row
"""

from __future__ import annotations

import argparse
import ast
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SUITE = ROOT / "tools" / "reference" / "test_standardize_fortran.py"
TARGET = ROOT / "docs" / "traceability.md"

# Class to default category, from the port plan §8.1 category list.
CATEGORY = {
    "CommandLineTests": "CLI/file-I/O",
    "FormattingTests": "blank-line/layout",
    "DeclarationCaseTests": "scope/project-case",
    "ContinuationTests": "continuation",
    "SpacingTests": "lexical",
    "RegressionFixTests": "lexical",
}

HEADER = """# Traceability: Python behaviour to Rust tests

One row per test in the frozen reference suite
(`tools/reference/test_standardize_fortran.py`, 90 terminal rows in 6 classes).  Gate B
of the port plan needs every row to carry a terminal status.

Statuses: `ported`, `covered by broader test`, `intentionally changed`
(with a rationale and a fixture), or `excluded` (with a row-specific scope reason).

Categories: lexical, case, scope/project-case, OpenMP, CPP/macro, comment,
continuation, wrapping, blank-line/layout, CLI/file-I/O, semantic-compile.

A `covered by broader test` row names the exact check and its regression signal.
Rows excluded at the Python-helper boundary name that boundary individually.
Golden cases belong in `tests/manifests/core.manifest`, using its existing
metadata.

Regenerate the row skeleton with `python3 tools/gen_traceability.py`; the last
three columns are hand-maintained and preserved. The added
`python_external_macro` fixture is derived from the frozen `-D SIZE` assertion.

| Python test | Category | Rust destination | Named Rust test | Status |
|---|---|---|---|---|
"""

ROW = re.compile(r"^\|\s*`?(?P<test>[A-Za-z0-9_.]+)`?\s*\|(?P<rest>.*)\|\s*$")


def existing_rows() -> dict[str, list[str]]:
    if not TARGET.exists():
        return {}
    rows = {}
    for line in TARGET.read_text().splitlines():
        match = ROW.match(line)
        if not match or match.group("test") in ("Python test", "---"):
            continue
        cells = [cell.strip() for cell in match.group("rest").split("|")]
        rows[match.group("test")] = cells
    return rows


def tests() -> list[tuple[str, str, int]]:
    tree = ast.parse(SUITE.read_text())
    found = []
    for node in tree.body:
        if not isinstance(node, ast.ClassDef):
            continue
        for item in node.body:
            if isinstance(item, ast.FunctionDef) and item.name.startswith("test"):
                found.append((node.name, item.name, item.lineno))
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    kept = existing_rows()
    lines = [HEADER]
    missing = []
    for class_name, test_name, lineno in tests():
        key = f"{class_name}.{test_name}"
        previous = kept.get(key)
        if previous is None:
            missing.append(key)
            cells = [CATEGORY.get(class_name, "lexical"), "—", "—", "todo"]
        else:
            cells = previous
        lines.append(f"| `{key}` | {' | '.join(cells)} |\n")
        _ = lineno

    body = "".join(lines)
    if args.check:
        if missing:
            print("tests with no traceability row:", *missing, sep="\n  ", file=sys.stderr)
            return 1
        return 0
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(body)
    print(f"wrote {TARGET} ({len(lines) - 1} rows, {len(missing)} new)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
