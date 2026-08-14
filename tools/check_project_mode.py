#!/usr/bin/env python3
"""Explain **project mode** differences from the reference, over CAMB.

`check_camb_corpus.sh` pipes every file through stdin, one at a time, with no
project context.  That is not how the tool is deployed: the CAMB pre-commit
hooks run findent and then `standardize_fortran` over the repository, where
declaration cases are resolved from *every* tracked source at once.  Nothing in
the standing checks has ever exercised that path, so a project-only case
resolution defect would be invisible to all of them.

This builds a throwaway git repository containing the files currently present
under `fortran/` and `forutils/`, runs the Rust binary over it in project mode,
and compares each file against the reference computed the way
`standardize_fortran.main` computes it for a multi-path run. An unperturbed
corpus must already be a fixed point of the Rust formatter, and Rust must agree
with the reference. Perturbed runs retain the differential diagnostics without
applying the input fixed-point requirement. The fixed-point snapshot is raw
bytes, so newline or encoding changes cannot compare equal through text-mode
translation.

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


def decode_source(source: bytes) -> str:
    """Decode arbitrary source bytes while matching read_text's newline view."""
    return source.decode(errors="surrogateescape").replace("\r\n", "\n").replace("\r", "\n")


def encode_perturbation(source: bytes, perturbation: str) -> bytes:
    """Apply a text perturbation without changing a uniform CRLF file to LF."""
    text = source.decode(errors="surrogateescape")
    perturbed = D.apply(text, perturbation, 1)
    if b"\r\n" in source and source.count(b"\n") == source.count(b"\r\n"):
        perturbed = perturbed.replace("\r\n", "\n").replace("\n", "\r\n")
    return perturbed.encode(errors="surrogateescape")


def corpus_files(camb: Path) -> list[Path]:
    return [
        path
        for directory in SUBDIRECTORIES
        for path in sorted((camb / directory).glob("*.f90"))
    ]


def build_repository(
    camb: Path, files: list[Path], destination: Path, perturbation: str | None = None
) -> dict[Path, Path]:
    """Copy the corpus into a fresh git checkout, preserving its layout.

    With a perturbation the whole project is moved off the fixed point before
    anything runs, so project-wide case resolution has to be *right* rather
    than merely quiet: CAMB is already a fixed point, and against a fixed
    point a correct resolver and a dead one score identically.
    """
    mapping: dict[Path, Path] = {}
    for path in files:
        relative = path.relative_to(camb)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if perturbation:
            target.write_bytes(encode_perturbation(path.read_bytes(), perturbation))
        else:
            shutil.copyfile(path, target)
        mapping[relative] = target
    for command in (["git", "init", "-q"], ["git", "add", "-A"]):
        subprocess.run(command, cwd=destination, check=True, capture_output=True)
    return mapping


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--camb", type=Path, default=ROOT / "CAMB")
    parser.add_argument("--binary", default=str(ROOT / "target/release/forformat"))
    parser.add_argument("--findent", default="findent")
    parser.add_argument("--show", type=int, default=3, help="differing lines to print per file")
    parser.add_argument(
        "--converge",
        action="store_true",
        default=False,
        help="compare against the reference's converged output (diagnostic)",
    )
    parser.add_argument(
        "--single-pass",
        action="store_false",
        dest="converge",
        help="compare against the historical first pass",
    )
    parser.add_argument("--perturbation", choices=D.ALL_PERTURBATIONS)
    args = parser.parse_args()

    files = corpus_files(args.camb)
    if not files:
        print(f"no corpus under {args.camb}", file=sys.stderr)
        return 2

    module = D.load_reference()
    D.VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)
    from pathlib import Path as _Path

    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory)
        mapping = build_repository(args.camb, files, workspace, args.perturbation)
        # `--all` rewrites in place, so capture the inputs before it runs.
        input_bytes = {
            relative: source.read_bytes()
            for relative, source in mapping.items()
        }

        result = subprocess.run(
            [args.binary, "--full", *D.FINDENT_ARGS, "--all"],
            cwd=workspace,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"binary exited {result.returncode}: {result.stderr.strip()}", file=sys.stderr)
            return 2

        actual_bytes = {
            relative: (workspace / relative).read_bytes()
            for relative in input_bytes
        }
        fixed_point_changes = [
            relative
            for relative in input_bytes
            if actual_bytes[relative] != input_bytes[relative]
        ]
        if not args.perturbation:
            for relative in fixed_point_changes:
                print(f"NOT FIXED POINT  {relative}")

        # The reference composition is P(R(x)); the default target is its
        # historical single pass, while --converge is a diagnostic for
        # investigating first-pass instability.
        current = {
            _Path(str(relative)): decode_source(source)
            for relative, source in input_bytes.items()
        }
        actual = {
            relative: decode_source(source)
            for relative, source in actual_bytes.items()
        }
        for _ in range(10 if args.converge else 1):
            findented = {
                relative: D.run([args.findent, *D.FINDENT_ARGS], text)
                for relative, text in current.items()
            }
            targets = tuple(findented)
            cases = module.collect_declaration_cases(findented, target_paths=targets)
            expected = {
                relative: module.format_text(
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
                for relative in findented
            }
            if not args.converge or expected == current:
                break
            current = expected

        differing = 0
        changed_lines = 0
        pairs = 0
        more_than_case = 0
        for relative in sorted(findented, key=str):
            expected_text = expected[relative]
            actual_text = actual[relative]
            if actual_text == expected_text:
                continue
            lines = [
                line
                for line in difflib.unified_diff(
                    expected_text.splitlines(), actual_text.splitlines(),
                    "reference", "rust", lineterm="", n=0)
                if line.startswith(("-", "+")) and not line.startswith(("---", "+++"))
            ]
            differing += 1
            # Same acceptance rule as the differential: under an identifier
            # perturbation a residue of case-only differences is expected, but
            # anything more than case means a structural rule is wrong.
            left, right = expected_text.splitlines(), actual_text.splitlines()
            file_changed_lines = 0
            for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(
                None, left, right, autojunk=False
            ).get_opcodes():
                if tag == "equal":
                    continue
                file_changed_lines += max(i2 - i1, j2 - j1)
                for a, b in zip(left[i1:i2], right[j1:j2]):
                    pairs += 1
                    if a.lower() != b.lower():
                        more_than_case += 1
                        continue
            changed_lines += file_changed_lines
            print(f"{file_changed_lines:5d} lines  {relative}")
            for line in lines[: args.show * 2]:
                print("    " + line)

    print()
    print(f"files          {len(findented)}")
    print(f"differing      {differing}")
    print(f"changed lines  {changed_lines}")
    print(f"pairs          {pairs}")
    print(f"more than case {more_than_case}")
    if not args.perturbation:
        print(f"fixed-point changes {len(fixed_point_changes)}")
        return 1 if differing or fixed_point_changes else 0
    return 1 if more_than_case else 0


if __name__ == "__main__":
    raise SystemExit(main())
