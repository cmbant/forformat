# forformat

[![CI](https://github.com/cmbant/forformat/actions/workflows/ci.yml/badge.svg)](https://github.com/cmbant/forformat/actions/workflows/ci.yml)

`forformat` is a standalone formatter for free-form Fortran. Its native implementation is written
in Rust and provides findent-compatible indentation together with an optional full-format mode for
lexical normalization and wrapping long statements.

The repository builds both the `forformat` Rust binary and a Python package that bundles that binary.
Installing a published Python wheel does not require Rust or a Fortran compiler. `forformat` is a
command-line formatter, not an importable Python formatting library.

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

Wheels contain a platform-specific native executable. If `forformat` is not yet available for your
platform on PyPI, build a wheel from a source checkout as described below.

### From source

To build and run the native formatter, install Rust 1.85 or newer and use Cargo:

```sh
cargo build --locked --release
./target/release/forformat --version
```

To build the Python wheel as well, use Python 3.9 or newer and the Python build frontend:

```sh
cargo build --locked --release
python -m pip install --upgrade build
python -m build --wheel --outdir dist
python -m pip install dist/forformat-*.whl
```

The wheel build packages the already-built `target/release/forformat` executable. A source build
therefore needs Rust; an installed wheel does not.

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

Use `--check` in CI or pre-commit, and `--diff` to review changes without modifying files:

```sh
forformat --check src/*.f90
forformat --diff src/*.f90
```

The default is `--full`. The main modes are:

- `--full`: findent-compatible layout plus lexical normalization and statement wrapping.
- `--indent-only`: indentation and trailing-horizontal-whitespace handling compatible with findent.
- `--normalize-only`: lexical normalization without structural layout.

Run `forformat --help` for indentation controls, wrapping options, preprocessor definitions, and
the complete compatibility option set. Fixed-form Fortran and automatic format detection are not
supported.

### Project context

When explicit paths are supplied, the formatter scans the current Git checkout for free-form
Fortran sources and uses declarations from those files to resolve names during full formatting. Only
the paths supplied on the command line are changed. This makes project-wide formatting useful when
a declaration in one module controls the spelling or formatting of code in another file.

For independent file processing, disable repository scanning with `--isolated`:

```sh
forformat --isolated src/module.f90
forformat --isolated --stdout src/module.f90 > /tmp/module.f90
```

To format every tracked free-form Fortran source in a checkout, use:

```sh
forformat --all
```

Repository-wide formatting settings are discovered from the nearest project root. For any
repository, put formatter options in a top-level `.forformat.toml`; Python projects can use the
same option names in `[tool.forformat]` in `pyproject.toml`:

```toml
[tool.forformat]
mode = "full"
indent = 4
line-length = 100
align-paren = true
defines = ["USE_MPI", "REAL_KIND"]
```

The standalone `.forformat.toml` uses the same top-level keys. Configuration applies to `--all`,
explicit file paths, and standard-input runs from that project. Command-line options take
precedence over the project file; use `--config PATH` to select a file explicitly or `--no-config`
to ignore discovered settings. Workflow controls such as `--all`, `--check`, and `--diff` remain
command-line-only.

## Rust library

The Rust crate also exposes the core formatter for applications that need an in-memory API:

```rust
let result = forformat::format_source(source, &config)?;
```

The supported entry points are `forformat::format_source`, `forformat::format_source_with_context`,
`forformat::format_to`, and `forformat::format_to_owned`, together with the configuration and result
types they use (`FormatConfig`, `FormatMode`, `WrapConfig`, `MacroDefine`, `FormatResult`,
`FormatMeta`, `FormatError`, and `analyze_project`/`ProjectContext`). `FormatConfig` has public
fields and a `Default` impl, so it is configured with struct-update syntax.

The library is byte-oriented: it preserves source bytes outside the formatting contract, including
non-UTF-8 bytes in comments and strings.

The remaining modules (`source`, `classify`, `transform`, `format`, `io`, `cli`) are public only
because the binary, the integration tests and the fuzz targets are separate crates. They are
implementation details, are not covered by semantic versioning, and change with the pipeline.

## Development

The main implementation is under `src/`; the Python wheel launcher is under `forformat_runner/`.
Tests and golden fixtures are in `tests/`. The frozen Python reference and differential tools are
in `tools/reference/`.

Run the normal local checks with:

```sh
cargo test --locked --all-targets
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

For changes to full-mode normalization, wrapping, or layout, also run the invariant and corpus
checks when the CAMB verification checkout is available:

```sh
python3 tools/check_invariants.py
sh tools/check_camb_corpus.sh
```

To check that a built wheel actually works before it is published — the same check both CI
workflows run on every platform:

```sh
python -m build --wheel --outdir dist
sh tools/check_wheel.sh dist
```

Documentation:

- [`AGENTS.md`](AGENTS.md) — the short orientation, and the checks a change has to pass.
- [`docs/design.md`](docs/design.md) — why the pipeline has this shape; the design of record.
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
