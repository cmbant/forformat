# Frozen reference tools

These files are the **oracle**, not a dependency. Nothing in `src/` may call them, and they must
not be edited during the port: an expected output that changes because the oracle changed is not
evidence of anything.

| File | SHA-256 |
|---|---|
| `standardize_fortran.py` | `8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e` |
| `standardize_fortran_patched.py` | `0180e6fe7842c43a73ac63e59ffbea32bd30a04c04ca64221c99bf07846ce11d` |
| `test_standardize_fortran.py` | `03fb7f6a2ca25a1092bc70c53cdc0533c3dd74a2d24b0a1a0fb2e2913908d892` |
| `test_standardize_fortran_patched.py` | `fe7ce4f40c66bbec4521e91eced37e0cecb0c4a556ae568d692c2d6a212cb5b5` |
| `findent_fortran.py` | `62785d11868e736b255e149f915b7de70d48f1075fb061846d2020fc36cede3b` |

Source: `CAMB` working tree at commit `a1db7a71505520e217bacdc788152c26e07fedeb`
(2026-08-10 22:02:17 +0000), copied 2026-08-11.

`standardize_fortran_patched.py` is not an oracle and does not retarget any
standing check. It loads the frozen file above and adds the G10 governing-
declaration fixes: old-style entity extraction, top-level program parameters,
typed local entities, owner-keyed type-bound bindings, and extension-first
path validation. It was created in
this repository on 2026-08-12 from the frozen file; its hash is recorded above.

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
8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e  tools/reference/standardize_fortran.py
0180e6fe7842c43a73ac63e59ffbea32bd30a04c04ca64221c99bf07846ce11d  tools/reference/standardize_fortran_patched.py
03fb7f6a2ca25a1092bc70c53cdc0533c3dd74a2d24b0a1a0fb2e2913908d892  tools/reference/test_standardize_fortran.py
fe7ce4f40c66bbec4521e91eced37e0cecb0c4a556ae568d692c2d6a212cb5b5  tools/reference/test_standardize_fortran_patched.py
62785d11868e736b255e149f915b7de70d48f1075fb061846d2020fc36cede3b  tools/reference/findent_fortran.py
EOF
```

The patched-reference check is explicit and separate:

```sh
python3 tools/adjudicate_case.py --pre /tmp/camb-pre --patched
```

The frozen run remains the default invocation and remains the source for every
existing differential, project, route, perturbation, historic, and corpus
check.

## Baseline of the Python suite

Run from the CAMB root:

```sh
python3 -m unittest scripts.test_standardize_fortran
```

Verified 2026-08-11: **85 pass, 1 fails with 8 subtest errors** —
`test_standard_free_form_extensions_are_accepted`, the known
extension-vs-existence defect recorded as §9.1 of the port plan. Rust fixes it rather than
reproducing it.
