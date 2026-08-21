# forformat

[![CI](https://github.com/cmbant/forformat/actions/workflows/ci.yml/badge.svg)](https://github.com/cmbant/forformat/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/forformat.svg)](https://pypi.org/project/forformat/)

`forformat` is a standalone formatter for free-form Fortran. It combines findent-compatible
indentation with lexical normalization, project-aware identifier casing, and statement wrapping.
The native formatter is written in Rust; published Python wheels bundle the executable, so using a
wheel does not require Rust or a Fortran compiler.

Automatic fixed/free input detection is enabled by default. Sources detected as fixed form are
left unchanged; use `-ifree` or `--input-format=free` to force free-form handling. Fixed-form output
is not supported.

## Install

From PyPI:

```sh
python -m pip install forformat
forformat --version
```

The package requires Python 3.9 or newer.

For pre-commit, use the separate hook repository:

```yaml
repos:
  - repo: https://github.com/cmbant/forformat-pre-commit
    rev: v0.1.3
    hooks:
      - id: forformat
      # - id: forformat-check  # check only; do not rewrite
```

To build from source, install Rust 1.85 or newer:

```sh
cargo build --locked --release
"${CARGO_TARGET_DIR:-target}/release/forformat" --version
```

## Quick start

Format files in place:

```sh
forformat src/module.f90
forformat src/*.f90
forformat src/
```

Explicit paths use declarations from the surrounding project for identifier case resolution; add
`--isolated` for standalone formatting.

Check formatting without changing files, or print a diff:

```sh
forformat --check src/*.f90
forformat --diff src/*.f90
```

Format every tracked free-form source in the current checkout:

```sh
forformat --all-files
```

Use stdin/stdout in a pipeline:

```sh
forformat --stdin < src/module.f90 > /tmp/module.f90
forformat --stdout src/module.f90 > /tmp/module.f90
```

For an editor buffer that needs declarations from the project, identify the buffer's source path:

```sh
forformat --stdin --project-context=src/module.f90 < src/module.f90
```

The stdin bytes replace that file's stale on-disk contents during project analysis; only the stdin
buffer is formatted. Pass a directory instead when the buffer has no corresponding source file.

### Formatting modes

`--full` is the default. The four modes are:

- `--full` — normalization, wrapping, and findent-compatible layout.
- `--indent-only` — findent-compatible indentation and trailing-whitespace handling only.
- `--normalize-only` — normalization without structural layout or wrapping.
- `--canonicalize-only` — canonical transformations without whitespace or layout normalization.

For example:

```sh
forformat --indent=4 --indent-module=0 --indent-procedure=0 src/module.f90
forformat --keyword-case=upper --line-length=100 src/module.f90
```

Options configure formatter policy; they do not correspond one-for-one with internal formatting
passes. Some safe full-mode transformations are part of the mode contract rather than separate
switches.

See **[CLI and configuration options](docs/options.md)** for the main user-facing option reference,
defaults, configuration keys, and small examples. Legacy findent spellings that are not useful in
normal operation are kept in the migration and compatibility docs instead. `forformat --help` is
the compact terminal summary.

## Project configuration

Project settings can live in a top-level `.forformat.toml`, the compatibility spelling
`.findent.toml`, or `[tool.forformat]` in `pyproject.toml`:

```toml
[tool.forformat]
mode = "full"
indent = 4
line_length = 100
keyword_case = "lower"
context_paths = ["src", "modules"]
exclude = ["vendor/"]
extend_exclude = ["**/generated-*.f90"]
```

Hyphens and underscores are equivalent in configuration keys. Command-line scalar options override
project settings. `--exclude` replaces the configured exclusion set; `--extend-exclude` adds to it.
Workflow choices such as `--all-files`, `--check`, `--diff`, `--stdin`, and `--stdout` remain
command-line-only. See [the options reference](docs/options.md#configuration) for discovery,
precedence, and the full list of command-line-only settings.

## Python API

Python callers can format text or bytes with the bundled native executable:

```python
from forformat import format_source

formatted = format_source(
    source,
    options=("--config=/absolute/path/to/.forformat.toml",),
    repo_context_path="/path/to/checkout/src/module.f90",
)
```

The return type matches the input type. Automatic configuration discovery is disabled for this API
unless `options` explicitly supplies `--config`.

## Documentation

- [CLI and configuration options](docs/options.md) — primary user-facing option reference.
- [File and project workflow](docs/file-workflow.md) — tracked-file selection, project context,
  exclusions, submodules, stdin identity, and fixed/free detection.
- [Full-mode guide](docs/full-mode.md) — normalization, wrapping, layout, invariants, and code map.
- [Compatibility](docs/compatibility.md) — the findent compatibility boundary and reviewed
  divergences.
- [Migrating from findent](docs/migration.md) — legacy option mapping and unsupported features.

## Development

The Rust implementation is under `src/`; tests and golden fixtures are under `tests/`. Run:

```sh
./tools/check_local.sh
```

For changes to full-mode normalization, wrapping, or layout, also run
`./tools/check_fuzz_regression.sh` and the relevant focused properties. See [`AGENTS.md`](AGENTS.md)
for the full repository verification bar.

## Relationship to findent

`--indent-only` implements the findent-compatible indentation contract for findent 4.3.7; full mode
intentionally adds behavior beyond that contract. See [compatibility](docs/compatibility.md) for the
reviewed differences and [migration](docs/migration.md) for legacy option mapping.

The project is BSD-3-Clause licensed. Third-party attribution and license terms are in
[`LICENSE-THIRD-PARTY`](LICENSE-THIRD-PARTY).
