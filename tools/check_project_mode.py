#!/usr/bin/env python3
"""Compare **project mode** against the reference, over the whole CAMB corpus.

`check_camb_corpus.sh` pipes every file through stdin, one at a time, with no
project context.  That is not how the tool is deployed: the CAMB pre-commit
hooks run findent and then `standardize_fortran` over the repository, where
declaration cases are resolved from *every* tracked source at once.  Nothing in
the standing checks has ever exercised that path, so a project-only case
resolution defect would be invisible to all of them.

This builds a throwaway git repository containing `fortran/` and `forutils/`,
runs the Rust binary over it in project mode, and compares each file against
the reference computed the way `standardize_fortran.main` computes it for a
multi-path run: one `collect_declaration_cases` over the whole file set, then
`format_text` per file with that file's tables.

The reference is fed findent output because in the real hook chain findent runs
first; the Rust binary does its own layout, so it is fed the original.

Usage:
    python3 tools/check_project_mode.py [--camb CAMB] [--show N]
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


def build_repository(camb: Path, files: list[Path], destination: Path) -> dict[Path, Path]:
    """Copy the corpus into a fresh git checkout, preserving its layout."""
    mapping: dict[Path, Path] = {}
    for path in files:
        relative = path.relative_to(camb)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        mapping[relative] = path
    for command in (["git", "init", "-q"], ["git", "add", "-A"]):
        subprocess.run(command, cwd=destination, check=True, capture_output=True)
    return mapping


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--camb", type=Path, default=ROOT / "CAMB")
    parser.add_argument("--binary", default=str(ROOT / "target/release/findent"))
    parser.add_argument("--findent", default="findent")
    parser.add_argument("--show", type=int, default=3, help="differing lines to print per file")
    args = parser.parse_args()

    files = corpus_files(args.camb)
    if not files:
        print(f"no corpus under {args.camb}", file=sys.stderr)
        return 2

    module = D.load_reference()
    from pathlib import Path as _Path

    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory)
        mapping = build_repository(args.camb, files, workspace)

        result = subprocess.run(
            [args.binary, "--full", *D.FINDENT_ARGS, "--all"],
            cwd=workspace,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"binary exited {result.returncode}: {result.stderr.strip()}", file=sys.stderr)
            return 2

        # The reference, exactly as `main` does it for a multi-path run.
        findented = {
            _Path(str(relative)): D.run([args.findent, *D.FINDENT_ARGS],
                                        source.read_text(errors="surrogateescape"))
            for relative, source in mapping.items()
        }
        targets = tuple(findented)
        cases = module.collect_declaration_cases(findented, target_paths=targets)

        differing = 0
        changed_lines = 0
        for relative in sorted(findented, key=str):
            expected = module.format_text(
                findented[relative],
                module_cases=cases[relative].module_cases,
                symbol_cases=cases[relative].symbol_cases,
                procedure_cases=cases[relative].procedure_cases,
                scope_cases=cases[relative].scope_cases,
                type_procedure_cases=cases[relative].type_procedure_cases,
                type_component_cases=cases[relative].type_component_cases,
                variable_type_cases=cases[relative].variable_type_cases,
                type_component_type_cases=cases[relative].type_component_type_cases,
            )
            actual = (workspace / str(relative)).read_text(errors="surrogateescape")
            if actual == expected:
                continue
            lines = [
                line
                for line in difflib.unified_diff(
                    expected.splitlines(), actual.splitlines(),
                    "reference", "rust", lineterm="", n=0)
                if line.startswith(("-", "+")) and not line.startswith(("---", "+++"))
            ]
            differing += 1
            changed_lines += len(lines)
            print(f"{len(lines):5d} lines  {relative}")
            for line in lines[: args.show * 2]:
                print("    " + line)

    print()
    print(f"files          {len(findented)}")
    print(f"differing      {differing}")
    print(f"changed lines  {changed_lines}")
    return 1 if differing else 0


if __name__ == "__main__":
    raise SystemExit(main())
