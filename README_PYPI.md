# forformat

`forformat` formats free-form Fortran source files. It provides findent-compatible indentation,
plus an optional full-format mode for selected lexical normalization and wrapping.

The package installs the `forformat` command. It requires Python 3.9 or newer and does not provide
an importable Python formatting API.

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

### Example profile

This profile mirrors the indentation settings used by CAMB. The final
`--indent-contains=restart` setting is intentional; options may be written with hyphens or
underscores.

```sh
forformat \
	--full \
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
