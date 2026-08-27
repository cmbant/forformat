# forformat

`forformat` formats free-form Fortran. Full formatting is the default: it combines
findent-compatible indentation with lexical normalization, project-aware identifier casing, and
statement wrapping. `--indent-only` provides the findent-compatible indentation contract.

The package installs the `forformat` command and an importable in-memory API. It requires Python
3.9 or newer. Published wheels include the formatter, so normal installation does not require Rust
or a Fortran compiler.

## Install

```sh
python -m pip install forformat
forformat --version
```

For pre-commit, use the separate hook repository:

```yaml
repos:
  - repo: https://github.com/cmbant/forformat-pre-commit
    rev: v0.1.4
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

For an editor buffer, give stdin the filename it represents:

```sh
forformat --stdin-filename=src/module.f90 < src/module.f90
```

The filename supplies configuration and project discovery, source-form detection, relative
`INCLUDE` resolution, and diagnostics. It may name a new file whose parent already exists. Use
`--project-context=/path/to/other/checkout` only when semantic project context should come from a
different Git checkout.

Automatic fixed/free input detection is enabled by default. A source detected as fixed form is
skipped unchanged. Use `-ifree` or `--input-format=free` to force free-form handling, and
`--query-format` to print the detected form. Fixed-form output is unsupported.

## Modes and common options

- `--full` — normalization, wrapping, and structural layout; this is the default.
- `--indent-only` — findent-compatible indentation and trailing-whitespace handling.
- `--normalize-only` — normalization without structural layout or wrapping.
- `--canonicalize-only` — canonical transformations without whitespace or layout normalization.
- `--canonicalize-and-indent` — canonical transformations followed by findent-compatible
  indentation, without wrapping or full-mode post-layout alignment.

Normalizing modes target Fortran 2003 output by default. Use `--target-standard=f95` (or
`target_standard = "f95"` in configuration) to prevent the formatter from introducing syntax newer
than Fortran 95. The target constrains formatter-generated syntax; it does not validate or downgrade
syntax already present in the input.

Common examples:

```sh
forformat --indent=4 --indent-module=0 --indent-procedure=0 src/module.f90
forformat --keyword-case=upper --line-length=100 src/module.f90
forformat --canonicalize-and-indent src/module.f90
forformat --indent-only src/module.f90
```

The main option reference, including defaults, project settings, and file-selection options, is in
the [project documentation](https://github.com/cmbant/forformat/blob/main/docs/options.md).

For setup instructions for using `forformat` with VS Code, see the [VS Code setup guide](https://github.com/cmbant/forformat/blob/main/docs/vscode.md).

## Project context and configuration

Explicit file paths use declarations from the surrounding Git checkout for project-aware case
resolution. Use `--isolated` when each file should be processed independently. A named stdin buffer
uses `--stdin-filename=FILE` to derive the same project context and to shadow FILE's stale on-disk
copy when it is tracked. `--project-context=DIRECTORY` can override only the Git project used for
analysis.

Project settings can live in `.forformat.toml`, the compatibility spelling `.findent.toml`, or
`[tool.forformat]` in `pyproject.toml`:

```toml
[tool.forformat]
mode = "full"
target_standard = "f2003"
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
    filename="/path/to/checkout/src/module.f90",
    options=("--config=/absolute/path/to/.forformat.toml",),
)
```

The return type matches the input type. `filename` supplies file identity and default project
context; `repo_context_path` can override that project with a directory. Configuration discovery is
disabled for this API unless `options` explicitly supplies `--config`.
