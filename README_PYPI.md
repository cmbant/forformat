# forformat

`forformat` formats free-form Fortran. Full formatting is the default: it combines
findent-compatible indentation with lexical normalization, project-aware identifier casing, and
statement wrapping. `--indent-only` provides the findent-compatible indentation contract.

The package installs the `forformat` command and an importable in-memory API. It requires Python
3.9 or newer. Published wheels bundle the native Rust executable, so they do not require Rust or a
Fortran compiler; Rust is only needed when building from source.

## Install

```sh
python -m pip install forformat
forformat --version
```

For pre-commit, use the separate hook repository:

```yaml
repos:
  - repo: https://github.com/cmbant/forformat-pre-commit
    rev: v0.1.3
    hooks:
      - id: forformat
      # - id: forformat-check  # check only; do not rewrite
```

## Quick start

Format files in place:

```sh
forformat src/module.f90
forformat src/*.f90
forformat src/
```

Check or preview changes without rewriting files:

```sh
forformat --check src/*.f90
forformat --diff src/*.f90
```

Format every tracked free-form source in the current checkout, or check the whole checkout:

```sh
forformat --all-files
forformat --all-files --check
```

Use stdin/stdout in a pipeline:

```sh
forformat --stdin < src/module.f90 > /tmp/module.f90
forformat --stdout src/module.f90 > /tmp/module.f90
```

The default input mode automatically detects fixed versus free form. A source detected as fixed
form is skipped unchanged. Use `-ifree` or `--input-format=free` to force free-form handling, and
`--query-format` to print the detected form. Fixed-form output is unsupported.

## Modes and common options

- `--full` — normalization, wrapping, and structural layout; this is the default.
- `--indent-only` — findent-compatible indentation and trailing-whitespace handling.
- `--normalize-only` — normalization without structural layout or wrapping.
- `--canonicalize-only` — canonical transformations without whitespace or layout normalization.
- `--canonicalize-and-indent` — canonical transformations followed by findent-compatible indentation, without wrapping or full-mode post-layout alignment.

For example:

```sh
forformat --indent=4 --indent-module=0 --indent-procedure=0 src/module.f90
forformat --keyword-case=upper --line-length=100 src/module.f90
forformat --canonicalize-and-indent src/module.f90
```

If authored internal spacing must be preserved while both canonical spelling and structural
indentation are wanted, use the combined mode:

```sh
forformat --canonicalize-and-indent src/module.f90
```

It is defined as canonicalize-only followed by indent-only with the same settings, so it preserves
authored interior spacing, does not wrap statements, and does not run full-mode alignment passes.

If all spelling must also be preserved, use the indentation-only mode:

```sh
forformat --indent-only src/module.f90
```

This changes indentation and trailing whitespace but deliberately does not apply keyword casing or
full-mode normalization.

The main option reference, including defaults, project settings, and file-selection options, is in
the [project documentation](https://github.com/cmbant/forformat/blob/main/docs/options.md).

## Project context and configuration

Explicit file paths use declarations from the surrounding Git checkout for project-aware case
resolution. Use `--isolated` when each file should be processed independently. For an editor buffer
on stdin, identify its source file so the buffer shadows the stale on-disk copy during analysis:

```sh
forformat --stdin --project-context=src/module.f90 < src/module.f90
```

Project settings can live in `.forformat.toml`, the compatibility spelling `.findent.toml`, or
`[tool.forformat]` in `pyproject.toml`:

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

## Python API

```python
from forformat import format_source

formatted = format_source(
    source,
    options=("--config=/absolute/path/to/.forformat.toml",),
    repo_context_path="/path/to/checkout/src/module.f90",
)
```

The return type matches the input type. Configuration discovery is disabled for this API unless
`options` explicitly supplies `--config`.
