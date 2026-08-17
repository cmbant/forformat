# forformat

[![CI](https://github.com/cmbant/forformat/actions/workflows/ci.yml/badge.svg)](https://github.com/cmbant/forformat/actions/workflows/ci.yml)

`forformat` is a standalone formatter for free-form Fortran. Its native implementation is written
in Rust and provides findent-compatible indentation together with an optional full-format mode for
lexical normalization and wrapping long statements. Source suffix matching is case-insensitive,
so legacy `.F` and `.F90` spellings are accepted for free-form code that is preprocessed at build
time. Genuine fixed-form sources are detected and skipped unchanged; use `-ifree` when a source
needs to be forced through the free-form formatter.

The repository builds both the `forformat` Rust binary and a Python package that bundles that binary.
Installing a published Python wheel does not require Rust or a Fortran compiler. The wheel provides
both the command and an importable in-memory formatting function.

## Install

### From PyPI

When a release is published, install the command with:

```sh
python -m pip install forformat
```

The package requires Python 3.9 or newer. Check the installation with:

```sh
forformat --version
```

Wheels contain a platform-specific native executable compiled from Rust.

### Pre-commit

Add the repository hook:

```yaml
repos:
  - repo: https://github.com/cmbant/forformat-pre-commit
    rev: v0.1.3
    hooks:
      - id: forformat
      - id: forformat-check
```

The hook package installs the published `forformat` wheel from PyPI, so pre-commit does not build
the formatter from source or require Rust.

### From source

To build and run the native formatter, install Rust 1.85 or newer and use Cargo:

```sh
cargo build --locked --release
./target/release/forformat --version
```

To build the Python wheel as well, use Python 3.9 or newer and the Python build frontend:

```sh
python -m pip install --upgrade build
python -m build --wheel --outdir dist
python -m pip install dist/forformat-*.whl
```

The wheel build uses an existing release binary when one is available and otherwise runs
`cargo build --locked --release` from the checkout. Building from source therefore needs Rust;
an installed wheel does not.

## Use

Format one or more free-form Fortran files in place:

```sh
forformat src/module.f90
forformat src/*.f90
```

Use standard input and output when composing the formatter with other tools:

```sh
forformat --stdin < src/module.f90 > /tmp/module.f90
forformat --stdout src/module.f90 > /tmp/module.f90
```

For editor integrations that provide source through stdin but need project-wide
declarations, name the source file explicitly:

```sh
forformat --stdin --project-context src/module.f90 < src/module.f90
```

Pass the active file path, rather than only the workspace directory, when the
editor supports a file-name placeholder. The buffer remains the only formatted
output, its stdin bytes replace the stale on-disk copy in project analysis,
and the other tracked Fortran sources supply the remaining declarations. A
directory is still accepted for anonymous buffers. Select configuration
independently with `--config` or disable
discovery with `--no-config`.

Python callers can format text or bytes with the same native executable:

```python
from forformat import format_source

formatted = format_source(
    source,
    options=("--config=/absolute/path/to/.forformat.toml",),
    repo_context_path="/path/to/checkout/src/module.f90",
)
```

The return type matches the input type. The Python API disables automatic
configuration discovery unless `options` explicitly supplies `--config`.

Use `--check` in CI or with the check-only pre-commit hook, and `--diff` to review changes without
modifying files:

```sh
forformat --check src/*.f90
forformat --diff src/*.f90
```

For pre-commit, use the rewriting hook to format files in place, or use `forformat-check` when the
hook should only check formatting. Replace `vX.Y.Z` with the release you want to pin:

The separate hook repository installs the published `forformat` wheel from PyPI, so pre-commit
does not build the formatter from source or require Rust.

```yaml
repos:
  - repo: https://github.com/cmbant/forformat-pre-commit
    rev: vX.Y.Z
    hooks:
      - id: forformat          # rewrite files in place
      # - id: forformat-check  # use this instead for check-only CI behavior
```

The default is `--full`. The main modes are:

- `--full`: findent-compatible layout plus lexical normalization and statement wrapping.
- `--indent-only`: indentation and trailing-horizontal-whitespace handling compatible with findent.
- `--normalize-only`: lexical normalization without structural layout.

Run `forformat --help` for indentation controls, wrapping options, preprocessor definitions, and
the complete compatibility option set. Fixed-form Fortran and automatic format detection are not
supported.

### Formatting options

In full mode, wrapping is enabled by default. Use `--wrap` or `--wrap=1` to enable it, `--no-wrap`
or `--wrap=0` to disable it, and `--line-length=<n>` to set the wrapping budget. These options
can also be placed in `.forformat.toml` or `[tool.forformat]` in `pyproject.toml`; underscores and
hyphens are equivalent in configuration keys. For example:

```toml
[tool.forformat]
mode = "full"
indent = 4
wrap = true
line_length = 120
indent_module = 0
indent_procedure = 0
start_indent = 4
indent-contains = "restart"
indent_select = 4
indent_case = 4
indent_interface = 0
indent_continuation = 4
indent_ampersand = true
openmp = 0
```

`--align-declarations` (default on) and `--align-comments` (default off) each own a kind of gap
wherever it occurs: every declaration's `::` and every trailing comment's leading whitespace,
respectively, no matter how wide the authored padding is or how many lines the block spans.
`--ws-remred`'s broader whitespace reduction defers to whichever of those is enabled, leaving that
gap for the alignment pass to decide instead of collapsing it first — so a hand-aligned block of any
width stays aligned even when `--ws-remred` is on. Turning the corresponding alignment option off
hands that gap back to `--ws-remred` everywhere it occurs.

Full-mode style controls are also available from the command line, Rust API, and project TOML:

| Option / TOML key | Effect when enabled | Values | Default |
| --- | --- | --- | --- |
| `keyword-case` | case of recognized keywords and intrinsics | `lower`, `upper`, `preserve` | `lower` |
| `relational-symbols` | `.eq.` and friends become `==` | `0`, `1` | `1` |
| `array-brackets` | `(/ ... /)` becomes `[ ... ]` | `0`, `1` | `1` |
| `compact-multiplicative` | no spaces around binary `*`, `/`, `**` | `0`, `1` | `1` |
| `join-goto` | `go to` becomes `goto` | `0`, `1` | `1` |
| `split-compound-keywords` | `endif` becomes `end if` | `0`, `1` | `1` |
| `strip-empty-args` | drop `()` from `subroutine` definitions | `0`, `1` | `1` |
| `remove-redundant-parens` | drop safely redundant nested parentheses | `0`, `1` | `1` |
| `remove-terminal-return` | drop a bare `return` before `end` | `0`, `1` | `1` |
| `program-unit-spacing` | blank-line separators around program units | `0`, `1` | `1` |
| `max-blank-lines` | cap on consecutive blank lines | non-negative integer, `preserve` | `2` |
| `delimiter-spacing` | comma and bracket spacing | `0`, `1` | `1` |
| `comment-spacing` | gap before a trailing `!` | `0`, `1` | `1` |
| `continuation-markers` | continuation `&` and OpenMP sentinels | `0`, `1` | `1` |

`--max-blank-lines=0` removes program-unit separators even when
`--program-unit-spacing=1`, because the blank-line cap runs after separator insertion.

For example:

```toml
# .forformat.toml
keyword_case = "upper"
compact_multiplicative = false
array_brackets = true
strip_empty_args = false
remove_redundant_parens = true
remove_terminal_return = true
program_unit_spacing = true
max_blank_lines = 1
delimiter_spacing = true
comment_spacing = true
continuation_markers = true
```

These controls affect full and normalize-only normalization; `--indent-only` remains the
findent-compatible layout contract and ignores style settings.

### Project context

When explicit paths are supplied, the formatter scans the current Git checkout for free-form
Fortran sources and uses declarations from those files to resolve names during full formatting. Only
the paths supplied on the command line are changed. This makes project-wide formatting useful when
a declaration in one module controls the spelling or formatting of code in another file.

`--project-context=<path>` identifies the Git project, or the tracked source that stdin replaces.
It does not limit which files supply project context. Use repeatable `--context-path=<directory>`
options to limit semantic analysis to tracked sources beneath selected repository directories:

```sh
forformat --full --stdout src/main.f90 --context-path=src/ --context-path=modules/
```

Relative context paths are resolved from the Git repository root. Absolute paths are accepted only
when they resolve inside that checkout, and every selected path must be an existing directory. With
no context paths, the whole eligible checkout is used as before. `--isolated` disables project
context and cannot be combined with `--context-path`.

For independent file processing, disable repository scanning with `--isolated`:

```sh
forformat --isolated src/module.f90
forformat --isolated --stdout src/module.f90 > /tmp/module.f90
```

To format every tracked free-form Fortran source owned by the current checkout, use:

```sh
forformat --all-files
```

Initialized submodules are scanned only for project context. The optional directory limits the
target set:

```sh
forformat --all-files
forformat --all-files ./src
```

Use `--all` when recursive submodule sources should also be formatting targets:

```sh
forformat --all
```

Add `--no-submodules` when submodule sources should not be included even as project context. With
`--all`, this also prevents submodule sources from being targets; with explicit paths, the
superproject's other tracked sources remain available for context.

Use `--show-files` with explicit paths, `--all`, or `--all-files` to print the selected targets
without reading or modifying them:

```sh
forformat --all-files --show-files
forformat --all-files ./src --show-files --exclude='**/generated-*.f90'
```

Use repeatable `--exclude=<glob>` options to omit vendored or generated sources from automatic
`--all-files` and `--all` targets and from the tracked-source project-context scan. Exclusions are
applied after `context_paths` selects the project-context scope. Explicit paths are always formatted
even when they match an exclusion or fall outside `context_paths`:

```sh
forformat --all --exclude=vendor/ --exclude='**/generated-*.f90'
```

`--exclude` selects the exclusion set rather than adding to it, so it replaces both the default
set and any `exclude` in the configuration file. Use the repeatable `--extend-exclude=<glob>` to
add patterns on top of whatever is already configured.

Repository-wide formatting settings are discovered from the nearest project root. For any
repository, put formatter options in a top-level `.forformat.toml`; Python projects can use the
same option names in `[tool.forformat]` in `pyproject.toml`. The formatting options example above
shows a longer indentation and wrapping configuration; other settings include `align-paren` and
`defines = ["USE_MPI", "REAL_KIND"]`. Project-analysis settings include `context_paths`, an array
of repository directories, and `no_submodules = true`. The exclusion keys are arrays of patterns:
`exclude = ["vendor/"]` replaces the default exclusion set, while
`extend-exclude = ["**/generated-*.f90"]` adds patterns. The default exclusion set is empty:
`forformat` selects files with `git ls-files`, so anything it sees was tracked deliberately and
nothing is skipped unless you say so.

Patterns use `/` separators and support `*` (within one path component), `**` (across components),
and `?` (one non-separator character). A trailing `/` matches that directory and everything below
it. A leading `/` anchors the pattern at the repository root; without it, a pattern may match at
any path-component boundary. Paths are normalized to `/` before matching.

The standalone `.forformat.toml` uses the same top-level keys. Configuration applies to `--all`,
explicit file paths, and standard-input runs from that project. Command-line options take
precedence over the project file; use `--config PATH` to select a file explicitly or `--no-config`
to ignore discovered settings. Workflow controls such as `--all`, `--all-files`, `--check`, and
`--diff` remain command-line-only.

## Rust crate

The Rust crate exposes the core formatter because the binary and repository
tests are separate crate targets:

```rust
let result = forformat::format_source(source, &config)?;
```

This Rust surface, including its entry points and public types, is an
implementation interface and is not covered by semantic-versioning guarantees.
Use the command or Python API for a supported integration boundary. The core is
byte-oriented and preserves non-UTF-8 bytes outside the formatting contract.

## Development

The main implementation is under `src/`; the Python wheel launcher is under `forformat_runner/`.
Tests and golden fixtures are in `tests/`.

Run the normal local checks with:

```sh
cargo test --locked --all-targets
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

For changes to full-mode normalization, wrapping, or layout, also run
`./tools/check_fuzz_regression.sh` and the relevant focused Rust properties.

To check that a built wheel actually works before it is published — the same check both CI
workflows run on every platform:

```sh
python -m build --wheel --outdir dist
bash tools/check_wheel.sh dist
```

Documentation:

- [`AGENTS.md`](AGENTS.md) — the short orientation, and the checks a change has to pass.
- [`docs/history/full-mode-port-design.md`](docs/history/full-mode-port-design.md) — historical
  port design and rationale.
- [`docs/full-mode.md`](docs/full-mode.md) — the full-mode rules, standing checks, and known traps.
- [`docs/compatibility.md`](docs/compatibility.md) — the findent-compatibility boundary and the
  reviewed divergences.
- [`docs/migration.md`](docs/migration.md) and [`docs/file-workflow.md`](docs/file-workflow.md) —
  migration notes and the file/project workflow.
- [`docs/history/`](docs/history/) — the original port plan and its closed work ledger, kept for
  provenance.

The formatter's pipeline and idempotence invariants are described at the top of
[`src/format/full.rs`](src/format/full.rs).

## Relationship to findent

`forformat` is a clean-room Rust reimplementation informed by findent 4.3.7. `--indent-only` is
the findent-compatible indentation contract; full mode intentionally adds behavior beyond that
contract. See the compatibility document for the reviewed differences and migration guidance.

The project is licensed under the BSD-3-Clause license. Attribution for the findent reference and
the applicable third-party terms are in [`NOTICE`](NOTICE) and
[`LICENSE-THIRD-PARTY`](LICENSE-THIRD-PARTY); the project license is [`LICENSE`](LICENSE).
