# Working in this repo

`forformat` is a Rust formatter for free-form Fortran. `--indent-only` implements the
findent-compatible indentation contract; full mode also normalizes lexical details and wraps
long statements. The pipeline is:

```
bytes -> normalization (steps 1-15) -> wrapping (16) -> findent layout engine -> post-layout (17-20)
```

Read the module docs at the top of `src/format/full/mod.rs` before changing it. Two invariants gate
the pipeline:

- **I2** `indent_only(full(x)) == full(x)` — free, because the final bytes are the engine's output
  over normalized text. Never let a pass choose a column outside the engine.
- **I1** `full(full(x)) == full(x)` — not free. It is a per-pass obligation and where most bugs live.

Two rules I1 keeps re-teaching:

- Wrapping runs before layout, so every budget decision measures text as it will be emitted.
  Normalization widens lines, the engine moves them, and declaration-separator alignment is the
  only post-layout pass that can make a line longer. Measure via `engine::format`.
- Physical lines are not statement boundaries. `Analysis::StatementFacts` caches stable facts per
  logical statement, and source provenance selects the first owner for current-line context and the
  last owner for continuation carry. Dynamic line-rule state resets at real semicolons instead of
  leaking state from an earlier statement on the same physical line.

## Verification

`.githooks/pre-commit` runs `cargo fmt --check` and Clippy whenever a commit stages a `.rs` file or
a manifest. Clippy uses CARGO_TARGET_DIR, separate from the default Cargo target, so its metadata-only
artifacts cannot interfere with `cargo test`. Enable the hook once per clone (the devcontainer does this on create):

```sh
git config core.hooksPath .githooks
```

Run the full local bar (mirrors `.github/workflows/rust-checks.yml` and the `release` job in
`ci.yml`) sequentially:

```sh
./tools/check_local.sh
```

The devcontainer sets `CARGO_TARGET_DIR=/tmp/forformat-target` for normal builds; Clippy overrides it with
`CARGO_TARGET_DIR=/tmp/forformat-lint`. Outside the devcontainer, tests and formatter binaries use the
default `target/` directory.

Two cargo commands must never share a target directory at the same time. They take the same
fingerprint and binary paths, so a concurrent run rebuilds a binary out from under the other and the
failure looks like a real regression rather than a collision -- `check_local.sh` has been seen fail
with `target/release/forformat: No such file or directory` this way. Give every parallel invocation
its own `CARGO_TARGET_DIR`, and before believing a `check_local.sh` failure, check that only one is
running.

Only when a change touches packaging (`pyproject.toml`, `setup.py`, `forformat/`, `forformat_runner/`,
or `.github/workflows/pypi.yml`), also build and check the wheel:

```sh
python -m build --wheel --outdir dist && bash tools/check_wheel.sh dist
```

Run the non-default profiles too. `--align-paren`, `--indent=8`, and `--line-length=80` each
exercise distinct wrapping or layout paths. The requirement is zero non-idempotent files and zero
crashes for every profile.

`cargo bench` runs `benches/throughput.rs`, which reports peak RSS next to every timing. Its second
half builds a synthetic multi-file project — a USE chain, a type spelled like a module, component
chains, a shared INCLUDE — and formats it in normalize-only and full mode, because those are the
only modes that run project-aware resolution at all. Run it for any change to analysis, name
resolution, or the case passes. Time and memory have already come apart once here: a change that
left throughput flat took that half from 56 MB to 77 MB.

When a fixture difference exposes a bug, reduce it to a minimal snippet, add a fixture and a
manifest row, fix it, and rerun the checks. Keep expected findent outputs unchanged unless the
compatibility contract itself changes.

The devcontainer has original findent installed at /opt/findent.

## Documentation

Keep `docs/options.md` as the primary reference for normal user-facing options. Legacy findent
spellings that are only compatibility aliases can stay in the migration/compatibility docs. When a
long option advertised by `--help` or a configuration key changes, update the reference and check main READMEs.
`tools/check_docs.sh` checks advertised help/reference coverage, local Markdown links, stale
fixed/free wording, and the quick-start formatter examples; it is not an exhaustive parser inventory.

## Traps

- Do not use `git stash` to A/B a build: it resets the index for stashed paths, and `pop` does not
  put staged work back. Swap file contents in place instead.
- `cargo test` has been observed running against a stale library after alternating with Clippy in
  the same target directory. Use `./tools/check_local.sh`, or set
  `CARGO_TARGET_DIR=/tmp/forformat-lint` for manual Clippy runs. If a test result contradicts what the
  release binary does on the same input, do not debug the difference: run `cargo clean -p forformat`
  and re-run first. Instrumenting a function and seeing *no* output from it is the giveaway.
- Full-mode passes must preserve protected literal, Hollerith, preprocessor, and comment bytes
  except for the explicitly documented comment rule.
- A continuation line has no statement context of its own. Read stable facts from the owning
  logical statement; on a semicolon line current context belongs to the first statement and carried
  continuation state to the last. Never infer statement kind from the continuation's first token.
- A new post-layout width change must be included in the wrapper's emitted-width measurement.
- Never hand-roll a quote scanner. `src/source/regions.rs` owns lexical truth, and
  `advance_stream_line` owns the carried state for both `SourceBuffer` and step 11. Line rules may
  scan scratch copies to find protected bytes on the current line, but must not create another
  carried-state path. Rewrites use `map_code` or `tokenize`.
- A continued statement *steps over* blank, comment and directive lines. `advance_stream_line`
  decides whether a stream lexes a code line or treats it as transparent, and resets an
  unterminated literal when continuation syntax does not keep it open. A bare raw scan is not an
  equivalent source-stream reader.
- `cargo test --all-targets` does not run doctests. `cargo test --doc` is a separate step in
  `check_local.sh` and in CI.
- `fuzz/` is its own Cargo workspace, so a repo-root `cargo fmt`/`cargo clippy` does not reach the
  fuzz targets. Both are run against `fuzz/Cargo.toml` separately by `check_local.sh` and CI.
