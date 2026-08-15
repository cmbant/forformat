# forformat

`forformat` formats free-form Fortran source files. It provides findent-compatible indentation,
plus an optional full-format mode for selected lexical normalization and wrapping.

The package installs the `forformat` command and an importable in-memory API. It requires Python
3.9 or newer.

## Install

```sh
python -m pip install forformat
```

## Use

Format one or more Fortran files in place:

```sh
forformat src/module.f90
forformat src/*.f90
```

Use standard input and output in a pipeline:

```sh
forformat --stdin < src/module.f90 > /tmp/module.f90
forformat --stdout src/module.f90 > /tmp/module.f90
```

Or format a Python string or byte buffer directly:

```python
from forformat import format_source

formatted = format_source(
    source,
    options=("--config=/absolute/path/to/.forformat.toml",),
    repo_context_path="/path/to/checkout/src/module.f90",
)
```

The return type matches the input type. Configuration discovery is disabled
for this API unless `options` explicitly supplies `--config`.

Use `--check` in CI to fail when files need formatting, or `--diff` to print unified diffs:

```sh
forformat --check src/*.f90
forformat --diff src/*.f90
```

### Project context

When you pass explicit paths, `forformat` scans the current Git checkout for free-form Fortran
sources and uses declarations from those files to resolve names while formatting. Only the paths
you pass are changed. This project-aware behavior is useful when a declaration in one module
controls the spelling or formatting of code in another file.

For independent file processing, use `--isolated` with explicit paths:

```sh
forformat --isolated src/module.f90
forformat --isolated --stdout src/module.f90 > /tmp/module.f90
```

Input from standard input is also processed without a repository scan:

```sh
forformat --stdin < src/module.f90 > /tmp/module.f90
```

Use `--project-context PATH` with `--stdin` when an editor buffer needs
declarations from a Git checkout without selecting a file to format. Pass the
buffer's source path to replace that file's stale on-disk contents with stdin
in project analysis; pass a directory only when the buffer has no corresponding
file.

Project settings can be kept in a top-level `.forformat.toml`, or in `[tool.forformat]` in a
Python project's `pyproject.toml`. Keys use the same names as the long command-line options:

```toml
[tool.forformat]
indent = 4
line-length = 100
align-paren = true
```

Command-line options override project settings. `--config PATH` selects a specific file and
`--no-config` disables configuration discovery.

### Example profile

This profile demonstrates an explicit four-space indentation configuration. The final
`--indent-contains=restart` setting is intentional; options may be written with hyphens or
underscores.

```sh
forformat \
	--indent=4 \
	--indent-module=0 \
	--indent-procedure=0 \
	--start-indent=4 \
	--indent-contains=0 \
	--openmp=0 \
	--indent-contains=restart \
	--indent-select=4 \
	--indent-case=4 \
	--indent-interface=0 \
	--indent-continuation=4 \
	--indent-ampersand \
	src/*.f90
```

The default is `--full`. Use `--indent-only` for findent-compatible indentation and trailing
whitespace handling. Run `forformat --help` for all options. Fixed-form Fortran and automatic
format detection are not supported.
