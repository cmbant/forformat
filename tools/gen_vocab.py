#!/usr/bin/env python3
"""Generate `src/transform/vocab.rs` from the frozen Python reference.

The vocabularies are pure data (~363 lines of the reference formatter).  Copying
them by hand is a transcription-error factory, so they are generated, and the
generated file is committed so the build stays dependency-free.

    python3 tools/gen_vocab.py            # rewrite src/transform/vocab.rs
    python3 tools/gen_vocab.py --check    # fail if the file is stale
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "tools" / "reference" / "standardize_fortran.py"
TARGET = ROOT / "src" / "transform" / "vocab.rs"


def load_reference():
    spec = importlib.util.spec_from_file_location("frozen_standardize_fortran", REFERENCE)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def rust_str(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def emit_set(name: str, doc: str, words) -> str:
    items = "".join(f"    {rust_str(word)},\n" for word in sorted(words))
    return (
        f"/// {doc}\n"
        f"///\n"
        f"/// Sorted, lowercase, and looked up with [`contains`].\n"
        f"pub static {name}: &[&str] = &[\n{items}];\n\n"
    )


def emit_pairs(name: str, doc: str, pairs) -> str:
    items = "".join(f"    ({rust_str(a)}, {rust_str(b)}),\n" for a, b in sorted(pairs))
    return (
        f"/// {doc}\n"
        f"///\n"
        f"/// Sorted by the first element.\n"
        f"pub static {name}: &[(&str, &str)] = &[\n{items}];\n\n"
    )


def generate(module) -> str:
    digest = hashlib.sha256(REFERENCE.read_bytes()).hexdigest()
    header = f"""//! Fortran vocabularies, generated from the frozen Python reference.
//!
//! DO NOT EDIT BY HAND.  Regenerate with `python3 tools/gen_vocab.py`, which
//! reads `tools/reference/standardize_fortran.py`
//! (sha256 `{digest}`).
//!
//! Everything here is lowercase and sorted so lookups are a branch-predictable
//! binary search with no allocation and no hash map in the hot path.

/// Case-insensitive membership test over one of the sorted tables below.
pub fn contains(table: &[&str], word: &[u8]) -> bool {{
    lookup(table, word).is_some()
}}

/// The canonical lowercase spelling of `word`, when the table holds it.
pub fn lookup<'a>(table: &'a [&'a str], word: &[u8]) -> Option<&'a str> {{
    let index = table
        .binary_search_by(|entry| compare(entry.as_bytes(), word))
        .ok()?;
    Some(table[index])
}}

/// The second element of a pair table, keyed case-insensitively by the first.
pub fn lookup_pair<'a>(table: &'a [(&'a str, &'a str)], word: &[u8]) -> Option<&'a str> {{
    let index = table
        .binary_search_by(|entry| compare(entry.0.as_bytes(), word))
        .ok()?;
    Some(table[index].1)
}}

/// Compare a lowercase table entry against an arbitrary-case word.
fn compare(entry: &[u8], word: &[u8]) -> core::cmp::Ordering {{
    let mut left = entry.iter();
    let mut right = word.iter();
    loop {{
        match (left.next(), right.next()) {{
            (None, None) => return core::cmp::Ordering::Equal,
            (None, Some(_)) => return core::cmp::Ordering::Less,
            (Some(_), None) => return core::cmp::Ordering::Greater,
            (Some(a), Some(b)) => {{
                let b = b.to_ascii_lowercase();
                if *a != b {{
                    return a.cmp(&b);
                }}
            }}
        }}
    }}
}}

"""
    body = "".join(
        [
            emit_set(
                "FORTRAN_KEYWORDS",
                "Fortran 90-2018 language keywords, including multi-word statement components.",
                module.FORTRAN_KEYWORDS,
            ),
            emit_set(
                "FORTRAN_SPECIFIERS",
                "I/O and statement specifiers such as `unit`, `iostat`, `status`.",
                module.FORTRAN_SPECIFIERS,
            ),
            emit_set(
                "INTRINSIC_PROCEDURES",
                "Intrinsic procedures. Never override a locally declared identifier (I4).",
                module.INTRINSIC_PROCEDURES,
            ),
            emit_set(
                "INTRINSIC_NAMES",
                "Intrinsic procedures plus intrinsic module and type names.",
                module.INTRINSIC_NAMES,
            ),
            emit_set(
                "OPENMP_KEYWORDS",
                "OpenMP directive and clause words.",
                module.OPENMP_KEYWORDS,
            ),
            emit_set(
                "DECLARATION_ATTRIBUTES",
                "Attributes admissible after a type specification, in canonical order.",
                module.DECLARATION_ATTRIBUTES,
            ),
            emit_set(
                "PARENTHESIZED_STATEMENT_NAMES",
                "Statements written `name(...)` whose keyword takes no space before the paren.",
                module.PARENTHESIZED_STATEMENT_NAMES,
            ),
            emit_set(
                "COMPACT_ARITHMETIC_OPERATORS",
                "Arithmetic operators the reference formatter writes without surrounding spaces.",
                module.COMPACT_ARITHMETIC_OPERATORS,
            ),
            emit_pairs(
                "COMPOUND_KEYWORDS",
                "Run-together keyword spellings and their separated canonical form.",
                module.COMPOUND_KEYWORDS.items(),
            ),
            emit_pairs(
                "MULTIWORD_KEYWORD_PAIRS",
                "Keyword pairs whose separating whitespace is normalized to one space.",
                module.MULTIWORD_KEYWORD_PAIRS,
            ),
            emit_pairs(
                "MODERN_OPERATOR",
                "Legacy relational operators (`.eq.`) and their modern spelling.",
                module.MODERN_OPERATOR.items(),
            ),
        ]
    )
    extensions = "".join(
        f"    {rust_str(extension)},\n" for extension in sorted(module.FORTRAN_SOURCE_EXTENSIONS)
    )
    constants = f"""/// The reference formatter's default line-length budget.
pub const MAX_LINE_LENGTH: usize = {module.MAX_LINE_LENGTH};

/// A wrapped line must fill at least this fraction of its budget, otherwise the
/// break point is rejected as leaving too much whitespace.
pub const MINIMUM_BREAK_FILL: f64 = {module.MINIMUM_BREAK_FILL};

/// Free-form source extensions, lowercase.  Uppercase spellings are accepted too.
pub static SOURCE_EXTENSIONS: &[&str] = &[
{extensions}];
"""
    return header + body + constants


def rustfmt(source: str) -> str:
    """Normalize through rustfmt so `--check` cannot trip over `cargo fmt`.

    The generator emits one item per line; rustfmt collapses short tables onto a
    single line.  Without this step the committed file is permanently "stale"
    the moment anyone runs `cargo fmt`, which is exactly the false alarm that
    teaches people to ignore the check.
    """
    try:
        result = subprocess.run(
            ["rustfmt", "--edition", "2021", "--emit", "stdout", "--quiet"],
            input=source, capture_output=True, text=True, check=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"warning: rustfmt unavailable, emitting unformatted ({error})", file=sys.stderr)
        return source
    return result.stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = rustfmt(generate(load_reference()))
    if args.check:
        current = TARGET.read_text() if TARGET.exists() else ""
        if current != generated:
            print(f"{TARGET} is stale; run python3 tools/gen_vocab.py", file=sys.stderr)
            return 1
        return 0
    TARGET.write_text(generated)
    print(f"wrote {TARGET} ({len(generated.splitlines())} lines)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
