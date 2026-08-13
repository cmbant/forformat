#!/usr/bin/env python3
"""Run the Rust formatter over genuinely unformatted real-world code.

Every standing check formats CAMB *after* the reformat commit, which is a joint
fixed point of findent and the Python formatter.  Against a fixed point a
correct rule and a dead rule score identically, so the perturbation sweeps exist
to move the input off it — but a synthetic perturbation only ever exercises the
axis it perturbs.  Real hand-written Fortran is off the fixed point on every
axis at once, in the proportions a human actually produces them.

CAMB has exactly such a corpus in its own history:

    3b1b6e08  "Add Fortran standardization tools"   <- pre  (hand-written)
    a1db7a71  "Fortran reformat"                    <- post (the tools' output)

with `forutils` at `c4b1e072` and `49bc7c36` respectively.  Extract both trees
somewhere outside this repository (nothing under `CAMB/` may be modified and no
`CAMB/` path may be referenced from `src/`, `tests/` or `benches/`) and point
this script at them:

    git -C CAMB archive 3b1b6e08 | tar -x -C /tmp/camb-pre
    git -C CAMB/forutils archive c4b1e072 | tar -x -C /tmp/camb-pre/forutils
    python3 tools/check_historic_corpus.py --pre /tmp/camb-pre

This is a **development diagnostic**, not a gate and not a unit-test source.
Its output is a category breakdown of where we diverge, ranked by how much a
divergence matters:

    other / line-count   a rule is wrong; look here first
    case                 an identifier is spelled differently
    spacing              intra-line whitespace
    indent               leading whitespace on an ordinary line
    continuation         leading whitespace on a wrapped line, or where a
                         statement was split -- accepted divergence when it is
                         consistent with findent's own continuation indent

With `--post DIR` it also checks the oracle against what CAMB actually
committed, which is the one way to find out whether the frozen reference really
is the thing that produced the repository we are matching.
"""

from __future__ import annotations

import argparse
import difflib
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE / "reference"))

import differential as D  # noqa: E402

SUBDIRECTORIES = ("fortran", "fortran/tests", "forutils", "forutils/tests")

CATEGORY_ORDER = (
    "other", "line-count", "array-constructor", "case", "spacing", "comment-content",
    "indent", "continuation"
)


def corpus_files(root: Path) -> list[Path]:
    return [
        path
        for directory in SUBDIRECTORIES
        for path in sorted((root / directory).glob("*.f90"))
    ]


def categorize(left: str, right: str) -> str:
    """Label one differing line pair, most-serious label wins.

    Order matters: a pair that differs in both case and indentation is a case
    difference, because that is the one that changes what the code says.
    """
    if left is None or right is None:
        return "line-count"
    # The frozen reference changes a multi-line `(/ ... /)` constructor's
    # opening delimiter to `[` but never reaches the closing `/)` on a later
    # physical line.  Rust rewrites the whole statement and emits `]`; this
    # reviewed oracle defect is useful to count separately from real drift.
    closing_mismatch = (
        left.rstrip().endswith("/)") and right.rstrip().endswith("]")
    ) or (
        right.rstrip().endswith("/)") and left.rstrip().endswith("]")
    )
    if closing_mismatch:
        return "array-constructor"
    left_comment = comment_start(left)
    right_comment = comment_start(right)
    if left_comment is not None and right_comment is not None:
        if left[:left_comment] == right[:right_comment]:
            return "comment-content"
    if left.strip() == right.strip():
        stripped = left.strip()
        # A comment line is not a continuation line.  Folding `!` in here moves
        # every mis-indented comment out of `indent`, which is a defect bucket,
        # and into `continuation`, which is an accepted-divergence bucket.
        if stripped.startswith("&") or stripped.endswith("&"):
            return "continuation"
        return "indent"
    if left.split() == right.split():
        return "spacing"
    # Compare with case intact before folding it, or a pair that differs only in
    # *where* the spaces are — `a=b` against `a = b` — lands in `case` and a
    # spacing defect is reported as identifier drift.
    if "".join(left.split()) == "".join(right.split()):
        return "spacing"
    if left.lower() == right.lower():
        return "case"
    if "".join(left.split()).lower() == "".join(right.split()).lower():
        return "case"
    if left.rstrip().endswith("&") or right.rstrip().endswith("&"):
        return "continuation"
    return "other"


