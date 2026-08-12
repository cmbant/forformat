#!/usr/bin/env python3
"""Check I1 and I2 over *perturbed* corpus input, not only pristine input.

`check_camb_corpus.sh` runs the invariants over CAMB as committed, which is
already a joint fixed point of findent and the reference — so it can only fail
if a pass actively breaks a file it should have left alone.  Every input that
would exercise convergence is excluded by construction.

This runs the same two invariants over each corpus file under every
perturbation `differential.py` knows about:

    I1   full(full(x)) == full(x)
    I2   indent_only(full(x)) == full(x)

With `--oracle`, a failing input is re-tested against the frozen reference
pipeline itself, which classifies the failure:

    ours       the reference converges in one pass and we do not — a bug
    inherited  the reference does not converge either, and we match its first
               pass — faithful, and a product decision rather than a defect

Usage:
    python3 tools/check_invariants.py [--oracle] [--perturbation NAME]...
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE / "reference"))

import differential as D  # noqa: E402


def corpus_files() -> list[Path]:
    return [
        path
        for directory in ("CAMB/fortran", "CAMB/forutils")
        for path in sorted((ROOT / directory).glob("*.f90"))
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(ROOT / "target/release/findent"))
    parser.add_argument("--findent", default="findent")
    parser.add_argument("--perturbation", action="append", choices=["none", *D.ALL_PERTURBATIONS])
    parser.add_argument("--oracle", action="store_true", help="classify failures against the reference")
    parser.add_argument("--show", type=int, default=1, help="failing files to name per perturbation")
    args = parser.parse_args()

    names = args.perturbation or ["none", *D.ALL_PERTURBATIONS]
    files = corpus_files()
    if not files:
        print("no corpus files found; nothing to check", file=sys.stderr)
        return 2

    module = None
    if args.oracle:
        module = D.load_reference()
        D.VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)

    def full(text: str) -> str:
        return D.run([args.binary, "--full", *D.FINDENT_ARGS], text)

    def indent_only(text: str) -> str:
        return D.run([args.binary, *D.FINDENT_ARGS], text)

    def oracle(text: str) -> str:
        return D.reference_format(module, D.run([args.findent, *D.FINDENT_ARGS], text))

    totals: dict[str, list[int]] = {}
    inherited_total = 0
    for name in names:
        failures = [0, 0]  # I1, I2
        shown = 0
        for path in files:
            text = path.read_text(errors="surrogateescape")
            perturbed = text if name == "none" else D.apply(text, name, 1)
            try:
                once = full(perturbed)
            except RuntimeError as error:
                print(f"  ERROR {name} {path}: {error}", file=sys.stderr)
                continue
            broken = []
            if full(once) != once:
                failures[0] += 1
                broken.append("I1")
            if indent_only(once) != once:
                failures[1] += 1
                broken.append("I2")
            if not broken:
                continue
            verdict = ""
            if args.oracle:
                reference_once = oracle(perturbed)
                inherited = oracle(reference_once) != reference_once
                verdict = "  [inherited]" if inherited else "  [ours]"
                inherited_total += inherited
            if shown < args.show:
                print(f"{name:11s} {'+'.join(broken):6s} {path.relative_to(ROOT)}{verdict}")
                shown += 1
        totals[name] = failures

    print()
    print(f"{'perturbation':12s} {'files':>6s} {'I1 fail':>8s} {'I2 fail':>8s}")
    failed = 0
    for name, (one, two) in totals.items():
        print(f"{name:12s} {len(files):6d} {one:8d} {two:8d}")
        failed += one + two
    if args.oracle and inherited_total:
        print(f"\n{inherited_total} failing file(s) are inherited: the reference does not converge either.")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
