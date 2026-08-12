#!/usr/bin/env python3
"""Decide case differences from the source, not from the reference.

The frozen Python has bugs of its own, so a difference between us and it is only
a defect when the correct spelling is settled by the code being formatted.  This
walks every identifier we spell differently from the reference, finds the
declaration sites for that name in the project, and reports what those sites
say — so a residue can be adjudicated as evidence rather than argued from
whichever tool one happens to trust.

It reports, it does not judge: a name whose declaration sites disagree is listed
with all of them, because that is exactly the case where scope decides and a
grep cannot.

    python3 tools/adjudicate_case.py --pre /tmp/camb-pre

Verdicts:

    ours            every declaration site agrees, and agrees with us
    reference       every declaration site agrees, and agrees with the reference
    neither         every declaration site agrees, and neither of us matches it
    scope-decides   the sites disagree; the governing declaration is the answer
                    and only a scope-aware resolver can pick it
    undeclared      no declaration site found (associate names, intrinsics,
                    names declared in a file outside the tree)
"""

from __future__ import annotations

import argparse
import collections
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "reference"))

import check_historic_corpus as H  # noqa: E402
import differential as D  # noqa: E402

WORD = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")

# Declaration shapes, in the order a reader would rank them.  Each returns the
# declared spelling of `name` if this line declares it.
DECLARATION_PATTERNS = (
    ("type-bound", r"\b(?:procedure|generic|final)\b[^:]*::\s*(?P<rest>.*)$"),
    ("entity", r"::\s*(?P<rest>.*)$"),
    ("function", r"\bfunction\s+(?P<one>{name})\b"),
    ("subroutine", r"\bsubroutine\s+(?P<one>{name})\b"),
    ("type", r"\btype\s*(?:,[^:]*)?::\s*(?P<one>{name})\b"),
    ("module", r"\bmodule\s+(?P<one>{name})\b"),
    (
        "old-entity",
        r"^\s*(?:(?:integer|real|complex|logical|character)(?:\s*\([^)]*\))?|(?:type|class)\s*\([^)]*\)|double\s+precision)(?:\s*,[^:]*)?\s*(?P<rest>.*)$",
    ),
)


def declared_entity_spelling(rest: str, lowered: str) -> str | None:
    """Return ``lowered`` only when it is an entity-list head.

    After ``::`` (or an old-style type specifier), each top-level comma starts
    a new entity.  Array bounds, initializers, and procedure targets are not
    declarations, even when they contain the same identifier.
    """
    depth = 0
    start = 0
    for index in range(len(rest) + 1):
        at_end = index == len(rest)
        character = "" if at_end else rest[index]
        if at_end or (character == "," and depth == 0):
            entity = rest[start:index]
            depth_entity = 0
            equals = len(entity)
            for offset, item in enumerate(entity):
                if item in "([":
                    depth_entity += 1
                elif item in ")]":
                    depth_entity = max(0, depth_entity - 1)
                elif item == "=" and depth_entity == 0:
                    equals = offset
                    break
            match = WORD.search(entity[:equals])
            if match and match.group(0).lower() == lowered:
                return match.group(0)
            start = index + 1
        elif character in "([":
            depth += 1
        elif character in ")]":
            depth = max(0, depth - 1)
    return None


