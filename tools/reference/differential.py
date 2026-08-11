#!/usr/bin/env python3
"""Differential test: Rust `--full` against the frozen `P(R(x))` reference pipeline.

The CAMB corpus is already a joint fixed point, so formatting it proves only that
the Rust passes are *harmless*.  To prove they are *correct* the input has to be
moved off the fixed point first.  This script perturbs code bytes (never string,
comment, Hollerith or CPP bytes), then compares:

    reference:  standardize_fortran.format_text(findent(perturbed))
    candidate:  findent-rs --full (perturbed)

Usage:

    tools/reference/differential.py CAMB/fortran/*.f90
    tools/reference/differential.py --perturbation case --show 5 FILE...
    tools/reference/differential.py --list-perturbations
"""

from __future__ import annotations

import argparse
import difflib
import importlib.util
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

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


# --- perturbations ----------------------------------------------------------
#
# Each takes one line of *code* (already stripped of comment and literal spans)
# and returns a variant.  They must be reversible in the sense that a correct
# formatter maps the variant back onto the canonical spelling.

def perturb_spacing(code: str) -> str:
    code = re.sub(r"\s*([-+*/=<>])\s*", r"\1", code)
    return re.sub(r",\s*", ",", code)


def perturb_case(code: str) -> str:
    """Uppercase every identifier.

    This exercises project case resolution as much as keyword lowering, so its
    differences are dominated by whatever the declaration engine cannot see yet.
    Use `keywords` to isolate the per-line rules.
    """
    return re.sub(r"\b[A-Za-z_][A-Za-z0-9_]*\b", lambda m: m.group(0).upper(), code)


VOCABULARY: frozenset[str] = frozenset()


def perturb_keywords(code: str) -> str:
    """Uppercase only words the reference itself calls keywords or intrinsics.

    Declared names keep their spelling, so a difference here is a per-line
    lowering bug rather than a missing declaration extractor.
    """
    return re.sub(
        r"\b[A-Za-z_][A-Za-z0-9_]*\b",
        lambda m: m.group(0).upper() if m.group(0).lower() in VOCABULARY else m.group(0),
        code,
    )


def perturb_operators(code: str) -> str:
    # `=>` is a pointer assignment, not a comparison; corrupting it only produces
    # garbage input on which both formatters are entitled to disagree.
    code = code.replace("=>", "\x00")
    for modern, legacy in (("==", ".eq."), ("/=", ".ne."), (">=", ".ge."),
                           ("<=", ".le."), (">", ".gt."), ("<", ".lt.")):
        code = code.replace(modern, legacy)
    return code.replace("\x00", "=>")


def perturb_compound(code: str) -> str:
    for separated, joined in (("end if", "endif"), ("end do", "enddo"),
                              ("end select", "endselect"), ("go to", "goto")):
        code = re.sub(rf"\b{separated}\b", joined, code, flags=re.IGNORECASE)
    return code


def perturb_exponent(code: str) -> str:
    return re.sub(r"(\d)([ed])([-+]?\d)", lambda m: m.group(1) + m.group(2).upper() + m.group(3),
                  code, flags=re.IGNORECASE)


def perturb_mixed(code: str) -> str:
    """Everything at once, which is where rule interactions surface."""
    for step in (perturb_compound, perturb_operators, perturb_exponent, perturb_spacing):
        code = step(code)
    return code


PERTURBATIONS = {
    "spacing": perturb_spacing,
    "case": perturb_case,
    "keywords": perturb_keywords,
    "operators": perturb_operators,
    "compound": perturb_compound,
    "exponent": perturb_exponent,
    "mixed": perturb_mixed,
}


# --- protected-span aware application ---------------------------------------

def code_spans(line: str) -> list[tuple[int, int]]:
    """Byte ranges of `line` that are ordinary code: outside quotes and comments."""
    if line.lstrip().startswith(("#", "!")):
        return []
    spans, start, quote, index = [], 0, "", 0
    while index < len(line):
        character = line[index]
        if quote:
            if character == quote:
                quote = ""
                start = index + 1
        elif character in "'\"":
            spans.append((start, index))
            quote = character
        elif character == "!":
            spans.append((start, index))
            return [s for s in spans if s[1] > s[0]]
        index += 1
    if not quote:
        spans.append((start, len(line)))
    return [s for s in spans if s[1] > s[0]]


def apply(text: str, name: str, stride: int) -> str:
    transform = PERTURBATIONS[name]
    out = []
    for number, line in enumerate(text.splitlines()):
        if stride > 1 and number % stride:
            out.append(line)
            continue
        if line.rstrip().endswith("&") or line.lstrip().startswith("&"):
            out.append(line)  # continuations: leave the reflow question alone
            continue
        pieces, cursor = [], 0
        for begin, end in code_spans(line):
            pieces.append(line[cursor:begin])
            pieces.append(transform(line[begin:end]))
            cursor = end
        pieces.append(line[cursor:])
        out.append("".join(pieces))
    return "\n".join(out) + ("\n" if text.endswith("\n") else "")


def run(command: list[str], text: str) -> str:
    result = subprocess.run(command, input=text, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"{command[0]} exited {result.returncode}: {result.stderr.strip()}")
    return result.stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("files", nargs="*", type=Path)
    parser.add_argument("--binary", default=str(ROOT / "target/release/findent"))
    parser.add_argument("--findent", default="findent", help="findent 4.3.7 binary")
    parser.add_argument("--perturbation", action="append", choices=sorted(PERTURBATIONS))
    parser.add_argument("--stride", type=int, default=1, help="perturb every Nth line")
    parser.add_argument("--show", type=int, default=3, help="differing lines to print per file")
    parser.add_argument("--list-perturbations", action="store_true")
    args = parser.parse_args()

    if args.list_perturbations:
        print("\n".join(sorted(PERTURBATIONS)))
        return 0

    module = load_reference()
    global VOCABULARY
    VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)
    names = args.perturbation or sorted(PERTURBATIONS)
    totals = {name: [0, 0, 0] for name in names}  # files, differing files, differing lines

    for name in names:
        for path in args.files:
            text = path.read_text(errors="surrogateescape")
            perturbed = apply(text, name, args.stride)
            try:
                expected = module.format_text(run([args.findent, *FINDENT_ARGS], perturbed))
                actual = run([args.binary, "--full", *FINDENT_ARGS], perturbed)
            except RuntimeError as error:
                print(f"  ERROR {name} {path}: {error}", file=sys.stderr)
                continue
            totals[name][0] += 1
            if expected == actual:
                continue
            differing = [
                line for line in difflib.unified_diff(
                    expected.splitlines(), actual.splitlines(),
                    "reference", "rust", lineterm="", n=0)
                if line.startswith(("-", "+")) and not line.startswith(("---", "+++"))
            ]
            totals[name][1] += 1
            totals[name][2] += len(differing)
            print(f"{name:10s} {len(differing):5d} lines  {path}")
            for line in differing[: args.show * 2]:
                print("    " + line)

    print()
    print(f"{'perturbation':12s} {'files':>6s} {'differ':>7s} {'lines':>8s}")
    failed = 0
    for name, (files, differ, lines) in totals.items():
        print(f"{name:12s} {files:6d} {differ:7d} {lines:8d}")
        failed += differ
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
