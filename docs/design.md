# Design of record — full-mode formatting

**Status:** the design of record. The port it describes is complete; this document explains *why*
the code is shaped the way it is, and is the place to record a deliberate change to that shape.

**Document map.** `AGENTS.md` is the short version — read it first. **This** document owns the
*design* of full-mode formatting: normalization, project casing, wrapping, file/project workflow.
[`full-mode.md`](full-mode.md) is the working reference for the rules and the standing checks.
[`compatibility.md`](compatibility.md) owns the findent-compatibility boundary, and
[`history/`](history/) holds the original port plan and its completed work ledger. Keep a fact in
exactly one of them.

## Implementation status

Every phase below has landed. Chunks A (per-line normalization), B (declaration engine), C (case
application), D (structure passes and wrapping integration), E (post-layout passes) and F/G
(file and project workflow, release) are all in the tree and validated against the frozen oracle.

| Phase | State |
|---|---|
| 0 — oracle and baselines | **done**: `tools/reference/` frozen with hashes, `converge.py`, `tests/reference/convergence-baseline.json` (34 fixtures, no cycles), `docs/traceability.md` (86 rows), `tools/check_camb_corpus.sh` |
| 1 — mode, config, CLI | **done**: formatting options and the Phase 10 file-workflow flags |
| 3 — shared lexical infrastructure | **done**: `source/regions.rs`, `source/tokens.rs`, statement provenance, duplicate scanners removed |
| 4 — simple normalization | **done**: all 19 Chunk A rules landed and differentially validated |
| 5 — project declaration and case analysis | **done**: resolution, scopes, the scope-ranged declared-name model (B9), every extractor B1-B8/B10, and case application (Chunk C). No perturbation produces a more-than-case difference. Derived-type inheritance (B11) is modelled by walking the `extends` chain — a deliberate divergence, since the oracle reaches the same output through a global fallback that is unsafe under our `(type, component)` keying |
| 6 — structure-changing cleanup | **done** as Chunk D: lexical joins, redundant-parenthesis removal, terminal `RETURN` removal |
| 7 — planner/emitter split | **done**: `format/planner.rs` owns every structural decision |
| 8 — wrapping | **done**: break-point engine plus Chunk D integration — continued-statement rewrap, inline-comment detachment, per-break parenthesis alignment, OpenMP sentinels, decline diagnostics |
| 9 — post-layout passes | **done** as Chunk E: `transform/passes/layout_post.rs` — declaration-separator alignment, program-unit spacing, blank-line limits, output whitespace |
| 10 — file and project workflow | **done** as Chunk F: `io/mod.rs` — path/`--all` selection, one project-analysis pass, bounded parallel formatting, atomic replacement, `--check`/`--diff` |
| 11-12 — packaging and release | **done** as Chunk G: `cargo package` verification, the CLI contract script, the PyPI wheel workflow with a per-platform install smoke test |

Two things are worth knowing before reading further, because they change what later sections have
to prove:

- **I2 is now structural.** Full mode emits its final bytes by running the indent-only engine over
  the normalized text, so `indent_only(full(x)) == full(x)` holds by construction rather than by
  testing (`src/format/full.rs`). I1 is still a per-pass obligation.
- **Corpus status is generated at runtime.** `tools/check_camb_corpus.sh` reports fixed-point,
  idempotence, width and decline metrics for whatever external CAMB checkout is supplied; those
  volatile totals are not part of this normative design.
- **The corpus alone cannot validate normalization.** CAMB is already a joint fixed point of
  findent and the Python formatter, so formatting it unchanged proves only that the rules are
  *harmless*, not that they are *correct* — an entirely inert pass scores the same. §8 now has a
  differential harness (`tools/reference/differential.py`) that first moves the input off the fixed
  point and then compares against `P(R(x))`. It found six defects in Chunk A alone that the corpus
  check could not see.
- **The harness must invoke the reference the way its own CLI does.** `format_text` takes the
  declaration case tables as *arguments*; calling it bare applies almost no declared casing. That
  was wrong for the whole of Chunk A's validation, so any "0 differing" claim from before
  2026-08-11 was measured against a weaker oracle than it appeared.
- **Chunk D is invisible to both standing checks.** No perturbation creates split tokens, redundant
  parentheses or terminal `RETURN`s, and the corpus is already a fixed point, so those passes score
  clean whether they work or do nothing. They were verified by direct comparison against the oracle
  on constructed input, and future changes there need the same.

---

# 1. Goal and contract

Build one Rust binary that does what CAMB's pre-commit pipeline currently does with two tools:

1. `CAMB/scripts/findent_fortran.py` — findent 4.3.7 with a fixed argument set;
2. `CAMB/scripts/standardize_fortran.py` — 4,288 lines of case, spacing and wrapping
   normalization.

The target is not a mechanical Python-to-Rust translation. It is a **stable combined formatting
contract**:

```text
input -> lexical/project analysis -> semantic-safe normalization
      -> findent-compatible structural layout -> wrapping using the real continuation policy
      -> emission -> post-layout cleanup -> stable output
```

## 1.1 Invariants

These are stated here once. Later sections reference them by number rather than restating them.

