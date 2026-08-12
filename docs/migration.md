# Migrating from findent 4.3.7

The Rust binary keeps the stdin/stdout workflow and free-form structural indentation. The default
is now `FormatMode::Full`: it retains findent-compatible indentation and adds lexical normalization
and wrapping. Use `--indent-only` for the findent-compatible indentation contract, or spell
`--full` explicitly when adopting the additions. `-ifree`, `-ofree`, and `-osame` remain accepted as
free-form compatibility no-ops.

| Legacy option family | Rust behavior |
| --- | --- |
| `-i`, `-I`, `-M`, `-l`, `-C`, `-c`, `-e` | supported, including attached and separated short values |
| `-a`, `-b`, `-d`, `-E`, `-F`, `-f`, `-j`, `-m`, `-r`, `-s`, `-t`, `-w`, `-x` | supported |
| `--indent-associate`, `--indent-block`, `--indent-case`, `--indent-contains`, `--indent-do`, `--indent-entry`, `--indent-enum`, `--indent-forall`, `--indent-if`, `--indent-interface`, `--indent-module`, `--indent-procedure`, `--indent-select`, `--indent-type`, `--indent-where`, `--indent-changeteam` | supported; `_` and `-` spellings are equivalent |
| `--start-indent`, `--max-indent`, `--label-left`, `--include-left`, `--openmp` | supported |
| `-k`, `-K`, `--indent-continuation`, `--indent-ampersand`, `--align_paren` | supported for free form |
| `-Rr`, `-RR`, `--ws_remred` | supported as explicit transformations; bare `--ws_remred` enables it |
| `-i-`, `--indent=none` | supported; preserve source indentation while retaining other formatting contracts |
| `-lastindent`, `-lastusable` | supported |
| `-ifixed`, `-ofixed`, `-iauto`, `--continuation` | rejected with status 2 |
| `--relabel`, `--deps`, editor wrappers, `--safe`, `--selfrep` | not implemented |

Rust intentionally does not read `FINDENT_FLAGS` and rejects unknown options. `--indent-only`
matches the legacy formatter's removal of trailing spaces/tabs while preserving other source
spelling and non-trailing body bytes. Full mode additionally normalizes keywords, separators,
comments, array constructors, declaration-driven names and kind suffixes, and can wrap statements.
Use `--ws_remred` or `--ws_remred=1` for broader redundant-whitespace reduction; `=0` disables it.
The formatter does not accept filenames on stdin; pipe source through stdin and capture stdout.

Full mode's intentional divergences are centralized in [docs/compatibility.md](compatibility.md):
array constructors, conservative comment bodies, kind suffixes on continuation lines, the governing
declaration rule versus the reference ambiguity veto, `!$` sentinel spacing, and preservation of a
valid literal under `--ws_remred`.

## CAMB pre-commit hook

The Python wheel is a thin launcher around the native `findent` executable. Its PEP 517 backend is
setuptools because it is widely available, produces ordinary platform wheels, and lets the build
copy a prebuilt release executable without adding a Python runtime dependency. The wheel version is
read from this repository's `Cargo.toml`; the two version numbers therefore cannot drift. A wheel
install does not require Rust. Building a wheel from a source checkout does require a matching
prebuilt binary (`cargo build --locked --release`, or `FINDENT_BINARY=/path/to/findent`), which is
what the release CI matrix supplies for Linux x86_64, macOS x86_64/arm64, and Windows x86_64.

The hook must use a Python environment managed by pre-commit. `language: system` is not safe here:
it resolves `entry:` through `PATH`, where findent 4.3.7 accepts `--full` and exits successfully
without formatting anything. The wheel's `findent` console script forwards all arguments and
filenames to the native binary, so `language: python` plus the pinned dependency below resolves
the entry inside pre-commit's environment instead.

The hook remains a CAMB-local hook, so its repository reference is `repo: local` and it has no
`rev`. The version pin is `additional_dependencies`, not a guessed revision of this formatter's
source repository. This source tree has no release tag or pre-commit hook manifest; using
`https://github.com/wvermin/findent` with `rev: 5b333b8` would make pre-commit install the source
project itself, whose wheel build requires a prebuilt native binary. A separate published hook
repository and release tag could be adopted later, but that is an owner/release decision, not a
revision that can be inferred safely here.

