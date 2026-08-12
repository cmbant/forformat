#!/usr/bin/env python3
"""Adjudicate a perturbed project sweep against ground truth, not against the oracle.

`check_project_mode.py` and `differential.py` both compare us to the frozen
reference.  That comparison can say *whether* we differ; it cannot say who is
right, and once the governing-declaration rule made us diverge on purpose, "who
is right" is the only question worth asking.

The committed CAMB tree can answer it.  It is the spelling the authors wrote,
and both tools reproduce it byte for byte when it is left alone — so after
scrambling identifier case, the tool that restores *more* of it is the tool that
resolved more names correctly.  The decisive number is not the total, which is
dominated by names whose own declaration the perturbation scrambled, but the
per-line split: lines exactly one tool gets wrong.

    python3 tools/check_restoration.py --perturbation case

This is a development diagnostic, like the historic corpus.  It found B12 after
that defect had survived the entire port behind a comparison-form fold.
"""

from __future__ import annotations

import argparse
import difflib
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "reference"))

import check_project_mode as P  # noqa: E402
import differential as D  # noqa: E402


def distance(truth: dict[Path, str], produced: dict[Path, str]) -> int:
    """Differing diff lines between a tree and ground truth."""
    total = 0
    for relative, text in produced.items():
        total += sum(
            1
            for line in difflib.unified_diff(
                truth[Path(str(relative))].splitlines(), text.splitlines(), lineterm="", n=0
            )
            if line.startswith(("-", "+")) and not line.startswith(("---", "+++"))
        )
    return total


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--camb", type=Path, default=ROOT / "CAMB")
    # Absolute: the binary runs with a throwaway checkout as its working
    # directory, so a relative path would resolve against that.
    parser.add_argument("--binary", default=str(ROOT / "target/release/findent"))
    parser.add_argument("--findent", default="findent")
    parser.add_argument("--perturbation", required=True, choices=D.ALL_PERTURBATIONS)
    parser.add_argument("--show", type=int, default=10, help="disputed lines to print per side")
    args = parser.parse_args()

    files = P.corpus_files(args.camb)
    if not files:
        print(f"no corpus under {args.camb}", file=sys.stderr)
        return 2

    module = D.load_reference()
    D.VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)
    truth = {
        path.relative_to(args.camb): path.read_text(errors="surrogateescape") for path in files
    }

    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory)
        mapping = P.build_repository(args.camb, files, workspace, args.perturbation)
        inputs = {
            relative: source.read_text(errors="surrogateescape")
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
        ours = {
            relative: (workspace / str(relative)).read_text(errors="surrogateescape")
            for relative in inputs
        }

    # The reference composition is P(R(x)), single pass — the one that produced
    # the committed tree.
    findented = {
        relative: D.run([args.findent, *D.FINDENT_ARGS], text)
        for relative, text in inputs.items()
    }
    cases = module.collect_declaration_cases(findented, target_paths=tuple(findented))
    theirs = {
        Path(str(relative)): module.format_text(
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

    print(f"perturbation        {args.perturbation}")
    print(f"lines unrestored    ours {distance(truth, ours)}   reference {distance(truth, theirs)}")

    # Totals are dominated by names whose declaration the perturbation scrambled,
    # which neither tool can restore.  The split below is the part that decides.
    ours_only: list[tuple[Path, int, str, str]] = []
    reference_only: list[tuple[Path, int, str, str]] = []
    for relative in sorted(ours, key=str):
        expected = truth[Path(str(relative))].splitlines()
        mine = ours[relative].splitlines()
        other = theirs[Path(str(relative))].splitlines()
        if not len(expected) == len(mine) == len(other):
            continue
        for number, (want, got, alt) in enumerate(zip(expected, mine, other), 1):
            if got != want and alt == want:
                ours_only.append((relative, number, want, got))
            elif alt != want and got == want:
                reference_only.append((relative, number, want, alt))

    print(f"only we get wrong   {len(ours_only)}")
    print(f"only they get wrong {len(reference_only)}")
    for label, rows in (("ours", ours_only), ("reference", reference_only)):
        for relative, number, want, got in rows[: args.show]:
            print(f"  {label:9s} {relative}:{number}")
            print(f"    truth {want.strip()}")
            print(f"    {label:5s} {got.strip()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