def comment_start(line: str) -> int | None:
    """Return the unquoted Fortran comment marker, if this line has one."""
    quote = ""
    index = 0
    while index < len(line):
        character = line[index]
        if quote:
            if character == quote:
                if index + 1 < len(line) and line[index + 1] == quote:
                    index += 2
                    continue
                quote = ""
        elif character in "'\"":
            quote = character
        elif character == "!":
            return index
        index += 1
    return None


def pairs(expected: str, actual: str) -> list[tuple[str | None, str | None]]:
    left, right = expected.splitlines(), actual.splitlines()
    out: list[tuple[str | None, str | None]] = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(
        None, left, right, autojunk=False
    ).get_opcodes():
        if tag == "equal":
            continue
        span = max(i2 - i1, j2 - j1)
        for offset in range(span):
            a = left[i1 + offset] if i1 + offset < i2 else None
            b = right[j1 + offset] if j1 + offset < j2 else None
            out.append((a, b))
    return out


def reference_outputs(
    module, findent: str, sources: dict[Path, str], converge: bool, binary_findent_args: list[str]
) -> dict[Path, str]:
    """`P(R(x))` with project-wide tables, exactly as the hook computes it.

    With `converge` the composition is iterated to a fixed point instead of run
    once.  The reference pipeline itself does not iterate; the Rust formatter is
    specified to reach the converged answer in one pass, so this is the correct
    comparison target for it and the single pass is what the *old* hook would
    have written on its first run.
    """
    current = dict(sources)
    for _ in range(10 if converge else 1):
        findented = {
            path: D.run([findent, *binary_findent_args], text) for path, text in current.items()
        }
        cases = module.collect_declaration_cases(findented, target_paths=tuple(findented))
        after = {
            path: module.format_text(
                text,
                module_cases=cases[path].module_cases,
                symbol_cases=cases[path].symbol_cases,
                procedure_cases=cases[path].procedure_cases,
                scope_cases=cases[path].scope_cases,
                type_procedure_cases=cases[path].type_procedure_cases,
                type_component_cases=cases[path].type_component_cases,
                variable_type_cases=cases[path].variable_type_cases,
                type_component_type_cases=cases[path].type_component_type_cases,
            )
            for path, text in findented.items()
        }
        if not converge or after == current:
            return after
        current = after
    return current


def rust_stdin(binary: str, sources: dict[Path, str]) -> dict[Path, str]:
    out = {}
    for relative, text in sources.items():
        result = subprocess.run(
            [binary, "--full", *D.FINDENT_ARGS],
            input=text.encode(errors="surrogateescape"),
            capture_output=True,
        )
        if result.returncode != 0:
            raise RuntimeError(f"{binary} exited {result.returncode} on {relative}")
        out[relative] = result.stdout.decode(errors="surrogateescape")
    return out


