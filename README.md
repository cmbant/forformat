# Findent Rust Formatter

This repository contains a standalone, free-form-only Rust implementation of the Findent
formatter. It reads bytes from stdin and writes formatted bytes to stdout, preserving source
spelling while adjusting indentation and removing trailing spaces/tabs:

```sh
cargo run --release -- -ifree < source.f90 > source.f90.indented
```

With the reference installation available, `tools/differential_free.sh target/release/findent`
checks the retained legacy fixtures against findent 4.3.7 byte-for-byte.

The public library API is `findent::format_source` / `findent::format_to` /
`findent::format_to_owned`. Fixed-form conversion,
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
