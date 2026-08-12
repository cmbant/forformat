#!/usr/bin/env python3
"""Run CAMB's frozen test suite against the **patched** reference module.

`standardize_fortran_patched.py` is a delta over the frozen oracle, so the
question that matters is not whether its own four tests pass but whether it
still satisfies everything CAMB already asserts.  This aliases
`scripts.standardize_fortran` to the patched module, loads the frozen suite
from `tools/reference/` (byte-identical to `CAMB/scripts/`, so no CAMB path is
needed), and adds this repository's patched-reference tests.

One frozen test is excluded, by name and with its reason recorded here rather
than by a predicate that could quietly grow:
`SpacingTests.test_type_bound_procedures_only_supply_component_case` asserts the
flat project-wide `type_procedure_cases` map that the governing-declaration fix
deliberately replaces with an owner-keyed one.  Its replacement lives in
`test_standardize_fortran_patched.py` and asserts the new contract, so the
behaviour is still pinned — it is not dropped.

The eight `test_standard_free_form_extensions_are_accepted` subtests fail
against the *frozen* module (CAMB's own suite is red on them today) and pass
here, because the patched module checks the suffix before the file's existence.
That difference is the point of the check: if it ever stops holding, the patch
has regressed.

Usage:
    python3 tools/check_patched_reference.py
"""

from __future__ import annotations

import importlib
import sys
import types
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

# The frozen suite is CAMB's file verbatim, so it imports `scripts.
# standardize_fortran`.  Point that name at the patched module: this is the
# whole mechanism, and it is why the frozen file never needs editing.
EXCLUDED = (
    "tools.reference.test_standardize_fortran"
    ".SpacingTests.test_type_bound_procedures_only_supply_component_case"
)


def main() -> int:
    patched = importlib.import_module("tools.reference.standardize_fortran_patched")
    package = types.ModuleType("scripts")
    package.__path__ = []  # type: ignore[attr-defined]
    sys.modules.setdefault("scripts", package)
    sys.modules["scripts.standardize_fortran"] = patched

    frozen = importlib.import_module("tools.reference.test_standardize_fortran")
    ours = importlib.import_module("tools.reference.test_standardize_fortran_patched")

    loader = unittest.defaultTestLoader
    suite = unittest.TestSuite()
    excluded = 0
    for group in loader.loadTestsFromModule(frozen):
        cases = group if isinstance(group, unittest.TestSuite) else [group]
        for case in cases:
            if case.id() == EXCLUDED:
                excluded += 1
                continue
            suite.addTest(case)
    if excluded != 1:
        print(
            f"expected to exclude exactly one frozen test, excluded {excluded}: "
            "the frozen suite has changed, or the name has drifted",
            file=sys.stderr,
        )
        return 2
    suite.addTests(loader.loadTestsFromModule(ours))

    result = unittest.TextTestRunner(verbosity=1).run(suite)
    print(
        f"\npatched suite: {result.testsRun} tests, "
        f"failures={len(result.failures)}, errors={len(result.errors)}"
    )
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