| # | Invariant |
|---|---|
| **I1** | `full(full(x)) == full(x)` — one pass reaches the fixed point, for every fixture, every corpus file, and every truncated prefix where formatting is defined. |
| **I2** | `indent_only(full(x)) == full(x)` — full output is a findent fixed point. This is what makes a single pass possible where the old pipeline needed two tools. |
| **I3** | Protected source is preserved byte-for-byte: string literal contents, doubled quotes, comment text (beyond the documented marker/commented-assignment transforms), CPP directive bodies, explicitly-defined macro spellings, non-UTF-8 bytes, and Hollerith-bearing statements. |
| **I4** | Project casing never guesses. A locally declared spelling wins; an unambiguous project-wide spelling applies where the file does not declare the name; anything ambiguous is left alone. Intrinsics and standard specifiers never override a declared identifier, and CPP macro names outrank both. |
| **I5** | Wrapping is safe: no token is split, continued character literals keep their required markers, CPP directives are never Fortran-wrapped, OpenMP continuations stay syntactically valid, named arguments are never mistaken for assignment break points, and a wrap never changes the continuation indentation the next pass would choose (a corollary of I2). |
| **I6** | `indent_only` mode remains byte-exact against findent 4.3.7 for every currently supported case. Nothing in this plan may regress it. |

## 1.2 Definition of done

For a representative source `x`, with project context built once from all project sources:

```text
y = full(x)
full(y) == y                 # I1
indent_only(y) == y          # I2
reference_python(y) == y     # compatibility period, supported cases
```

and the CAMB pre-commit hooks invoke one Rust binary instead of two Python scripts, with no
production dependency on Python.

## 1.3 Decisions already taken

- **Full formatting is the default mode.** `--indent-only` retains today's byte-exact findent
  4.3.7 behavior for compatibility and as the `R` side of the oracle.
- **Indentation option defaults stay findent's** (`indent=3`, etc.). CAMB's profile continues to
  be passed as explicit CLI arguments, exactly as `findent_fortran.py` does today. An optional
  TOML config is a convenience, not a prerequisite.
- **Distribution** is ultimately a Python package wrapping prebuilt binaries, replacing both hook
  scripts.

---

# 2. Reference oracle and baselines

Establish this before writing formatter code. Without it, later changes can only be judged by
subjective diffs.

## 2.1 The two reference tools

`R` = **findent 4.3.7**, the C++ original — installed at `/usr/local/bin/findent`, sources at
`/opt/findent` (SVN checkout, binary at `/opt/findent/src/findent`).

> Do **not** use Rust `indent-only` as `R` in the oracle. That would make the baseline depend on
> the artifact under test. Rust `indent-only` is separately proven equal to `R` by
> `tests/manifest.rs` and `tools/differential_free.sh`; that proof is the reason the substitution
> is safe *inside* the product, and unsafe inside the oracle.

`P` = **`CAMB/scripts/standardize_fortran.py`**, frozen. Copy it to `tools/reference/` with a
recorded hash and provenance, and do not edit it during the port.

## 2.2 CAMB's findent profile

`R` must be invoked with the exact arguments the project uses. They are duplicated verbatim in
`CAMB/scripts/findent_fortran.py` and `CAMB/.vscode/settings.json`:

```text
--indent=4 --indent_module=0 --indent_procedure=0 --start_indent=4
--indent_contains=0 --openmp=0 --indent_contains=restart --indent_select=4
--indent_case=4 --indent_interface=0 --indent_continuation=4 --indent_ampersand
```

Note `--indent_contains` appears twice (`0` then `restart`); the last wins. These values differ
from findent's own defaults and are **not** becoming Rust defaults — see §7.2.

## 2.3 The reference pipeline is one pass, not a loop

pre-commit runs `findent-fortran` then `standardize-fortran`, both filtered
`files: \.(?:f90|F90)$` with `exclude: ^forutils/`. There is no iteration. Convergence works
today only because the corpus is already a joint fixed point (§3.3).

One asymmetry to decide deliberately rather than inherit: the hooks exclude `forutils/`, but
`standardize --all` discovers sources with `git ls-files --recurse-submodules` and therefore
*rewrites* forutils as well. Both tools read forutils either way, because its declarations are
needed for case resolution.

## 2.4 Convergence driver

`tools/reference/converge.py` composes `R` then `P` until the output stops changing:

```python
current = input
seen = {hash(current): 0}
for _ in range(MAX_ITERATIONS):
    after = P(R(current, findent_args), options)
    if after == current:
        return FIXED_POINT(after)
    if hash(after) in seen:
        return CYCLE(...)
    seen[hash(after)] = ...
    current = after
```

Requirements: detect fixed points; detect 2-cycles and longer; cap iterations and fail loudly;
save every intermediate on failure; print the first differing lines with a category; support
wrap/no-wrap, explicit `-D` macros, and a project-context mode rather than isolated stdin only.

Classify each result as **strong fixed point** (`R(x) == x` and `P(x) == x`),
**composition-only** (`P(R(x)) == x` but one tool alone changes `x`), **cycle**, or
**non-convergent**. The Rust formatter targets strong fixed points. A composition-only fixed
point is a sign the two tools disagree about who owns a formatting decision — almost always
continuation layout — and needs an explicit design decision, not an averaged result.

## 2.5 Baseline report

Produce `tests/reference/convergence-baseline.json` over: every checked-in Rust fixture and
expected output; every Python regression input; the CAMB corpus with project context; generated
long-line variants; OpenMP and CPP/macro cases; CRLF and mixed-newline inputs; and the
malformed/truncated inputs the existing property tests already use.

Fields: `name`, `input_hash`, `options`, `iteration_count`, `status`, `strong_fixed_point`,
`output_hash`, optional `known_divergence_id`.

Python stays test/oracle infrastructure. The production formatter never invokes it.

---

# 3. Current assets

## 3.1 Rust — what to reuse

Reuse these; do not rewrite them.

