# forformat

`forformat` is a Python-installable formatter for free-form Fortran source. It is a clean-room
Rust implementation of the core indentation behavior of [findent](https://github.com/cmbant/findent),
with an optional full-format mode that also normalizes selected lexical details and wraps long
statements.

The Python package installs the `forformat` command and includes a native executable in its wheel,
so users do not need Rust or a Fortran compiler. It is a command-line package rather than an
importable Python formatting library.

## Install

```sh
python -m pip install forformat
```

For a release artifact before the package is published on PyPI, install the compatible wheel by
path instead:

```sh
python -m pip install /path/to/forformat-0.1.0-py3-none-linux_x86_64.whl
```

The package requires Python 3.9 or newer and currently provides platform-specific wheels. Verify
the installation with:

```sh
forformat --version
```

## Use

Format one or more free-form Fortran files in place:

```sh
forformat src/module.f90
forformat src/*.f90
```

For pipelines, read from standard input and write to standard output:

```sh
forformat --stdin < src/module.f90 > /tmp/module.f90
forformat --stdout src/module.f90 > /tmp/module.f90
```

Use `--check` in CI to fail when files need formatting, or `--diff` to print unified diffs:

```sh
forformat --check src/*.f90
forformat --diff src/*.f90
```

The default is `--full`. Use `--indent-only` for findent-compatible indentation and trailing
whitespace handling, or `--help` for the complete option list. Fixed-form Fortran and automatic
format detection are not supported.

## Rust formatter

The repository also contains the standalone Rust implementation used to build the Python wheel.
It reads bytes from stdin and writes formatted bytes to stdout. The default is full formatting: it
applies findent-compatible indentation plus the documented lexical normalization and wrapping
passes.

```sh
cargo run --release -- -ifree < source.f90 > source.f90.formatted
```

Use `--indent-only` when adopting only findent-compatible indentation and trailing-horizontal-space
handling. Use `--full` explicitly in scripts that want to state the full-format policy; it is the
default. Full mode intentionally differs from the reference for multiline array
constructors, conservative comment bodies, kind suffixes on continuation lines, governing
declarations, `!$` sentinel spacing, and `--ws_remred` inside valid literals. The complete rationale
and examples are in [docs/compatibility.md](https://github.com/cmbant/forformat/blob/main/docs/compatibility.md);
migration guidance is in
[docs/migration.md](https://github.com/cmbant/forformat/blob/main/docs/migration.md).

With the reference installation available, `tools/differential_free.sh target/release/forformat`
checks the retained legacy fixtures against findent 4.3.7 byte-for-byte.

The public library API is `forformat::format_source` / `forformat::format_to` /
`forformat::format_to_owned`. Fixed-form conversion,
relabeling, dependency extraction, and editor payload generation are intentionally out of scope;
see [docs/compatibility.md](https://github.com/cmbant/forformat/blob/main/docs/compatibility.md).
Migration notes and the supported-option matrix are in
[docs/migration.md](https://github.com/cmbant/forformat/blob/main/docs/migration.md).

The formatter is a clean-room Rust reimplementation informed by findent 4.3.7. Attribution and the
BSD-3-Clause terms are included in
[NOTICE](https://github.com/cmbant/forformat/blob/main/NOTICE) and
[LICENSE-THIRD-PARTY](https://github.com/cmbant/forformat/blob/main/LICENSE-THIRD-PARTY); this
project's own license is [LICENSE](https://github.com/cmbant/forformat/blob/main/LICENSE).

## Development container

This workspace builds the current native Findent source from the SourceForge
SVN trunk during the devcontainer image build. The resulting `findent` and
`wfindent` commands are installed in `/usr/local/bin`.

Open this folder in VS Code and run **Dev Containers: Rebuild and Reopen in
Container**. Verify the installation with:

```sh
findent -h
wfindent -h
```

The checked-out source remains available in `/opt/findent` for inspection.
