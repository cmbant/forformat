# Working in this repo

`forformat` is a clean-room Rust port of findent's indentation, plus a full mode that
normalizes lexical details and wraps long statements. The Python reference it was ported
from is kept, frozen, in `tools/reference/`.

## The pipeline shape is the design

```
bytes -> normalization (steps 1-15) -> wrapping (16) -> findent layout engine -> post-layout (17-20)
```

Read the module docs at the top of `src/format/full.rs` before changing any of it. Two
invariants gate everything:

- **I2** `indent_only(full(x)) == full(x)` — free, because the final bytes *are* the engine's
  output over normalized text. Never let a pass choose a column outside the engine.
- **I1** `full(full(x)) == full(x)` — not free. It is a per-pass obligation, and it is where
  the bugs live.

Two rules that I1 keeps re-teaching:

- **Wrapping runs before layout, so any budget decision must measure the text as it will be
  emitted, not as it was read.** Normalization widens lines, the engine moves them, and step 17
  (`declaration_separator_alignment`) pads a `::` — that is the only post-layout pass that can
  make a line *longer*. Measure via `engine::format`, do not re-derive columns.
- **Per-line normalization has no statement context.** A continuation line carries no `format`,
  no `::`, no `call`. Anything the rule needs about the statement must be threaded in through
  `LineOptions.continued_*` (`src/transform/passes/line_rules.rs`).

## Final verifications

```sh
cargo test --release && cargo fmt --check && cargo clippy --release --all-targets
python3 tools/check_invariants.py          # I1/I2 over perturbed CAMB input
sh tools/check_camb_corpus.sh              # wants: non-idempotent 0, decline diagnostics 0
```

`CAMB/` and `cosmomc/` are untracked developer verification targets, never test data.
Only inspect on request or used by specific tests. The
workflow on any corpus difference is to reduce it to a minimal snippet, add a fixture and a
manifest row, fix it, and rerun (reset CAMB or cosmomc to HEAD recursively to return to original state).
When touching wrapping or layout, A/B the corpus output
against the pre-change binary — the fix should move only the cases it claims to.

CAMB is a *joint fixed point* of findent and the Python reference, so it can only show harm, never
absence.

Run the non-default profiles too. `--align-paren`, `--indent=8` and `--line-length=80` each exposed
a distinct class that 120 columns hid; the bar is zero non-idempotent files and zero crashes on
every profile.

One pass must reach the fixed point (`--check` clean immediately afterwards), and `findent`
with the same profile must be a no-op on the result.

Mostly should agree with tools/reference/standardize_fortran.py --all, but python is a bit behind
and comment handling is slightly different.

## Files that must be kept in sync

- `src/transform/vocab.rs` is generated from `tools/reference/standardize_fortran.py` (embeds
  its sha256). Run `python3 tools/gen_vocab.py` after editing the reference script; CI enforces
  this with `gen_vocab.py --check`.
- `docs/traceability.md` (or wherever `tools/gen_traceability.py` writes) is generated from the
  frozen Python test suite — one row per reference test. Run `python3 tools/gen_traceability.py`
  after adding/removing a reference test; CI enforces this with `--check`. The last three columns
  are hand-maintained and preserved across regeneration.

## Traps

- `tools/reference/standardize_fortran.py --all` resolves the repository from the *script's*
  location, not the cwd, so running it inside a corpus checkout reformats this repo's
  `tests/fixtures/`. Pass explicit paths, and `--isolated` to skip project scanning.
- Do not `git stash` to A/B a build: it resets the index for the stashed paths, and `pop` does
  not put staged work back. Swap file contents in place instead.