| Asset | Why |
|---|---|
| `src/source/buffer.rs` — `SourceBuffer`, `PhysicalLine` | Byte-offset spans, newline preservation, non-UTF-8 tolerance, mixed CRLF. Property-tested over every prefix of every fixture. The best asset in the repo. |
| `src/classify/recognizers.rs` (781 lines) | Encodes hundreds of hard-won 4.3.7 edge cases. Extend `StatementInfo`; never rewrite the recognizer chain. |
| `src/format/stack.rs` — `IndentStack` | The `values`/`raw_values` split (visible-clamped vs structural depth) is exactly what a wrapper needs, and `snapshot()` makes speculative layout cheap. |
| `src/format/preprocessor.rs` | Branch snapshot/restore generalizes to any per-branch analysis. |
| `src/format/continuation.rs` — `ParenAlignmentState` | A quote-aware bracket-column tracker across physical lines: most of what a break-point chooser needs. |
| Test harness: manifest format, fixture/expected goldens, `properties.rs`, fuzz targets, `tools/*.sh`, CI reproducibility gates | Extend with a `mode` key rather than forking the driver. |

## 3.2 Rust — structural obstacles

In rough order of severity. Each is addressed by a phase in §6.

1. **Emission is fused to state mutation.** `format_buffer` (`src/format/engine.rs:66`) computes
   state and writes bytes inside one closure (`:92-336`); `last_indent`/`last_usable` accumulate
   as side effects. Nothing can look ahead. Wrapping needs the first-line indent and continuation
   policy *before* choosing break points. → Phase 7.
2. **The emitter is byte-copy-with-indent.** `emit_line_to_with_quote` (`src/format/emitter.rs:81`)
   writes the trimmed original through with leading spaces replaced and one all-or-nothing
   `replacement` escape hatch. It has no notion of a line as a sequence of rewritable spans. Its
   10-argument signature and the per-group `FormatConfig` clones at `engine.rs:102`/`:174` are
   the smell. → Phase 7.
3. **`LogicalStatement.text` is an owned copy with no offset mapping**
   (`src/source/logical_statement.rs:5`, joined at `:91`). Classification works on the copy;
   emission works on the original. Any transform that rewrites *content* needs "this token came
   from bytes `a..b` of physical line `n`". → Phase 3.
4. **No real tokenizer, and quote state is reimplemented about seven times** — `comment_start`
   (`buffer.rs:122`), `split_statements` and `tokens` (`scanner.rs`), `is_assignment`
   (`recognizers.rs:231`), `paren_alignment` and `ParenAlignmentState` (`continuation.rs`),
   `reduce_line_into` (`whitespace.rs:18`). `scanner::tokens` has no token kinds and merges
   `10abc` into one token. → Phase 3, the highest-leverage refactor and a *pure* one.
5. **No home for a project pass.** `format_source(&[u8], &FormatConfig)` is one-buffer,
   one-shot; `main.rs` reads stdin only; `cli.rs:294` rejects bare arguments. → Phase 5 and 10.
   Mitigating: `SourceBuffer.lines` is fully materialized before any group is visited, so a
   second pass *within* a file is free.
6. **Invariants that must be renegotiated, not quietly broken.** `tests/properties.rs:29`
   (body preserved except trailing whitespace), `:38` (only label padding may change), and
   `:94`/`:107` (keyword-case invariance of indent columns) all become false under full mode.
   Gate them on `mode == IndentOnly` and add the full-mode counterparts. Input line count ==
   output line count also dies with wrapping.
7. **Minor.** `PhysicalLine` spans are `u32`, fine per file but a project index needs a file id
   alongside. `FormatConfig` is cloned per group — must stop before it carries keyword or symbol
   tables. `panic = "abort"` and zero dependencies are worth preserving deliberately.

Dead public API that can be repurposed or deleted: `scanner::normalized_statement`
(`scanner.rs:119`), `continuation::paren_alignment` (`continuation.rs:17`),
`classify::findentfix::payload` (`findentfix.rs:3`, duplicated by
`logical_statement::fix_payload`).

## 3.3 The CAMB corpus — development verification, not test data

The reference corpus is **`CAMB/fortran/` and `CAMB/forutils/`**. The two directories must be
analyzed **together**: forutils supplies shared utility modules, types and components whose
declarations determine case resolution in `fortran/`. The corpus is an external, temporary
developer checkout, so its size and revision are intentionally not recorded here.

**The supplied corpus must be a joint strong fixed point.** Check it with:

```sh
python3 tools/check_project_mode.py
sh tools/check_camb_corpus.sh
```

The acceptance target is exact: **`full(x) == x` byte-for-byte for every supplied corpus file,
with project context spanning both directories.**

**What the corpus is not.** It is a developer verification target, not test data. CAMB sources are
never vendored into this repository, never committed as goldens, and never wired into
`cargo test`. The outer `.gitignore` carries `*.f90*` and `fortran/`, and CAMB is an untracked
nested git repository, so this is also the path of least resistance.

The workflow on any difference is: **reduce to a minimal snippet → add a unit test or a
`tests/fixtures/` + manifest case → fix → rerun the corpus check.** The committed suite stays
small and self-contained while still getting real-world pressure.

`tools/check_camb_corpus.sh <camb-path> <binary>` formats every file with project context,
report each file whose output differs from its input, plus changed-line counts, maximum output
line length, and any statement the wrapper declined to wrap. Never write into the CAMB tree.

The corpus provides real-world CPP, boundary-width and long-line pressure, while the generated
stress corpus of §8.4 provides deterministic wrapping coverage. Live corpus characteristics come
from the check scripts rather than this document.

## 3.4 Python — measured effort, by area

`CAMB/scripts/standardize_fortran.py`, 4,288 lines:

| Lines | Area | Notes |
|---|---|---|
| ~363 | Keyword/intrinsic/specifier/OpenMP vocabularies | Pure data, trivially portable |
| ~460 | Physical-line scanning and logical-statement assembly | Largely subsumed by existing Rust |
| ~1,250 | Declaration/scope/type analysis and case resolution | **The hard core.** Stack machines with subtle scope rules |
| ~1,100 | Per-line spacing and casing rules | Many small independent rules; easy to port incrementally, large test surface |
| ~450 | Wrapping and break-point selection | |
| ~280 | CLI and file I/O | |

