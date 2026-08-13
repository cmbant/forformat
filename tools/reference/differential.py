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
import hashlib
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


def load_reference(path: Path | None = None):
    path = path or (HERE / "standardize_fortran.py")
    path = path.resolve()
    digest = hashlib.sha256(str(path).encode()).hexdigest()[:12]
    module_name = f"reference_{path.stem}_{digest}"
    spec = importlib.util.spec_from_file_location(
        module_name,
        path,
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
    # Every identifier, with no exceptions.  An exception list here is a list of
    # names the sweep can no longer test, and it is invisible in the totals.
    return re.sub(r"\b[A-Za-z_][A-Za-z0-9_]*\b", lambda m: m.group(0).upper(), code)


VOCABULARY: frozenset[str] = frozenset()


def perturb_keywords(code: str) -> str:
    """Uppercase only words the reference itself calls keywords or intrinsics.

    Declared names keep their spelling, so a difference here is a per-line
    lowering bug rather than a missing declaration extractor.
    """
    # No `%` exclusion and no pre-normalization of member spelling.  Members
    # whose names collide with keywords are exactly what this sweep exists to
    # test, and excusing them turns a resolution failure into a silent 0.
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
    # Join compound keywords, including the no-space shape that exercises both
    # halves of the `elseif(` normalization rule.
    for separated, joined in (("end if", "endif"), ("end do", "enddo"),
                              ("end select", "endselect"), ("go to", "goto"),
                              ("else where", "elsewhere")):
        code = re.sub(rf"\b{separated}\b", joined, code, flags=re.IGNORECASE)
    return re.sub(r"\belse\s*if\s*\(", "elseif(", code, flags=re.IGNORECASE)


def perturb_exponent(code: str) -> str:
    return re.sub(r"(\d)([ed])([-+]?\d)", lambda m: m.group(1) + m.group(2).upper() + m.group(3),
                  code, flags=re.IGNORECASE)


def perturb_mixed(code: str) -> str:
    """Everything at once, which is where rule interactions surface."""
    for step in (perturb_compound, perturb_operators, perturb_exponent, perturb_spacing):
        code = step(code)
    return code


def perturb_separators(code: str) -> str:
    """Mangle the whitespace around a declaration `::`.

    None of the perturbations above disturbs separator alignment, and CAMB is
    already a fixed point of the alignment pass, so without this one the pass is
    invisible to *both* standing checks — it scores identically whether it works
    or does nothing.  Only depth-0 `::` is touched, so an array stride triplet
    such as `a(1::2)` is left alone.
    """
    out: list[str] = []
    wide = len(code) % 2 == 0
    depth = index = 0
    while index < len(code):
        character = code[index]
        if character in "([":
            depth += 1
        elif character in ")]":
            depth -= 1
        if depth == 0 and code.startswith("::", index):
            while out and out[-1] in " \t":
                out.pop()
            out.append("     ::   " if wide else "::")
            index += 2
            while index < len(code) and code[index] in " \t":
                index += 1
            continue
        out.append(character)
        index += 1
    return "".join(out)


# --- whole-text perturbations -----------------------------------------------
#
# Blank-line structure is a property of the file, not of a line, so these bypass
# the per-line `code_spans` machinery.  They may only add or remove
# whitespace-only lines: no protected byte is ever touched.

def _cpp_continues(line: str) -> bool:
    return line.rstrip("\r\n").rstrip().endswith("\\")


def perturb_blanks(text: str) -> str:
    """Delete every blank line, forcing the reference to re-insert the ones its
    program-unit spacing rule requires."""
    lines = text.splitlines(keepends=True)
    kept: list[str] = []
    for index, line in enumerate(lines):
        if not line.strip() and not (index and _cpp_continues(lines[index - 1])):
            continue
        kept.append(line)
    return "".join(kept)


def perturb_blankruns(text: str) -> str:
    """Insert a three-blank-line run periodically, which `limit_blank_lines` must
    cap at two and program-unit spacing must collapse at a unit boundary."""
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    statements = 0
    for index, line in enumerate(lines):
        out.append(line)
        stripped = line.strip()
        if not stripped:
            continue
        statements += 1
        following = lines[index + 1] if index + 1 < len(lines) else ""
        unsafe = (
            stripped.endswith("&")
            or following.lstrip().startswith("&")
            or _cpp_continues(line)
            or stripped.startswith("#")
        )
        if statements % 9 == 0 and not unsafe and following:
            out.extend(["\n"] * 3)
    return "".join(out)


PERTURBATIONS = {
    "spacing": perturb_spacing,
    "case": perturb_case,
    "keywords": perturb_keywords,
    "operators": perturb_operators,
    "compound": perturb_compound,
    "exponent": perturb_exponent,
    "mixed": perturb_mixed,
    "separators": perturb_separators,
}

TEXT_PERTURBATIONS = {
    "blanks": perturb_blanks,
    "blankruns": perturb_blankruns,
}

ALL_PERTURBATIONS = sorted({*PERTURBATIONS, *TEXT_PERTURBATIONS})

# Opt-in only, so it never joins a default sweep and changes anyone's totals.
# Without it nothing compares our *unperturbed* output against the oracle:
# `check_camb_corpus.sh` compares against the input file rather than the oracle,
# which is a fixed-point claim and not the same statement at all.
PERTURBATIONS["none"] = lambda code: code
SELECTABLE_PERTURBATIONS = sorted({*ALL_PERTURBATIONS, "none"})


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
    if name in TEXT_PERTURBATIONS:
        return TEXT_PERTURBATIONS[name](text)
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


def reference_format(module, text: str, findent: str = "findent", converge: bool = False) -> str:
    """Run the reference the way its own CLI does.

    `format_text` takes the declaration case tables as *arguments*; calling it
    bare applies almost no declared casing, which silently turns the `case`
    perturbation into a comparison against a crippled reference.  This mirrors
    the `--stdin` branch of `standardize_fortran.main` exactly: one file, no
    project, every table passed through.
    """
    from pathlib import Path as _Path

    current = text
    for _ in range(10 if converge else 1):
        cases = module.collect_declaration_cases({_Path("<stdin>"): current})[_Path("<stdin>")]
        after = module.format_text(
            current,
            module_cases=cases.module_cases,
            symbol_cases=cases.symbol_cases,
            procedure_cases=cases.procedure_cases,
            scope_cases=cases.scope_cases,
            type_procedure_cases=cases.type_procedure_cases,
            type_component_cases=cases.type_component_cases,
            variable_type_cases=cases.variable_type_cases,
            type_component_type_cases=cases.type_component_type_cases,
        )
        if not converge or after == current:
            return after
        # The reference composition is P(x) = R(x) followed by the Python
        # formatter.  `text` has already gone through R once in main(), so
        # feed the next iteration through findent before applying Python.
        current = run([findent, *FINDENT_ARGS], after)
    return current


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
    parser.add_argument("--converge", action="store_true", default=False,
                        help="iterate the reference to its converged answer (diagnostic)")
    parser.add_argument("--single-pass", action="store_false", dest="converge",
                        help="compare against the reference's historical first pass (default)")
    parser.add_argument("--perturbation", action="append", choices=SELECTABLE_PERTURBATIONS)
    parser.add_argument("--stride", type=int, default=1, help="perturb every Nth line")
    parser.add_argument("--show", type=int, default=3, help="differing lines to print per file")
    parser.add_argument("--list-perturbations", action="store_true")
    args = parser.parse_args()

    if args.list_perturbations:
        print("\n".join(ALL_PERTURBATIONS))
        return 0

    module = load_reference()
    global VOCABULARY
    VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)
    names = args.perturbation or ALL_PERTURBATIONS
    totals = {name: [0, 0, 0] for name in names}  # files, differing files, differing lines

    for name in names:
        for path in args.files:
            text = path.read_text(errors="surrogateescape")
            perturbed = apply(text, name, args.stride)
            try:
                expected = reference_format(
                    module,
                    run([args.findent, *FINDENT_ARGS], perturbed),
                    args.findent,
                    args.converge,
                )
                actual = run([args.binary, "--full", *FINDENT_ARGS], perturbed)
            except RuntimeError as error:
                print(f"  ERROR {name} {path}: {error}", file=sys.stderr)
                continue
            totals[name][0] += 1
            # No per-sweep "comparison form" here.  Normalizing the output
            # before comparing it makes a sweep structurally unable to report
            # the very class of difference it exists to find: folding kind
            # suffixes hid B12, and folding `%member` spelling hid the whole
            # member-resolution category from the keyword sweep.  A known
            # defect is reported, not normalized away.
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
