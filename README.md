# Forformat Rust Formatter

This repository contains a standalone, free-form-only Rust implementation of the Findent
formatter. It reads bytes from stdin and writes formatted bytes to stdout. The default is full
formatting: it applies findent-compatible indentation plus the documented lexical normalization
and wrapping passes.

```sh
cargo run --release -- -ifree < source.f90 > source.f90.formatted
```

Use `--indent-only` when adopting only findent-compatible indentation and trailing-horizontal-space
handling. Use `--full` explicitly in scripts that want to state the full-format policy; it is the
default. Full mode intentionally differs from the frozen reference for multiline array
constructors, conservative comment bodies, kind suffixes on continuation lines, governing
declarations, `!$` sentinel spacing, and `--ws_remred` inside valid literals. The complete rationale
and examples are in [docs/compatibility.md](docs/compatibility.md); migration guidance is in
[docs/migration.md](docs/migration.md).

With the reference installation available, `tools/differential_free.sh target/release/forformat`
checks the retained legacy fixtures against findent 4.3.7 byte-for-byte.

The public library API is `forformat::format_source` / `forformat::format_to` /
`forformat::format_to_owned`. Fixed-form conversion,
relabeling, dependency extraction, and editor payload generation are intentionally out of scope;
see [docs/compatibility.md](docs/compatibility.md).
Migration notes and the supported-option matrix are in [docs/migration.md](docs/migration.md).

The formatter is a clean-room Rust reimplementation informed by findent 4.3.7. Attribution and the
BSD-3-Clause terms are included in [NOTICE](NOTICE) and [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY).

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