The porting oracle is `CAMB/scripts/test_standardize_fortran.py` — **86 tests in 6 classes**
(`CommandLineTests`, `FormattingTests`, `DeclarationCaseTests`, `ContinuationTests`,
`SpacingTests`, `RegressionFixTests`), run from the CAMB root as:

```sh
python3 -m unittest scripts.test_standardize_fortran
```

Baseline verified 2026-08-11: **85 pass; 1 test fails with 8 subtest errors** (see §5.1). Every
one of the 86 needs a traceability row (§8.1).

---

# 4. Target architecture

## 4.1 Pipeline

```text
project files ──► SourceBuffer + region/token view (one lexical truth)
                       │
        ┌──────────────┴──────────────┐
        ▼                             ▼
  project declarations          local lexical facts
  modules/types/cases           macros/strings/Hollerith
        └──────────────┬──────────────┘
                       ▼
              normalization (code regions only)
                       ▼
              structural planner  ──► DocumentLayout
                       ▼
              wrap/reflow engine (consumes the layout)
                       ▼
              emitter
                       ▼
              post-layout cleanup ──► output bytes
```

## 4.2 Separate planning from emission

```rust
struct GroupLayout {
    first_indent: usize,
    structural_depth_after: usize,
    continuation: ContinuationPolicy,
    is_preprocessor: bool,
    is_openmp: bool,
    replacement: Option<Vec<u8>>,
}

struct DocumentLayout { groups: Vec<GroupLayout>, meta: FormatMeta }
```

The exact types can differ. The requirement does not:

> The wrapper must know the final first-line indentation and the active continuation policy
> before it decides where lines break.

`LogicalGroup::visit` is already generic over its sink, so the planner can literally be today's
closure with `emit_line_to_with_quote` replaced by `layout.push(...)`.

## 4.3 One lexical truth

Do **not** port the Python helpers `quote_after`, `inline_comment_start`, `code_context` and
quote masking as independent scanners. Extend the Rust source layer so every transform shares one
protected-region view:

```rust
enum RegionKind { Code, StringLiteral, Comment, Preprocessor, Hollerith }
struct Region { range: Range<usize>, kind: RegionKind }
```

Requirements: byte offsets only; no UTF-8 assumption; doubled-quote handling in exactly one
place; continued character-literal state crossing physical lines; comment start determined once;
Hollerith exceptions available to transforms; CPP lines carrying an explicit directive kind. This
also collapses the six duplicated state machines listed in §3.2.4, and is validatable by the
existing property tests before any new feature lands.

## 4.4 Module sketch

Additions to today's tree, not a mandated split:

```text
src/source/    + regions.rs, tokens.rs
src/analysis/    macros.rs, declarations.rs, scope.rs, types.rs, names.rs, project.rs
src/transform/ + case.rs, keywords.rs, operators.rs, delimiters.rs,
                 declarations.rs, comments.rs, openmp.rs, cleanup.rs
src/format/    + planner.rs, wrapping.rs, post.rs
src/io/          repository.rs, files.rs, diff.rs
```

The separations that matter are lexical representation / semantic analysis / text normalization /
structural planning / wrapping / emission and I/O. Prefer smaller modules where clearer.

---

# 5. Pass ordering, and the exact Python reference

Ordering matters because the output must be stable under both normalization and indentation.

## 5.1 The real Python order

`format_text` (`standardize_fortran.py:3887-4006`). This is the frozen reference; the earlier
draft of this document invented a different order. Rust deviations must be deliberate and
recorded.

| # | Pass | Granularity |
|---|---|---|
| 0 | `dominant_line_ending` — CRLF→LF for the pipeline, restored at the end | whole text |
| 1–3 | macro cases: `-D` names, `#define` spellings, then replacement in unquoted code | per line |
| 4 | `extract_procedure_cases` (if the caller passed none) | statements |
| 5 | **`replace_declared_cases`** — the whole case-normalization engine, runs *first* | per line + statement context |
| 6 | `join_lexical_token_continuations` — rejoin `&`-splits that cut a token | whole text |
| 7 | `remove_redundant_nested_parentheses` | whole text |
| 8–10 | re-extract procedure/scoped names if the line count changed; build per-line lookup tables | |
| 11 | **per-line loop, in this exact order**: `lowercase_line` → `normalize_keyword_spacing` → `normalize_write_output_spacing` → `normalize_delimiter_spacing` → `normalize_comment_spacing` | per line |
| 12–13 | `normalize_continuations`, `normalize_openmp_continuation_sentinels` | line list |
| 14–15 | `remove_terminal_procedure_returns`; re-extract if lines were removed | statements |
| 16 | `rewrap_lines` (gated by `wrap`) | per logical statement |
| 17 | `normalize_declaration_separator_alignment` — compresses alignment and supplies owed `::` spaces | line blocks |
| 18–19 | `normalize_program_unit_spacing`, `limit_blank_lines` | line list |
| 20–21 | `normalize_output_whitespace`; restore line endings | whole text |

## 5.2 Load-bearing details

**Re-run spacing after joining.** `rewrap_lines` (`:3817`) joins a continued statement and then
**re-runs `lowercase_line`, `normalize_keyword_spacing` and `normalize_delimiter_spacing` in the
same order as step 11** — the comment at `:3875` explains why. Without this, joined continuation
boundaries are mis-spaced. This must be an explicit design requirement of the Rust wrapper, not
something rediscovered by a failing fixture.

**Case replacement runs before the lexical joins**, not after, and the line-changing transforms
(6, 7, 14) each force a re-extraction of scope metadata. Any Rust reordering must preserve those
dependencies.

