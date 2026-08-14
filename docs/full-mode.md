# Full-mode reference

This document describes the full formatter and the checks used to maintain it: normalization,
declaration-driven case handling, wrapping, and file/project workflow. [`design.md`](design.md)
says why the pipeline has this shape; [`compatibility.md`](compatibility.md) owns the indent-only
findent boundary.

## 0. How to work in this repository

The normal validation loop is:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo build --release && ./tools/check_camb_corpus.sh
python3 tools/reference/differential.py --show 0 CAMB/fortran/*.f90 CAMB/forutils/*.f90
python3 tools/check_invariants.py --oracle
```

The corpus check alone cannot establish that a rule works. CAMB is already a fixed point of the
formatter and the reference, so a correct rule and an inert rule can both report 0 differences.
The check detects harm; it cannot detect absence.

The differential harness perturbs each file off that fixed point—only code bytes, never string,
comment, Hollerith, or CPP bytes—and compares Rust `--full` against the reference pipeline. Use
the perturbation that isolates the rule under test:

| Perturbation | Exercises |
|---|---|
| `spacing` | operator and comma spacing |
| `operators` | legacy relational operators |
| `compound` | run-together keywords |
| `exponent` | real exponent markers |
| `mixed` | interactions among these rules |
| `keywords` | keyword/intrinsic lowering while holding declared names fixed |
| `case` | identifier case application after uppercasing every identifier |
| `separators` | whitespace around every depth-0 `::` |
| `blanks` | reinsertion of deleted blank lines |
| `blankruns` | spacing and blank-line capping |

The last three are whole-text perturbations (`TEXT_PERTURBATIONS`). They exercise post-layout
passes that a fixed-point corpus cannot expose. Their target is no disagreement because they do
not touch identifiers.

Use `keywords`, rather than `case`, to judge per-line rules: `case` also rewrites declared names.
`format_text` must receive declaration case tables as arguments. The differential reference path
mirrors the `--stdin` branch of `standardize_fortran.main`; calling `format_text` bare applies
almost no declared casing and is not a valid comparison.

Comparison counts are runtime status, not a design constant. The differential script prints the
live file, line, and classification totals for the supplied corpus. The count measures
disagreement rather than correctness; declarations and source adjudicate deliberate divergences.

`check_restoration.py` is useful for finding disagreements but cannot decide a case where a
perturbation changes a declaration without changing its uses. A committed spelling is not evidence
against the governing declaration. Every perturbation must preserve I1
(`f(f(x)) == f(x)`) and I2 (`indent_only(full(x)) == full(x)`). For each accepted case-only
difference, assert:

```text
a.lower() == b.lower()
```

and, for `keywords`, that an unperturbed continuation head still ends in `&`. The three pairs
common to every perturbation are deliberate: an unresolved `%` chain leaves the component alone.

### Rules that are not negotiable

1. **Do not edit `tools/reference/`.** It is the current comparison target; its hashes are recorded
   in `tools/reference/PROVENANCE.md`. If an expected output disagrees with it, the Rust side is
   wrong until the source and the rule prove otherwise.
2. **Do not change an existing expected output in `tests/expected/`.** Those are findent 4.3.7
   goldens and changing one silently breaks invariant I6. Add a fixture instead.
3. **Do not vendor CAMB sources.** They are a verification target, not test data. Reduce a corpus
   difference to a minimal snippet, add a fixture and manifest row, fix it, and rerun the checks.
4. A pass is either complete or inert. Half-applied normalization is worse than no normalization
   because full output is intended to be a fixed point.
5. Every new rule needs an idempotence test: `f(f(x)) == f(x)` on its fixtures.
6. Protected bytes never change (I3): string literal contents, comment text beyond the documented
   transforms, CPP directive bodies, Hollerith payloads, and non-UTF-8 bytes. Express edits as
   `EditBuffer` changes over token spans.

### Where things are

| Concern | Module |
|---|---|
| Protected regions, one quote scanner | `src/source/regions.rs` |
| Token stream with kinds and bracket depth | `src/source/tokens.rs` |
| Statement assembly with source provenance | `src/source/logical_statement.rs` |
| Structural layout decisions | `src/format/planner.rs` |
| Byte emission | `src/format/engine.rs`, `src/format/emitter.rs` |
| Break-point selection and reflow | `src/format/wrapping.rs` |
| Full-mode driver | `src/format/full.rs` |
| Mutable text and re-analysis | `src/transform/document.rs` |
| Span edits | `src/transform/edit.rs` |
| Generated word lists | `src/transform/vocab.rs` (`python3 tools/gen_vocab.py`) |
| Pass order | `src/transform/pipeline.rs` |
| Transformation passes | `src/transform/passes/` |
| Scopes and program units | `src/analysis/scope.rs` |
| Declaration extraction | `src/analysis/declarations.rs` |
| Case resolution (I4) | `src/analysis/names.rs` |
| Project context | `src/analysis/project.rs` |
| Perturbation differential | `tools/reference/differential.py` |
| I1/I2 over perturbed input | `tools/check_invariants.py` |

### Reading the reference

The reference is `tools/reference/standardize_fortran.py`. Each pass stub names the corresponding
function in its doc comment. To inspect one line:

```sh
cd CAMB && python3 -c "
import sys; sys.path.insert(0, 'scripts')
import standardize_fortran as S
print(repr(S.normalize_delimiter_spacing('x = a( 1 , 2 )')[0]))"
```

For a whole file, use:

```sh
python3 tools/reference/converge.py --project CAMB fortran/results.f90
```

## 1. Full-mode architecture

The full pipeline uses a shared protected-region walker, token stream, `LogicalStatement`
provenance, planner/emitter separation, `Document` re-analysis, and the real indentation engine.
It supports `IndentOnly`, `NormalizeOnly`, and `Full`, `WrapConfig`, macro definitions, `--full`,
`--normalize-only`, `--indent-only`, wrapping controls, line length, `-D/--define`, and
`--uppercase-single-l`. The manifest `mode` key pins the 53 existing cases to indent-only unless a
case opts into another mode. I2 is structural because full mode reuses the indentation engine.

### Per-line normalization

`src/transform/passes/line_rules.rs` applies span-local edits to tokenized code. It never rebuilds
a line from tokens. The rules are:

- expand compound keywords such as `endif` → `end if` and `blockdata` → `block data`, and convert
  `go to` → `goto` only at statement start and outside protected regions;
- normalize multiword keyword spacing and the individual spacing rules for `if (`, `dimension(`,
  `associate(`, `result(`, `type(`, `class(`, `select type (`, and parenthesized statements such
  as `write(` and `open(`;
- remove whitespace adjacent to `(`, `)`, `[` and `]`, normalize `) then`, and normalize `end x`,
  `do while (`, arithmetic-IF, one-line-IF, and `COMMON /blk/` forms;
- remove empty subroutine arguments (`subroutine s()` → `subroutine s`), modernize `(/ ... /)` to
  `[ ... ]` except in `FORMAT`, and normalize delimiter, comma, arithmetic, assignment, and
  logical-operator spacing;
- modernize legacy relational operators, lowercase genuine real-literal exponent markers, add
  `WRITE(...)item` spacing, and optionally uppercase a lone `l` with `--uppercase-single-l`;
- normalize only the documented comment marker and commented-assignment forms; this is the only
  transform that changes comment text;
- lowercase intrinsics and Fortran specifiers using context guards for `bind(c)`, `only:`,
  `kind(`, attributes, and identifiers after `::`; and
- normalize declaration attribute order and old-style declarations.

String literals, Hollerith data, CPP bodies, and non-UTF-8 bytes are protected. The rules include
tests with strings and comments and a full-mode property test for protected bytes and idempotence.

### Declaration engine

`src/analysis/declarations.rs` extracts names into scoped case maps. It handles old-style
declarations without `::` such as `real x, y(3)` and `integer*4 n`; procedure arguments and
`RESULT` clauses; `USE ... ONLY:` and rename clauses; full-form type-bound procedures, generic and
final bindings; `SELECT TYPE` aliases; `EXTERNAL`, `INTRINSIC`, `COMMON`, `NAMELIST`, and `ENTRY`
lists; interface signatures; component type chains; synthetic multi-file project cases; and
derived-type inheritance.

The `TypeMaps` chain resolves `a%b%c` through `local_types`, `variable_types`, and
`component_types`. Components are keyed by `(type_name, component_name)`, as in the reference,
because CAMB has two `tcmb` components and two `limber_windows` components in different types.
Inheritance walks the parent chain explicitly. The reference does not model inheritance; this
port does so because a name-only fallback would be unsafe with the type-qualified keys. Unknown
parents and cycles are guarded, nearest-level shadowing wins, and ambiguity remains silent.

The declaration index is represented by per-scope maps and a per-line ancestor list, not by copying
all visible names into every physical line. This keeps lookup proportional to the scopes traversed;
the measured benchmark is approximately 30 ms across 0/50/200/800 module declarations, compared
with 548 ms for the copied 800-declaration representation on a 4000-line file.

`local_names` and `file_declared_names` have different roles:

| | `local_names` | `file_declared_names` |
|---|---|---|
| Built from | the innermost enclosing procedure | every enclosing module/program/procedure |
| Holds | declarations before `contains`, dummy arguments, `RESULT`, `SELECT TYPE` aliases | scope-opening names, derived-type names, and module variables before `contains` |
| Excludes | — | procedure locals, components, type-bound procedures, and names inside `INTERFACE` |
| `KEYWORD=` arguments | suppresses keyword lowering | does not suppress keyword lowering |
| Scope | current file; project tables are not used for suppression | current file |

`CaseResolver::declared_filewide` is used only by identifier case application. A project declaration
in another file must not silence a keyword in the current file.

Numeric-literal kind suffixes are identifier occurrences. A governing `DL` declaration applies to
every continuation line, including `2.0_DL` and `3.0_DL`; numeric kinds such as `_8` and undeclared
names are inert. The reference can miss the final continuation line and can leave
`2.99792458e8_dl` unchanged on the declaration line; the consistent governing-declaration result
is intentional.

### Case application

`src/transform/passes/case_pass.rs` applies macro and declared-name spellings in their name spaces:
module names in `USE` and after `%` module qualifiers, type names after `TYPE(`/`CLASS(`,
resolved components after `%`, type-bound procedure names, plain symbols, and kind suffixes.
Procedure-local spellings outrank file and project tables. An unresolved `%` chain remains authored;
it must not fall through to keyword lowering. Ambiguous local or project cases are silent.

### Structure, wrapping, and post-layout

The structure passes rejoin lexical token continuations, remove redundant nested parentheses only in
RHS, `IF`, and `DO WHILE` conditions, and remove only a final single-line bare `RETURN` before a
procedure `END`; procedure arguments, `ASSOCIATE` targets, and returns with inline comments remain.

Reflow joins an already-continued statement, reapplies line rules, detaches a final inline comment
before wrapping, and uses `ParenAlignmentState` for `--align-paren`. OpenMP wrapping repeats the
`!$OMP` sentinel, keeps `&` markers valid, subtracts the sentinel from available width, and
protects macros before keyword case handling. Decline diagnostics classify an unwrappable line.
Generated stress cases cover line lengths 60/80/100/120, default and aligned parentheses, and
`-k0/-k3/-k9/-K`; the properties are I1, I2, and no wrapper-created line exceeding the limit where
a safe break exists.

`layout_post.rs` contains declaration-separator alignment, program-unit spacing, blank-line limits,
and output-whitespace cleanup. Alignment compresses authored padding, but a compact `::` gains its
owed space on either side and can grow by two columns. Wrapping measures the step-17 spelling and
layout repeats when its width changes. A detached comment is emitted at the statement indent, and
`copy_group_without_final_comment` preserves other comments in the group.

### File and project workflow

`src/io/` handles repository-root and tracked-source discovery using `vocab::SOURCE_EXTENSIONS` and
uppercase spellings. Nested Git calls clear `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, and
`GIT_INDEX_FILE`. `validate_extension(path)` is pure and runs before opening a file; extension and
existence failures both exit 2 with distinct messages.

The workflow supports positional paths, `--all`, `--stdout`, `--isolated`, atomic in-place writes
that preserve mode bits and resolve symlinks, `--check`, and unified `--diff` with repository-
relative paths and exit statuses 0/1/2. `-lastindent` and `-lastusable` cannot be combined with
path-update, check, or diff modes. Project sources are read and analyzed once per invocation.
Pre-commit excludes `forutils/` from rewriting; `standardize --all` rewrites it, and both read it
for case resolution.

## 2. Design decisions and checks

### Governing declarations and the project fixed point

Resolve a use to the declaration that governs it. Omit case application only when that declaration
cannot be determined. Two locals with different spellings are separate entities when their scopes
resolve them separately. A type-bound binding and its module procedure are one entity. Components
with the same name are ambiguous when the owner type cannot be resolved; `TypeMaps::resolve_chain`
is consulted before any fallback.

The current developer checkout under `CAMB/fortran` and `CAMB/forutils` is treated as a project
fixed point. `tools/check_project_mode.py` copies whatever files are present, requires one
unperturbed project-mode run to leave them byte-identical, and independently compares that result
with the frozen reference. It records no correction allowance and no corpus revision.

### Deliberate reference divergences

- Multiline `(/ ... /)` constructors are rewritten as complete valid `[ ... ]` constructors. The
  reference can leave a later closing `/)` and produce `[ ... /)` in `DarkAge21cm.f90`, `bessels.f90`,
  and `massive_neutrinos.f90`; Rust matches the valid output committed by CAMB.
- Comment bodies are changed only when a narrow recognizer proves a simple identifier/member (or a
  single non-nested parenthesized subscript) followed by `=`. Prose, URLs, tables, directives,
  banners, markers, and uncertain expressions remain unchanged. The reference respaces seven such
  nested or non-Fortran expressions; those seven `comment-content` pairs are accepted.
- A kind suffix follows its governing declaration on continuation lines, including an exponent
  literal, even where the reference misses that application.
- Conditional `!$` sentinels retain authored boundary spacing while their Fortran-like body is
  normalized, including declaration-driven identifier case.
- `--ws_remred` preserves valid literal bytes. The reference can treat the quote after `error stop`
  as code and reduce spaces inside that literal.

The focused governing-declaration cases for owner-keyed type-bound bindings, old-style/typed local
entities, and top-level program-unit parameters agree byte-for-byte with the current reference.
The bare-program BJL validation file keeps `BJL_RECURRENCE_MAX_L` against the unrelated module's
`BJL_recurrence_MAX_L`.

### Standing checks and what each one proves

| Check | Claim |
|---|---|
| `differential.py --perturbation none` | unperturbed stdin output equals the reference |
| `differential.py` (ten sweeps) | perturbed input reaches the reference result, with deliberate differences adjudicated |
| `check_project_mode.py` | the current external project corpus is a fixed point and matches the reference |
| `check_route_equivalence.py` | identical bytes give identical output on every route |
| `check_invariants.py --oracle` | I1 and I2 under every perturbation |
| `check_historic_corpus.py` | hand-written Fortran is checked on every comparison axis |
| `adjudicate_case.py` | disputed spellings are checked against declarations in the source |
| `check_restoration.py` | restoration differences are measured after perturbation; it does not decide declaration cases |
| `check_fixture_syntax.sh` | formatting does not turn a compiling fixture into a non-compiling fixture |
| `check_camb_corpus.sh` | idempotence and line width; its `differing` column is a fixed-point signal, not a correctness claim |

Validation totals are deliberately not copied into this design reference. The fixture, corpus,
differential, route-equivalence, test, and release scripts print their live summaries when run;
CI logs are the status report for checks that do not require the external CAMB checkout.

## Known traps

These are durable rules for maintaining the formatter.

- **The reference is not the arbiter; the source is.** A difference is a defect only when the
  correct answer is settled by the code being formatted, especially a governing declaration.
  Use `tools/adjudicate_case.py` before classifying a case difference.
- **An inert branch is not automatically safe.** `return None` can leave authored bytes on an
  ordinary line while reflow emits a lowercased identifier; a component path can do the same, and
  an unresolved cross-file `%` member can fall through to keyword lowering. Test every "leave it
  alone" path, including `%Value`, `%Write`, and `%Init`.
- **Never run in-place mode with this repository as the working directory.** Use a throwaway
  checkout for `--all`, as in `check_historic_corpus.py::rust_project`, and inspect
  `git status --short tests/` afterwards. An already-indented fixture can be a fixed point even
  after its input has been overwritten.
- **A check that sees only a fixed point cannot prove a rule is live.** The corpus, route,
  project, and ordinary differential checks need perturbations or an unperturbed comparison that
  can fail on a planted defect.
- **Do not make a metric structurally unable to report a defect.** Normalizing before comparison,
  name skip-lists, `%` lookbehinds, broad case-pair suppression, and an unreachable `scoped_ours`
  branch can all manufacture zeroes. When a check reports 0, read its diff and test it against a
  known defect.
- **A governing declaration must be named before a case difference is classified.** For example,
  `yout(EVout%nvar)` in `CopyScalarVariableArray` is governed by the `EVOut` dummy at
  `equations.f90:702-703`; a committed spelling in another scope is irrelevant.
- **Case-only differences are still differences.** Project mode has no correction allowance: the
  unperturbed external corpus must remain byte-identical and agree with the reference.
- **A build or install result must be observed.** A predicted wheel transcript is not evidence;
  verify that the wheel exists, that installation ran, and that the executable actually used by
  the hook is the intended one.
- **`language: system` resolves through `PATH`.** The real findent accepts `--full`, exits 0, and
  leaves the file unchanged. The hook uses `language: python` with `additional_dependencies` so
  the `forformat` console script runs inside pre-commit's environment.
- **A traceability status must be a row-specific classification.** A blanket sentence such as
  "no dedicated release contract" does not establish that a row is excluded or covered.
- **A named test must contain the row's discriminating assertion.** Negative assertions need
  negative fixtures; a test name in the same area is not coverage.
- **End-to-end assertions belong at the end of the pipeline.** A pass-level test cannot pin a
  result produced by a later pass. Compare with the reference's output, not an intermediate stage.
- **Read the diff behind a metric.** The BJL correction belongs in `src/analysis/declarations.rs`
  and is covered by `analysis::project::tests::program_top_level_spelling_still_wins_over_a_module`;
  a compiled compatibility table is not a project-scope fix.
- **A rule must be justified by Fortran and source semantics, not by a perturbation.** The
  uppercase-exponent kind-suffix rule is governed by the declaration even when the perturbation
  changes only the declaration.
- **`check_restoration.py` cannot adjudicate a declaration/use mismatch.** If `integer, parameter
  :: dl` is uppercased while `1.0_dl` is not, the correct output is `1.0_DL`, even if the committed
  tree makes the other tool score better.
- **A perturbation that round-trips through the formatter tests nothing.** To test compound
  keywords, `else if` → `elseif` must also close the spacing gap (`elseif(`); the incomplete
  perturbation leaves the output unchanged.
- **CLI mode matching is order-sensitive.** The `--indent-only` arm must precede generic
  `--indent-<construct>` matching; a test covers this.
- **A keyword is not always a keyword.** `BIND(C, name=...)` in `CAMB/fortran/hyrec.f90` is not
  the language binding; `precision` needs a preceding `double`, `only` needs a following `:`, and
  an attribute needs a later `::`.
- **Complete declaration extraction before widening case rules.** Otherwise a component such as
  `Source` in `results.f90` can be rewritten as a keyword.
- **An over-registering extractor is worse than a missing one.** `CLASS(*), INTENT(IN) :: x` must
  not register `INTENT` as a type, and `DOUBLE PRECISION FUNCTION G()` must not register
  `FUNCTION` as a symbol. Assert what an extractor must not insert.
- **Span edits need left-to-right context.** Independent edits can produce `a =  .not. b` or drop
  the second operator in `.AND. .NOT.`. `OperatorSpacing` carries the needed state.
- **Not every case rule lives in the case pass.** `ONLY :` → `only:` is produced by the spacing pass
  in the reference; port the place where a behavior occurs, not only its final text.
- **Generated files must survive `cargo fmt`.** `gen_vocab.py` pipes through rustfmt before writing
  or comparing because raw one-item-per-line output is collapsed by rustfmt.
- **Stale cargo fingerprints can mimic source errors.** If a test reports a symbol that plainly
  exists, run `cargo clean -p forformat` before diagnosing the source.
- **Do not change valid literals under `--ws_remred`.** Rust preserves spaces inside
  `error stop '...  ...'`, an intentional divergence from findent 4.3.7.
- **Conditional `!$` sentinel bodies use declaration case application.** CAMB has 22 non-`OMP` `!$`
  lines in `MathUtils`, `cmbmain`, `halofit`, `lensing`, `results`, and `MpiUtils`. For example:

  ```text
  integer :: MyVar          !$ myvar = 1   becomes MyVar
  ...                       myvar   = 2    becomes MyVar
  ```

  The sentinel boundary remains protected, but the body follows the same declaration resolver as
  ordinary source, with or without project context. `!$OMP` directives retain their separate
  OpenMP keyword-only handling. The count is 22 source-line sites, not 22 expected corpus diffs;
  the current CAMB spellings are already fixed points.
- **Initializer scans reset per declaration entity.** In `integer :: A = 1, SIZE`, the `= 1`
  disqualifies neither `SIZE` nor its uppercase spelling; scanning from `::` for the whole statement
  would be wrong. `is_contextual_identifier` at `standardize_fortran.py:1914` resets at each
  top-level comma.
- **A fallback is safe only when its key carries the same information.** The reference's name-only
  fallback relies on a `(type, component)` table. A name-only port would incorrectly rewrite
  `CP%TCMB` to `CP%Tcmb` and `SourceTerms%limber_windows` to `Limber_windows`. An unresolved chain
  remains inert.
- **The corpus is a harm detector, not a score.** A lower differential count can coincide with
  corrupting real source; reaching 0 is a precondition, not a reason to accept a rule.
- **The reference and Rust have different convergence contracts.** Rust converges in one pass;
  the reference's declaration tables can change on a second pass. `check_invariants.py` covers I1
  and I2 over 480 perturbed inputs, and the `separators` sweep is measured with `--converge` when
  comparing destinations.
- **A per-line copied view is quadratic.** Both implementations key names by physical line, but
  materializing every visible name per line is O(lines × names); use per-scope maps and a per-line
  ancestor list.