Both CAMB hooks are replaced by the one below: `--full` does the standardize half itself, and
leaving `standardize-fortran` in place would keep the Python dependency this port exists to remove.
Deleting it loses nothing measurable — running the frozen `standardize_fortran` over the output of
this formatter changes **0 files and 0 lines across all 58 CAMB sources**, so the second hook is a
no-op on anything the first has already written.

An owner who prefers to migrate in two steps can swap `findent-fortran` first and keep
`standardize-fortran` for a release; the two compose safely for exactly that reason. The end state
is one hook. The twelve arguments are CAMB's editable house style and are deliberately hook
arguments, not formatter defaults:

### G10 Python reference fixes

The frozen oracle remains `tools/reference/standardize_fortran.py`. The corrected copy is
`tools/reference/standardize_fortran_patched.py`; it is a validation reference, not a replacement
for the frozen file and does not change existing sweep targets. If the owner wants CAMB's committed
tree to use the corrected Python behavior during the handoff, copy that patched file to
`CAMB/scripts/standardize_fortran.py` (preserving CAMB's filename). The owner then reruns the
standardization hook in a CAMB checkout; this repository's `CAMB/` copy is intentionally untouched.

On the supplied CAMB tree, the Rust output versus the patched reference was checked in a throwaway
checkout: the former 4-file/14-line declaration correction set becomes **0 files / 0 lines**.
That is a handoff verification, not a change to `CAMB/` here. The corrected Python also fixes the
three settled reference cases (`values`, `pk`, and `bjl_recurrence_max_l`) without changing the
frozen baseline.

```yaml
      # Before (both hooks):
      - id: findent-fortran
        name: findent Fortran
        entry: python scripts/findent_fortran.py
        language: system
        pass_filenames: true
        files: \.(?:f90|F90)$
        exclude: ^forutils/
      - id: standardize-fortran
        name: standardize Fortran
        entry: python scripts/standardize_fortran.py
        language: system
        pass_filenames: true
        files: \.(?:f90|F90)$
        exclude: ^forutils/

      # After (one hook replaces both):
      - id: findent-fortran
        name: findent Fortran
        entry: findent
        language: python
        additional_dependencies:
          - findent==0.1.0
        pass_filenames: true
        files: \.(?:f90|F90)$
        exclude: ^forutils/
        args:
          - --full
          - --indent=4
          - --indent_module=0
          - --indent_procedure=0
          - --start_indent=4
          - --indent_contains=0
          - --openmp=0
          - --indent_contains=restart
          - --indent_select=4
          - --indent_case=4
          - --indent_interface=0
          - --indent_continuation=4
          - --indent_ampersand
```

Explicit paths retain whole-repository declaration discovery; add `--isolated` only when that
project context is intentionally unwanted. The hook's explicit filenames use the same project-wide
analysis while limiting writes to pre-commit's selected files.

The package identity needs an owner decision before publication. `NOTICE` describes a clean-room
reimplementation of findent 4.3.7 and credits Willem Vermin, while the current metadata calls the
package `findent`, links to the upstream repository, and credits “findent contributors”. Options:

| Choice | Install line and hook entry | Implication |
| --- | --- | --- |
| Keep `findent` | `python -m pip install 'findent==0.1.0'`; `entry: findent` | Retains the current command but risks upstream attribution confusion and a PATH collision. |
| Rename the distribution and command, for example to `findent-rs` | `python -m pip install 'findent-rs==0.1.0'`; `entry: findent-rs` | Makes the reimplementation and its command distinct; metadata, wheel name, and hook dependency all change. |
| Keep a distinct distribution name but retain `findent` as the console script | `python -m pip install '<chosen-name>==0.1.0'`; `entry: findent` | Avoids a distribution-name collision but keeps the global command collision; the script is only unambiguous inside pre-commit's environment. |

The metadata is deliberately unchanged pending that choice.

The current CI job produces release artifacts, including a Linux wheel tagged
`py3-none-linux_x86_64`. That is not a PyPI-compatible manylinux tag, so the migration must not
claim a PyPI install path today. Until CI produces manylinux wheels and the owner selects and
publishes the package name, install a release artifact by path, for example:

```sh
python -m pip install /path/to/findent-0.1.0-py3-none-linux_x86_64.whl
```

The `findent==0.1.0` pre-commit dependency is the post-publication configuration. Local hook
verification can resolve that exact dependency from a release-artifact directory with
`PIP_FIND_LINKS=/path/to/artifacts`; contributors should not need a separate manual install once
the chosen, compatible distribution is published.