**Post-layout width changes must be measured.** Steps 17–19 run after wrapping. Declaration
alignment normally compresses authored padding, but a compact `integer::x` gains the spaces owed
around `::` and can grow by two columns. The wrapper measures that emitted spelling, and the layout
stage repeats when step 17 changes a width. Any future width-changing post-layout transform must
join that measurement and relayout contract.

---

# 6. Phase plan

Each phase names its scope, key files, tests and exit criterion. The dependency order deliberately
puts wrapping late: it is much easier once normalization and structural indentation are separately
testable and the wrapper can consume a stable layout.

## Phase 0 — Oracle and baselines

Freeze `P` into `tools/reference/` with a hash. Add `converge.py` with cycle detection and
intermediate capture (§2.4). Run convergence over Rust fixtures, Python fixtures, the CAMB corpus
with project context, and the option matrices (default, `--align-paren`, altered `-k`, OpenMP
on/off, wrap/no-wrap). Produce `convergence-baseline.json`. Seed the traceability table (§8.1)
with all 86 Python tests. Record any cycle or composition-only fixed point as a named issue.

**Exit:** every input has a known reference status — fixed point, known cycle, or explicit
unsupported case.

## Phase 1 — Mode, config and CLI scaffolding

Add internal `FormatMode { Full, IndentOnly, NormalizeOnly }`. `NormalizeOnly` need not be public;
it is very useful while the port is incomplete. Add `WrapConfig { enabled, line_length }` and
macro-case config. Add positional paths, `--all`, `--stdout`, `--check`, `--diff`, `--isolated`
mode validation. Preserve every findent short and long alias. Add a `mode` key to the manifest so
existing cases pin `indent-only` explicitly and stay byte-exact forever.

Keep `IndentOnly` as the *effective* default during the port; flip to `Full` in Phase 12, so the
change is one reviewable commit rather than a slow drift.

**Exit:** the CLI and config can express the final feature set while indent-only output is
unchanged (I6).

## Phase 2 — deliberately merged into Phase 3

## Phase 3 — Shared lexical infrastructure

Design the byte-oriented region/token iterator of §4.3. Rewrite `comment_start`,
`split_statements`, `tokens`, `paren_alignment` and `reduce_line_into` on top of it. Add span
provenance to `LogicalStatement` so a joined token maps back to `(physical_line, byte_range)`
(§3.2.3). Carry quote state across continued character literals in one carrier rather than two.

This is a **pure refactor**: zero behavior change, and the existing property and manifest tests
are the proof.

**Exit:** no transform needs its own quote parser, and every joined statement token can be mapped
back to source bytes.

## Phase 4 — Simple normalization

Everything that needs no project analysis: keyword and intrinsic case, standard specifiers, real
exponent markers (`E`/`D`), legacy relational operator modernization, operator spacing (including
unary vs binary `+`/`-` and the compact `*`, `/`, `**`, `//` behavior), named-argument detection,
delimiter and comma spacing, compound keywords (`goto`, `end if`, `else if`), statement
parenthesis spacing, `WRITE(...)item` spacing, `(/ ... /)` → `[ ... ]` except in `FORMAT` and
unsafe contexts, declaration attribute order and old-style unseparated declarations, comment
marker spacing and commented-out-assignment operators, `--uppercase-single-l`, and OpenMP
keyword/clause normalization with a macro-protection hook.

**Tests:** exact Python-reference output per rule; strings/comments/CPP preserved (I3);
normalize-only mode is idempotent.

**Exit:** all non-project normalization fixtures match `P` and are individually idempotent.

## Phase 5 — Project declaration and case analysis

The ~1,250-line hard core. Keep it out of the wrapping work entirely.

Extract declarations from `LogicalGroup` statement text rather than re-scanning raw files with
regexes. Cover: module/type/procedure names; procedure scope ranges, arguments, results and
locals; module variables and their types; derived-type component names and types; type-bound
procedures; `SELECT TYPE` aliases; old and new declaration styles; module specification vs
`contains` region; interface and type nesting; component-owner chain resolution through
`local_types` → `variable_types` → `component_types`.

Resolution rules (I4), mirroring `_case_for_file` (`:1589`): if the file itself declares the name
with exactly one spelling, that wins; otherwise, if the whole project agrees on one spelling, use
it; multiple local spellings or project ambiguity with no local declaration → omit entirely.
Module/symbol/type-procedure cases keep only byte-identical spellings; type maps keep names that
agree case-insensitively.

**Tests:** translated Python declaration tests; synthetic multi-file projects; semicolon and
continued declarations; multiple program units per file; procedure locals colliding with
intrinsics (`size`, `sign`, `precision`); components colliding with locals; nested `%` chains;
interface bodies vs module bodies; project-global vs target-local precedence.

**Exit:** case-normalized output matches `P` under project context on the synthetic projects and
the CAMB corpus, aside from separately tracked layout differences.

## Phase 6 — Structure-changing cleanup

Lexical token-continuation joins; safe redundant nested-parenthesis removal with the same
exclusions (procedure arguments and `associate` targets are protected; RHS, `if` and `while`
conditions are eligible); terminal bare `RETURN` removal only when it is the final single-line
statement before a procedure end, preserving inline-commented returns. Each of these changes the
line count, so rebuild source, statement and scope metadata afterwards — see §5.2.

**Exit:** structural cleanup matches the Python fixtures and reparses cleanly.

## Phase 7 — Planner/emitter split

Extract the structural transitions of `engine.rs:413` (`transition`) from emission, producing
`DocumentLayout` (§4.2). Preserve CPP snapshots, malformed-END recovery, labeled `DO`,
`contains_restart`, `last-indent`/`last-usable`, `refactor_end`, labels, includes, start and max
indent. Give the emitter a token/segment sequence to consume instead of a byte slice, retiring
the 10-argument signature and the per-group config clones. Keep a fast streaming path for
indent-only if it is worth it, but share one transition implementation.

