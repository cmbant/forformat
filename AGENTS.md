# Working in this repo

`forformat` is a Rust formatter for free-form Fortran. `--indent-only` implements the
findent-compatible indentation contract; full mode also normalizes lexical details and wraps
long statements. The pipeline is:

```
bytes -> normalization (steps 1-15) -> wrapping (16) -> findent layout engine -> post-layout (17-20)
```

Read the module docs at the top of `src/format/full.rs` before changing it. Two invariants gate
the pipeline:

- **I2** `indent_only(full(x)) == full(x)` — free, because the final bytes are the engine's output
  over normalized text. Never let a pass choose a column outside the engine.
- **I1** `full(full(x)) == full(x)` — not free. It is a per-pass obligation and where most bugs live.

Two rules I1 keeps re-teaching:

- Wrapping runs before layout, so every budget decision measures text as it will be emitted.
  Normalization widens lines, the engine moves them, and declaration-separator alignment is the
  only post-layout pass that can make a line longer. Measure via `engine::format`.
- Per-line normalization has no statement context. A continuation line carries no `format`, `::`,
  or `call`; carry required facts in `LineState` and expose them through `LineContext` in
  `src/transform/passes/line_rules/mod.rs`.

## Verification

`.githooks/pre-commit` runs `cargo fmt --check` and `cargo clippy` whenever a commit stages a `.rs`
file or a manifest. Those two are cheap enough to gate a commit (~7s warm); the rest of the bar
below stays in CI. Enable the hook once per clone (the devcontainer does this on create):

```sh
git config core.hooksPath .githooks
```

Run the full local bar (mirrors `.github/workflows/rust-checks.yml` and the `release` job in
`ci.yml`):

```sh
cargo test --locked --all-targets
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
./tools/check_fixture_syntax.sh target/debug/forformat
./tools/check_fuzz_regression.sh
./tools/check_cli_contract.sh target/debug/forformat
./tools/check_docs.sh target/debug/forformat

cargo build --locked --release
./tools/check_cli_contract.sh target/release/forformat
./tools/check_package.sh
```

Only when a change touches packaging (`pyproject.toml`, `setup.py`, `forformat/`, `forformat_runner/`,
or `.github/workflows/pypi.yml`), also build and check the wheel:

```sh
python -m build --wheel --outdir dist && bash tools/check_wheel.sh dist
```

Run the non-default profiles too. `--align-paren`, `--indent=8`, and `--line-length=80` each
exercise distinct wrapping or layout paths. The requirement is zero non-idempotent files and zero
crashes for every profile. `tools/differential_free.sh` remains the findent-oracle check for the
retained legacy free-form fixtures.

When a fixture difference exposes a bug, reduce it to a minimal snippet, add a fixture and a
manifest row, fix it, and rerun the checks. Keep expected findent outputs unchanged unless the
compatibility contract itself changes.

The devcontainer has original findent installed at /opt/findent.

## Documentation

Keep `docs/options.md` as the primary reference for normal user-facing options. Legacy findent
spellings that are only compatibility aliases can stay in the migration/compatibility docs. When a
long option advertised by `--help` or a configuration key changes, update the reference.
`tools/check_docs.sh` checks advertised help/reference coverage, local Markdown links, stale
fixed/free wording, and the quick-start formatter examples; it is not an exhaustive parser inventory.

## Traps

- Do not use `git stash` to A/B a build: it resets the index for stashed paths, and `pop` does not
  put staged work back. Swap file contents in place instead.
- `cargo test` has been observed running against a stale library after alternating with
  `cargo clippy`, which shares the dev profile. If a test result contradicts what the release
  binary does on the same input, do not debug the difference: run `cargo clean -p forformat` and
  re-run first. Instrumenting a function and seeing *no* output from it is the giveaway.
- Full-mode passes must preserve protected literal, Hollerith, preprocessor, and comment bytes
  except for the explicitly documented comment rule.
- A continuation line has no statement context of its own. Thread facts into line rules rather
  than inferring them from the continuation's first token.
- A new post-layout width change must be included in the wrapper's emitted-width measurement.
- Never hand-roll a quote scanner. `src/source/regions.rs` owns the lexical state; take a byte scan
  from `for_each_code_byte`/`for_each_code_span` and a rewrite from `map_code` or `tokenize`. Every
  private copy so far has gone wrong the same two ways: closing `'a''b'` on the first byte of the
  doubled pair, and forgetting to advance the cursor inside a literal.
- A continued statement *steps over* blank, comment and directive lines: they are emitted verbatim
  and never lexed. `line_comment_start`/`line_code_spans` walk a group's physical lines through them and carry one `LexState` across the whole group.
