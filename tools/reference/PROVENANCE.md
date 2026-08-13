# Reference tools

The current `standardize_fortran.py` is the single Python reference, derived from CAMB's committed
script and carrying the formatter fixes documented in this repository. Nothing in `src/` calls
it. The original snapshot is retained for the historic-corpus differential's pre-fix comparison
mode.

| File | SHA-256 |
|---|---|
| `standardize_fortran.py` | `14223363f92c189005cc793329725d25fa09e673909084ba243cf3ebf3e8cb26` |
| `standardize_fortran_original.py` | `8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e` |
| `test_standardize_fortran.py` | `3aacef41c135a5ecc1956d520723a97deae693cd57986db774d2846443a9662e` |
| `findent_fortran.py` | `62785d11868e736b255e149f915b7de70d48f1075fb061846d2020fc36cede3b` |

CAMB's own `scripts/standardize_fortran.py` and `scripts/test_standardize_fortran.py` contain four
fixes: extension-before-existence validation, typed local entities, top-level/program-unit
parameter scope, and owner-keyed type-bound procedure casing. The `tools/reference/` copies above
were the source for the reference copies. The current reference additionally models implicit-
typing permission scopes so unresolved implicit identifiers retain their authored case.
`standardize_fortran_original.py` is the byte snapshot used by `check_historic_corpus.py`'s
pre-fix comparison mode.


`R` = findent **4.3.7**, `/usr/local/bin/findent`, sources at `/opt/findent`.

## CAMB findent profile

The exact argument set both `findent_fortran.py` and `.vscode/settings.json` use. `--indent_contains`
appears twice on purpose; the last wins.

```text
--indent=4 --indent_module=0 --indent_procedure=0 --start_indent=4
--indent_contains=0 --openmp=0 --indent_contains=restart --indent_select=4
--indent_case=4 --indent_interface=0 --indent_continuation=4 --indent_ampersand
```

## Verifying reference hashes

```sh
sha256sum -c <<'EOF'
14223363f92c189005cc793329725d25fa09e673909084ba243cf3ebf3e8cb26  tools/reference/standardize_fortran.py
8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e  tools/reference/standardize_fortran_original.py
3aacef41c135a5ecc1956d520723a97deae693cd57986db774d2846443a9662e  tools/reference/test_standardize_fortran.py
62785d11868e736b255e149f915b7de70d48f1075fb061846d2020fc36cede3b  tools/reference/findent_fortran.py
EOF
```

The current reference is the default for adjudication and every differential, project, route,
perturbation, historic, and corpus check. Use `standardize_fortran_original.py` directly with
`differential.load_reference` for the historic-corpus pre-fix comparison mode.

## Baseline of the Python suite

Run from the CAMB root:

```sh
python3 -m unittest scripts.test_standardize_fortran
```

The reference suite reports **93 tests, 0 failures, 0 errors**.