**Exit:** every existing manifest expected output is unchanged in indent-only mode (I6), and
layout plans are available before emission.

## Phase 8 — Findent-aware wrapping

The crux of the port. See §8.

## Phase 9 — Post-layout passes

Declaration separator compression (never padding); program-unit spacing; `contains` spacing;
blank-line limits; trailing whitespace; full-mode dominant line ending; final newline. Assert no
post-wrap pass lengthens a line beyond its budget.

**Exit:** full output matches reference convergence on all layout fixtures and satisfies I2.

## Phase 10 — File and project workflow

Repository root discovery; tracked-extension discovery
(`.f90 .f95 .f03 .f08 .f18 .f23` plus uppercase); explicit path validation with extension
checking separated from existence (§5.1 of the fixes list); project source-set collection read
once per invocation; `--isolated`; `--stdout`; atomic in-place write preserving mode bits and
resolving symlinks to their target; `--check` reporting; unified `--diff` with repository-relative
paths; exit statuses.

Nested `git` invocations must drop `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` and
`GIT_INDEX_FILE` so the tool works correctly as a git hook — this is `_git_env`
(`standardize_fortran.py:37`) and is easy to omit.

**Exit:** Rust replaces the Python CLI for stream, one-file, multi-file and repository workflows.

## Phase 11 — Corpus, fuzz, compile and performance hardening

Run `tools/check_camb_corpus.sh` to green (§3.3), reducing every difference to a committed
fixture. Generate the long-line stress corpus (§8.4). Run `gfortran -fsyntax-only` on
self-contained fixtures and `-fopenmp` on OpenMP fixtures; individual CAMB files may need
generated `.mod` files or project macros, so use the project's own build for whole-corpus
validation rather than assuming per-file syntax checks work. Extend fuzzing with targets for the
region walker, declaration analyzer, project case resolver, wrapper, and the I1/I2 invariants.
Benchmark full mode and project analysis.

**Exit:** no crashes or cycles; all compatibility and convergence tests pass; the project still
builds; performance meets §9 Gate G.

## Phase 12 — Cutover

Flip the default to `Full` in one commit. Retain `--indent-only`. Update `README.md`,
`docs/compatibility.md` (distinguishing findent-compatible indentation from full-format
additions), `docs/migration.md`, and document every intentional divergence. Build the Python
package (§7.3) and switch the CAMB hooks to it. Keep the frozen Python reference under test
tooling for at least one release cycle; retire it only once the Rust corpus fully specifies the
behavior.

**Exit:** the Rust binary is the only production formatter and its test suite independently
specifies all supported behavior.

---

# 7. Wrapping, CLI, config and distribution

## 7.1 Wrapping must use findent's continuation layout

The Python formatter invents its own continuation indentation. That is exactly what must **not**
be copied. Division of labor:

- the **wrapper** decides *where* to break;
- the **layout plan** decides *where the next physical line begins*.

Per statement:

```text
target        = structural first-line indent (from GroupLayout)
remaining     = logical statement text
paren state   = empty

while remaining does not fit:
    available = line_length - target - width of the continuation suffix
    choose a safe break <= available
    build the physical line + " &"
    feed the emitted code into ParenAlignmentState
    target = continuation rule from the plan:
             parenthesis alignment when active, else the configured continuation indent
    remaining = rest

emit the final line at target
```

**Two reconciliations the old tools leave contradictory:**

- **Continuation indent is one value.** Python hard-codes `indent + 4`; findent is passed
  `--indent_continuation=4`. They agree only by coincidence. In Rust the wrapper reads the policy
  from the layout plan and never a literal — otherwise I2 fails the moment a user changes `-k`.
- **Leading `&`.** Python's `normalize_continuations` strips leading `&`, while the CAMB profile
  passes `--indent_ampersand` (`-K`). On the fixed-point corpus `-K` is inert because no
  continuation line starts with `&`. The wrapper must never emit a leading `&`; `-K` continues to
  govern how a *pre-existing* leading `&` is positioned.

## 7.2 Break-point semantics

Port the Python concepts, not the Python indentation: bracket depth profile; operator break tiers;
shallowest nesting preferred; looser-binding operator preferred over tighter; the minimum-fill
heuristic (`MINIMUM_BREAK_FILL = 0.25`); declaration `::` and assignment `=` statement-head
protection; named-argument exclusion from assignment breaks; whitespace fallback; never splitting
a multi-character operator.

Tiers, loosest first: comma; `.eqv.`/`.neqv.`; `.or.`; `.and.`; comparisons; `//`; additive;
multiplicative and `**`.

**Eligibility.** Decline to reflow when a character literal continues across physical lines, when
interleaved CPP or full-line comments make reflow unsafe, when Hollerith makes tokenization
unsafe, or when no safe break exists. Retain the Python behavior of detaching a single final
inline comment above the statement before reflow. Every declined long line must be classifiable in
diagnostic output so the corpus check can separate "unwrappable by design" from "wrapper bug".

**OpenMP** needs its own wrapper: join eligible continued directives, preserve quoted clause
expressions, emit repeated `!$OMP` sentinels with valid `&` markers, compute available width
including the sentinel, and run macro protection before keyword uppercasing. Note that
`--openmp=0` in the CAMB profile disables findent's OpenMP *indentation* while directive *text*
normalization stays on — **these are two config concerns and must not share one flag.**

## 7.3 CLI

Stdin/stdout stays the default with no paths. Do not adopt Python's "no paths means the whole
repository"; that is a surprising compatibility break. Use explicit `--all`.