def declaration_sites(sources: dict[Path, str], name: str) -> list[tuple[str, Path, int, str]]:
    """Every line in the project that declares `name`, with the spelling it uses."""
    lowered = name.lower()
    word = re.compile(rf"(?<![A-Za-z0-9_]){re.escape(lowered)}(?![A-Za-z0-9_])", re.IGNORECASE)
    found: list[tuple[str, Path, int, str]] = []
    for path, text in sources.items():
        for number, line in enumerate(text.splitlines(), 1):
            code = line.split("!", 1)[0]
            if not word.search(code):
                continue
            for kind, pattern in DECLARATION_PATTERNS:
                match = re.search(pattern.format(name=lowered), code, re.IGNORECASE)
                if not match:
                    continue
                if match.groupdict().get("one"):
                    found.append((kind, path, number, match.group("one")))
                    break
                spelling = declared_entity_spelling(match.group("rest"), lowered)
                if spelling is None:
                    continue
                found.append((kind, path, number, spelling))
                break
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pre", type=Path, required=True)
    # Absolute: `rust_project` runs the binary with a throwaway checkout as its
    # working directory, so a relative path would resolve against that.
    parser.add_argument("--binary", default=str(HERE.parent / "target/release/findent"))
    parser.add_argument("--findent", default="findent")
    parser.add_argument(
        "--patched", action="store_true",
        help="use the governing-declaration reference; the default remains frozen",
    )
    parser.add_argument("--converge", action="store_true", default=True)
    parser.add_argument("--sites", type=int, default=4, help="declaration sites to print")
    args = parser.parse_args()

    files = H.corpus_files(args.pre)
    if not files:
        print(f"no corpus under {args.pre}", file=sys.stderr)
        return 2

    reference_path = D.HERE / "standardize_fortran_patched.py" if args.patched else None
    module = D.load_reference(reference_path)
    if args.patched:
        print("reference       tools/reference/standardize_fortran_patched.py")
    D.VOCABULARY = frozenset(module.FORTRAN_STANDARD_WORDS) | frozenset(module.INTRINSIC_NAMES)
    sources = {
        path.relative_to(args.pre): path.read_text(errors="surrogateescape") for path in files
    }
    expected = H.reference_outputs(module, args.findent, sources, args.converge, D.FINDENT_ARGS)
    produced = H.rust_project(args.binary, sources)

    # name -> {(reference spelling, our spelling): count}
    disputes: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    dispute_files: dict[str, dict[tuple[str, str], set[Path]]] = collections.defaultdict(
        lambda: collections.defaultdict(set)
    )
    for relative in sorted(sources, key=str):
        for a, b in H.pairs(expected[relative], produced[relative]):
            if a is None or b is None or H.categorize(a, b) != "case":
                continue
            left, right = WORD.findall(a), WORD.findall(b)
            if len(left) != len(right):
                continue
            for x, y in zip(left, right):
                if x != y:
                    disputes[x.lower()][(x, y)] += 1
                    dispute_files[x.lower()][(x, y)].add(relative)

    verdicts: collections.Counter[str] = collections.Counter()
    rows = []
    for lowered, spellings in disputes.items():
        sites = declaration_sites(sources, lowered)
        declared = {spelling for _, _, _, spelling in sites}
        total = sum(spellings.values())
        if not sites:
            verdict = "undeclared"
        elif len(declared) > 1:
            # A type-bound binding and the procedure implementation it names
            # are not competing declarations.  The implementation's
            # procedure/local declaration governs its own definition; this is
            # the Vofphi/VofPhi shape in DarkEnergyQuintessence.f90.
            implementation_spellings = {
                spelling
                for kind, _, _, spelling in sites
                if kind in {"function", "subroutine"}
            }
            ours_spellings = {ours for _, ours in spellings}
            path_spellings = {
                path: {spelling for _, site_path, _, spelling in sites if site_path == path}
                for paths in dispute_files[lowered].values()
                for path in paths
            }
            # A project may contain different entities with the same normalized
            # name.  The resolver is correct when each disputed use follows the
            # sole declaration in its own source file; aggregate disagreement is
            # then a report-level scope distinction, not an unresolved engine.
            scoped_ours = all(
                ours in path_spellings.get(path, set())
                for pair, paths in dispute_files[lowered].items()
                for _, ours in [pair]
                for path in paths
            )
            # Type-bound bindings are evidence only when they are *unanimous*:
            # `generic :: Item => ...` four times over settles `T%item`, but two
            # types binding the same name with different case settles nothing,
            # and `<=` against a disagreeing set would excuse us for free.
            binding_spellings = {
                spelling for kind, _, _, spelling in sites if kind == "type-bound"
            }
            unanimous_binding = binding_spellings if len(binding_spellings) == 1 else set()
            # `ours_spellings <= declared` is deliberately *not* a clause here.
            # It excuses a use whenever our spelling appears as some declaration
            # anywhere in the project, in any file, which is the opposite of
            # scope resolution: `Pk(:)` in halofit.f90 would excuse `Pk` on a
            # line in InitialPower.f90 whose own declaration reads `PK(n)`.
            # `scoped_ours` is the same idea done correctly — our spelling must
            # be declared in the file the disputed line is in.
            if (
                implementation_spellings == ours_spellings
                or scoped_ours
                or ours_spellings == unanimous_binding
            ):
                verdict = "ours"
            else:
                verdict = "scope-decides"
        else:
            settled = next(iter(declared))
            references = {reference for reference, _ in spellings}
            ours = {ours for _, ours in spellings}
            if ours == {settled}:
                verdict = "ours"
            elif references == {settled}:
                verdict = "reference"
            else:
                verdict = "neither"
        verdicts[verdict] += total
        rows.append((verdict, total, lowered, spellings, sites))

    order = {"reference": 0, "neither": 1, "scope-decides": 2, "undeclared": 3, "ours": 4}
    rows.sort(key=lambda row: (order[row[0]], -row[1]))
    for verdict, total, lowered, spellings, sites in rows:
        pairs = ", ".join(f"{a} -> {b} ({n})" for (a, b), n in spellings.most_common())
        print(f"{verdict:14s} {total:4d}  {lowered}   [{pairs}]")
        for kind, path, number, spelling in sites[: args.sites]:
            print(f"                     {spelling:28s} {kind:11s} {path}:{number}")
        if len(sites) > args.sites:
            print(f"                     ... {len(sites) - args.sites} more sites")

    print()
    print(f"{'verdict':14s} {'pairs':>5s}")
    for verdict in ("reference", "neither", "scope-decides", "undeclared", "ours"):
        print(f"{verdict:14s} {verdicts[verdict]:5d}")
    # Only a declaration that settles the name and contradicts us is a defect.
    return 1 if verdicts["reference"] or verdicts["neither"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
