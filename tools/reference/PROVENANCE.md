# Reference tools

The current `standardize_fortran.py` is the single live Python reference, synced byte-for-byte
from CAMB's committed script. Nothing in `src/` calls it. The original snapshot is retained only
for historical differential comparisons.

| File | SHA-256 |
|---|---|
| `standardize_fortran.py` | `b7f03c94ae80e108d30f4568660baf1051d0a730a171d0504ca83de957ef03de` |
| `standardize_fortran_original.py` | `8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e` |
| `test_standardize_fortran.py` | `32d7730e7028892e81eee5e8787289209788bc989c1deb0cbec071445f465e77` |
| `findent_fortran.py` | `62785d11868e736b255e149f915b7de70d48f1075fb061846d2020fc36cede3b` |

The owner explicitly authorized the G10 source fix on 2026-08-12 after the freeze had served its
purpose. CAMB's own `scripts/standardize_fortran.py` and
`scripts/test_standardize_fortran.py` were edited directly for four fixes: extension-before-
existence validation, typed local entities, top-level/program-unit parameter scope, and
owner-keyed type-bound procedure casing. The `tools/reference/` copies above were then synced from
those CAMB files. `standardize_fortran_original.py` is the untouched pre-fix byte snapshot kept
under the old hash for the historical differential.


`R` = findent **4.3.7**, `/usr/local/bin/findent`, sources at `/opt/findent`.

## CAMB findent profile

The exact argument set both `findent_fortran.py` and `.vscode/settings.json` use. `--indent_contains`
appears twice on purpose; the last wins.

```text
--indent=4 --indent_module=0 --indent_procedure=0 --start_indent=4
--indent_contains=0 --openmp=0 --indent_contains=restart --indent_select=4
--indent_case=4 --indent_interface=0 --indent_continuation=4 --indent_ampersand
```

## Verifying the freeze

```sh
sha256sum -c <<'EOF'
b7f03c94ae80e108d30f4568660baf1051d0a730a171d0504ca83de957ef03de  tools/reference/standardize_fortran.py
8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e  tools/reference/standardize_fortran_original.py
32d7730e7028892e81eee5e8787289209788bc989c1deb0cbec071445f465e77  tools/reference/test_standardize_fortran.py
62785d11868e736b255e149f915b7de70d48f1075fb061846d2020fc36cede3b  tools/reference/findent_fortran.py
EOF
```

The current reference is the default for adjudication and every differential, project, route,
perturbation, historic, and corpus check. Use `standardize_fortran_original.py` directly with
`differential.load_reference` only when reproducing the historical pre-fix comparison.

## Baseline of the Python suite

Run from the CAMB root:

```sh
python3 -m unittest scripts.test_standardize_fortran
```

Verified 2026-08-12: **89 tests, 0 failures, 0 errors** from CAMB's own suite.