```sh
findent [options] < in.f90 > out.f90     # stream (default)
findent [options] file1.f90 file2.f90    # in-place
findent --stdout [options] file.f90      # one file to stdout
findent --all [options]                  # repository
findent --check --all                    # exit 1 if anything would change
findent --diff file.f90                  # unified diff, exit 1 if changed
```

Retain every current findent option: `-i<n>`/`--indent=<n>`/`-i-`, `-I<n>`/`-Ia`/`--start-indent`,
`-M<n>`, `-k<n>`, `-K`, `--align-paren[=<n>]`, label/include positioning, contains
restart/indent, per-construct indents, OpenMP indentation toggle, `-Rr`/`-RR`, `--ws-remred`,
`-lastindent`/`-lastusable`, free-form aliases, and explicit rejection of fixed-form requests.

Add: positional paths, `--all`, `--stdin`, `--stdout`, `--isolated`, `--check`, `--diff`,
`--wrap`/`--no-wrap`, `--uppercase-single-l`, `-D`/`--define NAME[=VALUE]` (repeatable),
`--line-length=<n>` (default 120), `--indent-only`.

**Exit status:** `0` success or clean check; `1` check/diff found changes, or a runtime I/O
failure; `2` invalid usage, unknown option, or path validation failure.

**Queries.** `-lastindent`/`-lastusable` operate on stream indentation analysis and suppress
formatted output. Reject them clearly when combined with path-update, check or diff modes. Full
normalization must not change their structural meaning.

## 7.4 Config and distribution

Defaults remain findent 4.3.7's. CAMB passes its profile as explicit arguments exactly as
`findent_fortran.py` does today, so nothing has to change on day one.

Project configuration is supported with precedence **CLI > config file > formatter defaults**.
The nearest `.forformat.toml` is used for general repositories; `.findent.toml` is accepted as a
compatibility spelling. Python projects can put the same keys in `[tool.forformat]` in
`pyproject.toml`. The config keys mirror the long CLI option names, with booleans and arrays where
that is more natural (`align-paren = true`, `defines = ["NAME"]`). `--config PATH` selects a file
explicitly and `--no-config` disables discovery. File-selection and reporting controls remain
CLI-only.

Distribution is a Python wheel wrapping prebuilt binaries, exposing console entry points that
replace both hook scripts. CI already builds reproducible static musl and Windows MSVC artifacts,
so the wheel-building step consumes existing outputs. Evaluate pre-commit's native
`language: rust` hook support first — if it is sufficient, it is much less machinery.

## 7.5 Public API

Keep `format_source` / `format_to` / `format_to_owned` working; add fields compatibly rather than
breaking `FormatConfig::default()` before the Phase 12 cutover. A single `&[u8]` cannot express
project context, so add that separately:

```rust
pub fn analyze_project<'a, I>(sources: I) -> Result<ProjectContext, FormatError>
where I: IntoIterator<Item = (&'a Path, &'a [u8])>;

pub fn format_source_with_context(
    source: &[u8], context: &ProjectContext, config: &FormatConfig,
) -> Result<FormatResult, FormatError>;
```

The existing single-buffer entry points remain valid as `ProjectContext::empty()`, because the
entire golden, property and fuzz suite calls them.

---

# 8. Test strategy

## 8.1 Traceability table

One table replaces the three checklists the previous draft carried. Maintain it in
`docs/traceability.md`, seeded with all 86 rows from `test_standardize_fortran.py`:

| Python behavior | Category | Rust destination | Named Rust test | Status |
|---|---|---|---|---|

Categories: lexical, case, scope/project-case, OpenMP, CPP/macro, comment, continuation, wrapping,
blank-line/layout, CLI/file-I/O, semantic-compile. Statuses: `ported`, `covered by broader test`,
`intentionally changed` (with rationale and a fixture), `not applicable`.

Golden cases go in `tests/manifests/python_formatter.manifest` using the existing manifest format
and its `source_test` / `oracle` / `category` / `support` / `normalization` metadata. Do not invent
a second mechanism.

## 8.2 Layers

1. **Existing Rust suite, unchanged**, pinned to `mode=indent-only` (I6): unit tests, manifest
   cases, `tools/differential_free.sh`, arbitrary-byte and prefix properties, fuzz targets.
2. **Ported Python regressions** — §8.1.
3. **Convergence differentials.** For each fixture: reference the old iterative fixed point; run
   Rust full once and assert equality; run it twice and assert byte equality (I1); run indent-only
   on the output and assert no change (I2); where feasible run frozen `P` and assert no change.
   This suite is the direct proof of the project goal.
4. **Corpus check** — `tools/check_camb_corpus.sh`, developer-run, not in `cargo test` (§3.3).
   Necessary but *not sufficient*: the corpus is already a joint fixed point, so it can only
   detect harm, never absence.
5. **Perturbation differential** — `tools/reference/differential.py`, developer-run. Moves each
   corpus file off the fixed point with a targeted, protected-span-aware perturbation, then
   compares Rust `--full` against `P(R(perturbed))`. Each perturbation isolates one class of rule,
   which is what makes a difference diagnosable rather than merely alarming:

   | Perturbation | What it exercises |
   |---|---|
   | `spacing` | operator and comma spacing (A10, A11) |
   | `operators` | legacy relational operators (A12) |
   | `compound` | run-together keywords (A1, A2) |
   | `exponent` | real exponent markers (A13) |
   | `mixed` | all of the above at once — where rule *interactions* surface |
   | `keywords` | keyword/intrinsic lowering only (A16), holding declared names fixed |
   | `case` | every identifier uppercased — dominated by project case application (Chunk C) |

   Use `keywords` rather than `case` to judge the per-line rules: `case` also rewrites declared
   names, so its differences are mostly whatever the declaration engine cannot see yet.