def rust_project(binary: str, sources: dict[Path, str]) -> dict[Path, str]:
    """One `--all` run over a throwaway checkout of the whole tree.

    This is the deployed configuration and the one the reformat commit was made
    with, so it is the only comparison that is fair against a reference whose
    declaration tables are collected over every file at once.
    """
    with tempfile.TemporaryDirectory() as directory:
        workspace = Path(directory)
        for relative, text in sources.items():
            target = workspace / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, errors="surrogateescape")
        for command in (["git", "init", "-q"], ["git", "add", "-A"]):
            subprocess.run(command, cwd=workspace, check=True, capture_output=True)
        result = subprocess.run(
            [binary, "--full", *D.FINDENT_ARGS, "--all"],
            cwd=workspace, capture_output=True, text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(f"{binary} --all exited {result.returncode}: {result.stderr.strip()}")
        return {
            relative: (workspace / relative).read_text(errors="surrogateescape")
            for relative in sources
        }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pre", type=Path, required=True, help="unformatted tree (3b1b6e08)")
    parser.add_argument("--post", type=Path, help="committed reformat (a1db7a71), for oracle check")
    parser.add_argument("--binary", default=str(ROOT / "target/release/forformat"))
    parser.add_argument("--findent", default="findent")
    parser.add_argument("--converge", action="store_true",
                        help="compare against the reference's converged output rather than its "
                             "first pass")
    parser.add_argument("--show", type=int, default=0, help="differing pairs to print per file")
    parser.add_argument("--category", choices=CATEGORY_ORDER,
                        help="only print pairs in this category")
    parser.add_argument(
        "--mode",
        choices=("project", "stdin"),
        default="project",
        help="project (default) runs `--all` over the whole tree, which is how the hook that "
             "produced the reformat commit ran and the only fair comparison against a reference "
             "whose case tables are collected over every file at once",
    )
    args = parser.parse_args()

    files = corpus_files(args.pre)
    if not files:
        print(f"no corpus under {args.pre}", file=sys.stderr)
        return 2

    module = D.load_reference()
    D.VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)

    sources = {
        path.relative_to(args.pre): path.read_text(errors="surrogateescape") for path in files
    }
    expected = reference_outputs(module, args.findent, sources, args.converge, D.FINDENT_ARGS)

    if args.post is not None:
        agree = sum(
            1
            for relative in sources
            if (args.post / relative).exists()
            and (args.post / relative).read_text(errors="surrogateescape") == expected[relative]
        )
        print(f"oracle vs committed reformat: {agree}/{len(sources)} files identical")
        print()

    if args.mode == "project":
        produced = rust_project(args.binary, sources)
    else:
        produced = rust_stdin(args.binary, sources)

    totals: Counter[str] = Counter()
    differing = 0
    changed_lines = 0
    trailing = 0
    per_file: list[tuple[int, Path, Counter[str]]] = []

    for relative in sorted(sources, key=str):
        got = produced[relative]
        if got.endswith("\n") != expected[relative].endswith("\n"):
            trailing += 1
        if got == expected[relative]:
            continue
        differing += 1
        counts: Counter[str] = Counter()
        shown = 0
        for a, b in pairs(expected[relative], got):
            # No "already settled" filter here.  One lived in this loop and
            # dropped a case pair whenever our spelling appeared, case-
            # insensitively, on any declaration-like line in any file of the
            # project — which is almost always — so the `case` bucket read 0
            # while 21 pairs were live.  This diagnostic reports; declarations
            # adjudicate, in `adjudicate_case.py`.
            label = categorize(a, b)
            counts[label] += 1
            if shown < args.show and (args.category is None or args.category == label):
                if shown == 0:
                    print(f"  {relative}")
                print(f"    {label:12s} - {a}")
                print(f"    {'':12s} + {b}")
                shown += 1
        changed_lines += sum(counts.values())
        totals.update(counts)
        per_file.append((sum(counts.values()), relative, counts))

    per_file.sort(reverse=True, key=lambda row: (row[2]["other"] + row[2]["line-count"], row[0]))
    print(f"{'pairs':>6s} {'other':>6s} {'lines':>6s} {'array':>6s} {'case':>6s} "
          f"{'space':>6s} {'comment':>7s} {'indent':>6s} {'cont':>6s}  file")
    for total, relative, counts in per_file:
        print(f"{total:6d} {counts['other']:6d} {counts['line-count']:6d} "
              f"{counts['array-constructor']:6d} {counts['case']:6d} {counts['spacing']:6d} "
              f"{counts['comment-content']:7d} {counts['indent']:6d} {counts['continuation']:6d}  "
              f"{relative}")

    print()
    print(f"mode           {args.mode}{' (converged reference)' if args.converge else ''}")
    print(f"files          {len(sources)}")
    print(f"differing      {differing}")
    print(f"changed pairs  {changed_lines}")
    if trailing:
        print(f"trailing eol   {trailing} files differ in final newline only")
    for label in CATEGORY_ORDER:
        print(f"  {label:12s} {totals[label]}")
    # `other` and `line-count` are the only categories that are never acceptable.
    # Array constructors and comment bodies are reviewed, accepted divergences:
    # the former fixes the reference's invalid multiline delimiter rewrite, and
    # the latter is the documented narrow commented-assignment boundary.
    return 1 if totals["other"] or totals["line-count"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