6. **Generated wrapping stress** — §8.4.
7. **Semantic validation** — `gfortran -ffree-form -ffree-line-length-none -fopenmp
   -fsyntax-only` on self-contained fixtures before and after formatting; the project's own build
   for the real corpus.
8. **Properties** — §8.3.
9. **Fuzzing** — extend the existing workspace with region-walker, declaration-analyzer,
   project-resolver and wrapper targets, plus the I1/I2 invariant:
   ```rust
   let once = full_format(input)?;
   assert_eq!(full_format(&once)?, once);      // I1
   assert_eq!(indent_only(&once)?, once);      // I2
   ```
   Require totality and stable protected-byte handling, not semantic validity of arbitrary bytes.

## 8.3 Full-mode properties

Add, alongside the indent-only properties they replace: I1 for arbitrary and truncated input; I2;
string literals byte-identical; comments byte-identical except the documented transforms; CPP
bodies byte-identical; random leading indentation on structurally valid fixtures yields the same
full output; safe case and space perturbations of keywords and operators converge to the same
result; formatting never creates an out-of-bounds span when reparsed.

Gate `tests/properties.rs:29`, `:38`, `:94` and `:107` on indent-only mode (§3.2.6).

## 8.4 Generated wrapping stress corpus

The real corpus has few over-long lines (§3.3), so generate from real statements: join an existing
continued call onto one long line; strip continuation breaks from declarations; widen expressions
with repeated operands; perturb leading indentation. Vary `--line-length` (60/80/100/120), default
continuation vs `--align-paren`, `-k0`/`-k3`/`-k9`/`-K`. Include labels, named arguments, nested
calls, array constructors and OpenMP directives.

Properties: I1; I2; and no wrapper-created line exceeds the limit where a safe break exists.

## 8.5 Newline policy

**Indent-only** keeps today's contract: byte-oriented, per-line LF/CRLF/mixed terminators
preserved.

**Full mode** initially adopts the Python contract: choose the dominant input line ending and
normalize output lines to it. This is not hypothetical — `CAMB/forutils/` is CRLF throughout and
is a fixed point today, so the rule has concrete test material on both sides. Test LF, CRLF,
mixed, a single unterminated line, and non-UTF-8 bytes in comments and strings.

If per-line preservation is later judged the better full-mode contract, treat it as an intentional
divergence and update the oracle expectations and documentation rather than letting it drift.

---

# 9. Known divergences and fixes

The goal is "equivalent **plus fixes**", so defects are listed rather than silently inherited.

## 9.1 `_validated_fortran_path` — extension vs existence

`standardize_fortran.py:4098-4109` validates the extension, then existence.
`test_standard_free_form_extensions_are_accepted`
(`test_standardize_fortran.py:1850`) expects a valid extension on a *non-existent* path to be
accepted, and fails today with 8 subtest errors — the pre-existing 1-test failure in the 86-test
baseline. The sibling test `test_invalid_extension_is_rejected_before_reading` confirms the
intent: extension validation is meant to be independent of opening the file.

**Fix:** separate the two checks in Rust. `validate_extension(path)` is pure; opening is a distinct
step. Both failures exit 2, with distinct messages.

## 9.2 `--ws_remred` on a valid literal

findent 4.3.7 treats a single-quoted literal following an alphanumeric token plus whitespace as
non-string code, so its `remred` heuristic collapses spaces inside a valid Fortran character
literal (for example `error stop '...  ...'`). Rust preserves the literal, following the option's
documented "outside strings" contract. Recorded in `compatibility.md` and the manifest; carry it forward as an
intentional, fixture-backed divergence and do not regress to match.

## 9.3 forutils inclusion asymmetry

pre-commit excludes `^forutils/` from rewriting; `standardize --all` rewrites it. Both read it for
case resolution. Decide explicitly which the Rust tool does and document it (§2.3).

## 9.4 Recording new divergences

Use the manifest's existing `support` / `normalization` / `oracle` fields, with the oracle command
and version recorded. Never change an expected output without that provenance and a reviewed
reason.

---

# 10. Acceptance gates

Each gate references the invariants of §1.1 rather than restating them.

| Gate | Criterion |
|---|---|
| **A — findent compatibility** | `cargo test`, strict clippy and `cargo fmt --check` pass. The manifest suite passes in indent-only mode. `tools/differential_free.sh` passes for supported cases. No regression in arbitrary-byte or prefix handling. (I6) |
| **B — Python coverage** | Every one of the 86 Python tests has a traceability row with a terminal status. Project declaration and case behavior matches the reference fixtures. (I4) |
| **C — Convergence** | Every supported convergence fixture has a known stable result that Rust reaches in one pass. No unclassified old-tool cycles. (I1, I2) |
| **D — Real corpus** | `tools/check_camb_corpus.sh` reports zero differing files across `CAMB/fortran/` + `CAMB/forutils/` with shared project context, and every long line is either wrapped or explicitly classified as unwrappable. (I1, I2, I4) |
| **E — Semantic safety** | Self-contained formatted fixtures pass `gfortran -fsyntax-only`; OpenMP fixtures pass with `-fopenmp`; the formatted CAMB tree builds and its tests pass. (I3, I5) |
| **F — CLI replacement** | Stream, one-file, multi-file and repository modes all work; check/diff are suitable for CI and pre-commit; `-D` and project casing work in a real build; help text documents mode interactions. |
| **G — Performance** | Project sources are read and analyzed once per invocation. The runtime status report compares project and startup performance; single-buffer formatting remains suitable for format-on-save. No production dependency on Python. |

Gate G's number is the actual motivation for the port: pre-commit and editor latency. Treat it as
a requirement, not a nice-to-have.
