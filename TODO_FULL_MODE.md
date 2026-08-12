# Full-mode work queue

Work queue for the combined formatter described in `FORTRAN_COMBINED_RUST_PORT_PLAN.md`.
`TODO.md` remains the queue for indent-only findent 4.3.7 compatibility; this file owns
normalization, project casing, wrapping and the file/project workflow.

Last updated 2026-08-11, after Chunks A, B, C, D and E landed and were differentially validated.

---

## 0. How to work in this repository

Read this once before picking up a task.

**The loop for every task.**

```sh
cargo test                                  # must stay green
cargo clippy --all-targets -- -D warnings   # must stay clean
cargo fmt                                   # must stay formatted
cargo build --release && ./tools/check_camb_corpus.sh   # differing is explained; must report 0 non-idempotent
python3 tools/reference/differential.py --show 0 CAMB/fortran/*.f90 CAMB/forutils/*.f90
python3 tools/check_invariants.py --oracle        # I1/I2 over perturbed input, not only pristine
```

**The corpus check alone cannot tell you your rule works.** CAMB is already a joint fixed point of
findent and the Python formatter, so a correct rule and a rule that does nothing at all both score
0 differing. It detects harm; it cannot detect absence.

The differential harness is what proves a rule *right*. It perturbs each file off the fixed point —
only code bytes, never string, comment, Hollerith or CPP bytes — and compares Rust `--full` against
`P(R(perturbed))`, the reference pipeline. Pick the perturbation that isolates your rule:

| Perturbation | Exercises |
|---|---|
| `spacing` | operator and comma spacing |
| `operators` | legacy relational operators |
| `compound` | run-together keywords |
| `exponent` | real exponent markers |
| `mixed` | all of the above — where rule *interactions* surface |
| `keywords` | keyword/intrinsic lowering, holding declared names fixed |
| `case` | every identifier uppercased — dominated by project case application |
| `separators` | whitespace around every depth-0 `::` — declaration alignment |
| `blanks` | every blank line deleted — program-unit spacing must re-insert |
| `blankruns` | three-blank-line runs injected — spacing and blank-line capping |

The last three are whole-text perturbations (`TEXT_PERTURBATIONS`), added because Chunk E was
invisible to every other check: CAMB is already a fixed point of the post-layout passes, so the
corpus scored 0 with all three stubbed out, and nothing else disturbs `::` alignment or blank-line
structure. Stubbed, they measured 47 files / 11427 lines, 48 / 1323 and 48 / 9263. **They touch no
identifier, so unlike `case` and `keywords` their target is a literal 0, not "0 more-than-case".**

Judge per-line rules with `keywords`, not `case`: `case` also rewrites declared names, so its
differences are mostly missing declaration extractors rather than missing per-line rules.

**`format_text` takes the declaration case tables as *arguments*.** Calling it bare applies almost
no declared casing, which quietly turns the whole `case` column into a comparison against a crippled
reference. `differential.py::reference_format` now mirrors the `--stdin` branch of
`standardize_fortran.main` exactly. This was wrong for the entire Chunk A validation, and the
"0 differing across five perturbations" claim from that period was measured against a weaker oracle
than it appeared. If you touch the harness, check it against the reference's own CLI first.

Current state, 48 files under those globs, against the reference's historical single pass:

| Perturbation | Differing files | Differing lines | More than case |
|---|---|---|---|
| `case`, `--converge` | 6 | 72 | **0** |
| every other sweep, `--converge` | 2 | 10 | **0** |

The `case` sweep's remaining 6 files / 72 diff lines are 64 kind-suffix lines and 8
declaration-bound lines where the perturbation changes a declaration but not the nested use. Both
are us applying the governing declaration where the reference does not; they are adjudicated and
they stay. The two real defects this sweep found — program-unit locals and `procedure(...) ::`
declarations, both under G0 — are fixed. Every other
converged sweep remains at the 2-file / 10-line floor: the `max_Nu` and `EVOut` corrections.

**An oracle-difference count cannot adjudicate a deliberate divergence.** Since the
governing-declaration rule landed, the table above measures *whether* we differ, not who is right.
`tools/check_restoration.py` answers it *partially*, by comparing both tools to the committed CAMB
tree after a perturbation: the tool that restores more of the authors' own spelling resolved more
names correctly. Its decisive number is the per-line split, not the total. Under `keywords` that
split is ours by 112 lines and the margin is real. Under `case` it reads `only we get wrong 72`
against `only they get wrong 1` and **that is not a verdict** — every one of the 72 is a kind
suffix whose declaration the perturbation moved and whose use it did not, so the committed tree is
no longer the correct answer there. See B12 and the corresponding Known trap. The tool finds
disagreements; declarations settle them.

I1 (`f(f(x)) == f(x)`) and I2 (`indent_only(full(x)) == full(x)`) hold across all 48 files and all
480 perturbed inputs, with no exceptions. The 5 `separators` cases that used to fail I1 were fixed
when we took the one-pass decision recorded under Chunk G.

**The summary table is not the acceptance test — the last column is.** A perturbation-count of 0 is
unreachable for anything touching identifiers: the harness uppercases declared names that collide
with keywords (`data`, `status`, `unit`, `out`, `err`, `name` are all real CAMB variables), the
reference propagates the new spelling through declared-case application, and any name space we have
not modelled yet shows up as a difference. What must never happen is a difference that is *more than
case*, because that means a lowering, spacing or structural rule is wrong:

```sh
# for each perturbation: extract every differing line pair and assert
#   a.lower() == b.lower()            # nothing but spelling differs
# plus, for `keywords`: a.rstrip().endswith("&")   # only unperturbed continuation heads
```

Right now that check reports **0 more-than-case differences on all ten perturbations**. Re-run it
after any change to the case, declaration or line-rule engines; treat a non-zero result as a stop.

The 3 pairs common to every perturbation are deliberate: an unresolved `%` chain leaves the
component alone rather than guessing (see "Known traps").

**Rules that are not negotiable.**

1. **Never edit `tools/reference/`.** It is the frozen oracle, hashes recorded in
   `tools/reference/PROVENANCE.md`. If an expected output disagrees with it, the Rust side is
   wrong until proven otherwise in writing.
2. **Never change an existing expected output in `tests/expected/`.** Those are findent 4.3.7
   goldens; changing one silently breaks invariant I6. Add a new fixture instead.
3. **Never vendor CAMB sources.** They are a developer verification target, not test data. On a
   corpus difference: reduce to a minimal snippet, add a fixture plus a manifest row, fix, rerun.
4. **A pass is either right or inert.** If you cannot implement a rule completely, leave it
   returning the input unchanged. Half-applied normalization is worse than none, because full
   output is supposed to be a fixed point.
5. **Every new rule needs an idempotence test.** `f(f(x)) == f(x)` on your own fixtures. This is
   invariant I1 and it is the only one the pipeline shape does not give you for free.
6. **Protected bytes never change** (I3): string literal contents, comment text beyond the
   documented transforms, CPP directive bodies, Hollerith payloads, non-UTF-8 bytes. Express your
   rule as `EditBuffer` edits over token spans and this holds by construction.

**Where things are.**

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
| Generated word lists | `src/transform/vocab.rs` (regenerate: `python3 tools/gen_vocab.py`) |
| Pass order | `src/transform/pipeline.rs` |
| The passes themselves | `src/transform/passes/` |
| Scopes and program units | `src/analysis/scope.rs` |
| Declaration extraction | `src/analysis/declarations.rs` |
| Case resolution (I4) | `src/analysis/names.rs` |
| Project context | `src/analysis/project.rs` |
| Perturbation differential | `tools/reference/differential.py` |
| I1/I2 over perturbed input | `tools/check_invariants.py` |

**Reading the oracle.** The frozen Python is at `tools/reference/standardize_fortran.py`. Every
pass stub names the exact function to port in its doc comment. To see what the reference does with
a line:

```sh
cd CAMB && python3 -c "
import sys; sys.path.insert(0, 'scripts')
import standardize_fortran as S
print(repr(S.normalize_delimiter_spacing('x = a( 1 , 2 )')[0]))"
```

To check a whole file:

```sh
python3 tools/reference/converge.py --project CAMB fortran/results.f90
```

---

## 1. What already exists

Do not redo any of this.

- [x] Shared protected-region walker; `comment_start` and `split_statements` rebuilt on it; the
      duplicated quote scanners in `buffer.rs`, `scanner.rs` and the dead `paren_alignment` /
      `normalized_statement` / `findentfix::payload` removed.
- [x] Token stream with kinds, bracket depth, correct `10abc` and numeric-literal lexing.
- [x] `LogicalStatement` provenance: `LogicalGroup::source_of` maps joined text back to
      `(physical line, byte offset)`.
- [x] `FormatMode` (`IndentOnly` default, `NormalizeOnly`, `Full`), `WrapConfig`, `MacroDefine`,
      and the CLI flags `--full`, `--normalize-only`, `--indent-only`, `--wrap/--no-wrap`,
      `--line-length`, `-D/--define`, `--uppercase-single-l`.
- [x] Manifest `mode` key; all 53 existing cases pin `indent-only` by default.
- [x] Planner/emitter split: `Planner::plan` owns every structural decision, `emit_group` writes
      bytes, per-group `FormatConfig` clones gone, 10-argument emitter signature gone.
- [x] Full-mode driver with the §5.1 pass order, `Document`/`Analysis` re-derivation, and layout by
      the real findent engine — which is what makes **I2 structural rather than tested**.
- [x] Break-point selection: tiers, depth ranking, minimum fill, head protection, named-argument
      exclusion, declines with reasons.
- [x] Case resolution (I4) complete and tested; scope tree; `USE`/`#define`/`::` declaration
      extraction; project merge with ambiguity collapse.
- [x] Keyword lowercasing with the context-sensitive guards (`bind(c)`, `only:`, `kind(`,
      attributes without `::`).
- [x] Oracle: frozen Python with hashes, `converge.py`, `tests/reference/convergence-baseline.json`
      (34 fixtures, 0 cycles), `tools/check_camb_corpus.sh`, `docs/traceability.md` (86 rows).
- [x] Perturbation differential harness (`tools/reference/differential.py`), which is what turned
      "the corpus is unchanged" into an actual proof of correctness for Chunk A.
- [x] All 19 Chunk A rules. No perturbation produces a more-than-case difference against
      `P(R(x))` on any corpus file.
- [x] **B9**: scope-ranged declared names (`DeclaredNameIndex`, `scoped_declared_names`). Keyword
      and intrinsic lowering is now exact against the oracle.
- [x] **Chunk C**: macro and declared-case application, with procedure-local spellings outranking
      the file and project tables.
- [x] **Chunk B** extractors B1-B8 and B10, with components re-keyed by `(type, component)`. The
      `case` differential went from 6980 differing lines to 170, and no perturbation produces a
      more-than-case difference, including derived-type inheritance (B11).

---

## Chunk A — Per-line normalization rules — **complete**

Landed and differentially validated on 2026-08-11. Six defects were found *after* the rules were
first marked done, every one of them invisible to `check_camb_corpus.sh`; they are recorded under
"Known traps" so the same shapes get caught earlier next time. Reopen a task here only if the
`keywords` or `mixed` differential regresses.

Each task was one rule in
`src/transform/passes/line_rules.rs`, each has a named Python function to port, and each is
independent of the others.

**Pattern to follow** — look at `lowercase_line` in the same file:

```rust
pub fn normalize_x(line: &[u8], cx: &PassContext) -> Vec<u8> {
    let tokens = tokenize(line, &mut LexState::default());
    let mut edits = EditBuffer::new(line);
    // ... find sites in `tokens`, call edits.replace(span, bytes) ...
    edits.finish()
}
```

Never rebuild the line from tokens; only replace spans. Add tests in the same file using the
`normalized()` helper, and always include one case with a string literal and one with a comment.

- [x] **A1. Compound keywords.** `endif` → `end if`, `blockdata` → `block data`, etc., from
      `vocab::COMPOUND_KEYWORDS`. Only at statement start, only outside strings. Python:
      `COMPOUND_KEYWORD` substitution in `_normalize_keyword_spacing_code`.
      - [x] Also `go to` → `goto` (Python `GO_TO`).
      - [x] Test: `ENDIF`, `end  if`, a variable named `endif` (must not change), `! endif`.
- [x] **A2. Multiword keyword spacing.** Collapse the whitespace inside every pair in
      `vocab::MULTIWORD_KEYWORD_PAIRS` to one space, lowercased. Python:
      `MULTIWORD_KEYWORD_SPACING`.
- [x] **A3. `keyword(` spacing.** `if (`, `dimension(`, `associate(`, `result(`, `type(`,
      `class(`, `select type (`, and `vocab::PARENTHESIZED_STATEMENT_NAMES` (`write(`, `open(`, …).
      Each has its own space-or-no-space answer; read
      `_normalize_keyword_spacing_code` line by line and port the cases in order.
      - [x] Test each keyword in the list, plus a same-named variable followed by `(`.
- [x] **A4. Bracket-adjacent whitespace.** `( x` → `(x`, `x )` → `x)`, `[ ` and ` ]` likewise.
      `) then` gets exactly one space. Python: the `([` / `)]` regexes and the `) then` rule.
- [x] **A5. `end x` spacing and `do while (`.** Python: the `end` and `do` blocks.
- [x] **A6. Arithmetic-IF and one-line-IF body separator.** A space between the closing condition
      paren and the statement that follows. Python: the `IF_STATEMENT_START` depth walk at the end
      of `_normalize_keyword_spacing_code`.
- [x] **A7. `COMMON /blk/` normalization.** Python: `COMMON_BLOCK_PREFIX`.
- [x] **A8. Empty subroutine arguments.** `subroutine s()` → `subroutine s`. Python:
      `EMPTY_SUBROUTINE_ARGUMENTS`. Note this *shortens* the line, so it is safe anywhere.
- [x] **A9. `(/ ... /)` → `[ ... ]`.** Never inside a `FORMAT` statement, where `/` is an edit
      descriptor with completely different meaning. Python:
      `_modernize_array_constructor_delimiters`.
      - [x] Test: a `FORMAT` statement containing `(/` must be untouched.
- [x] **A10. Delimiter and comma spacing** (`normalize_delimiter_spacing`). One space after a
      comma, none before; compact `*`, `/`, `**`, `//` per `vocab::COMPACT_ARITHMETIC_OPERATORS`.
- [x] **A11. Operator spacing** (part of `lowercase_line`): spaced binary `+`/`-` versus unary
      sign, `=`/`=>`/comparisons, `.and.`/`.or.`/`.not.` padding. Python:
      `append_normalized_operator` and `is_binary_arithmetic_operator` — port the *whole* of the
      latter, including the `1.-x` versus `x.lt.-1` distinction and the `1e-3` exception.
- [x] **A12. Legacy relational operators.** `.eq.` → `==` etc. from `vocab::MODERN_OPERATOR`.
      Python: `LEGACY_OPERATOR`.
- [x] **A13. Real exponent markers.** `1.0E5` → `1.0e5`, `D` likewise, only at a genuine literal.
      Python: `REAL_LITERAL_EXPONENT` and `is_real_literal_exponent_marker`. The tokenizer already
      identifies numeric literals, so this is a `TokenKind::Number` walk.
- [x] **A14. `WRITE(...)item` spacing** (`normalize_write_output_spacing`).
- [x] **A15. Comment spacing** (`normalize_comment_spacing`) and commented-out assignment
      operators (`format_comment_operators`, `COMMENTED_ASSIGNMENT`). This is the **only** transform
      permitted to change comment text; keep it exactly as narrow as the Python guard.
- [x] **A16. Intrinsics and specifiers in `lowercase_line`.** Extend the keyword rule to
      `vocab::INTRINSIC_NAMES` and `vocab::FORTRAN_SPECIFIERS`, with the guards already sketched in
      `keyword_in_context` plus `is_specifier_keyword_argument` and the "identifier after `::`"
      rule. **Do this after Chunk B**, since it depends on the declared-name tables being complete.
      - [x] Expect corpus churn; re-run `tools/check_camb_corpus.sh` and reduce every difference.
- [x] **A17. `--uppercase-single-l`.** A lone `l` used as a name becomes `L`. Python: the
      `uppercase_single_l` branch of `lowercase_keyword`.
- [x] **A18. Declaration attribute order and old-style declarations.** Python:
      `DECLARATION_ATTRIBUTES` ordering and `OLD_STYLE_DECLARATION`.
- [x] **A19. Property test for the chunk.** In `tests/properties.rs`, gated on full mode: string
      literals byte-identical, comments byte-identical except the A15 transforms, CPP bodies
      byte-identical, and `f(f(x)) == f(x)` over every fixture.

---

## Chunk B — Declaration engine — **complete**

`src/analysis/declarations.rs`. The framework, the scope tree and the resolution rules are done;
these tasks add extractors. Each one is "recognize a statement form, insert names into the right
`CaseMap`". Follow `entity_declaration` as the template, and add a test to the same file.

**B9 landed on 2026-08-11 and it was the load-bearing one.** `keywords` went from 15 files / 578
lines to 8 / 60, and every remaining line is a Chunk C case-application difference rather than a
lowering difference (see §0). The scope model the rest of this chunk hangs off now exists:
`DeclaredNameIndex` in `declarations.rs`, built by `scoped_declared_names(analysis, scopes)`.

What that model is, because the two name sets are **not** the same set at different ranges:

| | `local_names` | `file_declared_names` |
|---|---|---|
| Built from | the innermost enclosing *procedure* | the union of **every** enclosing module/program/procedure |
| Holds | declarations before that procedure's `contains`, dummy arguments, the `RESULT` name, `SELECT TYPE` aliases | scope-opening names attributed to the *enclosing* scope, derived-type names, and module variables declared before the module's `contains` |
| Excludes | — | procedure locals, components, type-bound procedures, anything inside an `INTERFACE` |
| On a `KEYWORD=` specifier argument | still suppresses | **does not** suppress |
| Scope of the file | current file only — the project tables are never consulted for suppression | same |

That last row was a second bug the original B9 note missed: `declared_anywhere` consulted the
*project* case tables, so a declaration in any project file could silence a keyword in this one.
The reference never does that. The file-wide predicate survives only as
`CaseResolver::declared_filewide`, used by the Chunk C identifier path and nothing else.

- [x] **B1. Old-style declarations without `::`.** `real x, y(3)`, `integer*4 n`. Python:
      `OLD_STYLE_DECLARATION`.
- [x] **B2. Procedure arguments and `RESULT` clauses.** From the procedure header, which
      `ScopeTree` already gives you the line of. Python: `SCOPE_HEADER`, `RESULT_CLAUSE`.
- [x] **B3. `USE ... ONLY:` lists and rename clauses** (`local => remote`), into `symbols`.
- [x] **B4. Type-bound procedures in full form:** `procedure(iface), pass :: run => run_impl`,
      `generic :: op(+) => add`, `final :: cleanup`.
- [x] **B5. `SELECT TYPE` aliases.** Python: `SELECT_TYPE_ALIAS`. The alias takes the type of its
      selector, so this also feeds `TypeMaps`.
- [x] **B6. `EXTERNAL` / `INTRINSIC` / `COMMON` / `NAMELIST` / `ENTRY` name lists.**
- [x] **B7. Interface bodies.** Names declared inside an `INTERFACE` are signatures;
      `ScopeTree::in_interface` already identifies them. Decide and document whether they
      contribute to `symbols` (check the Python: they do, via `extract_procedure_cases`).
- [x] **B8. Component type chains.** *Now the highest-value item in this chunk.* Extend `TypeMaps`
      population so `a%b%c` resolves through `local_types` → `variable_types` → `component_types`,
      matching the Python chain. `TypeMaps::resolve_chain` already exists and is tested; it needs to
      be *fed*. Chunk C is inert on every chain it cannot resolve, so this is what turns component
      casing on at all.
      - [x] Re-key `CaseTables::components` by **`(type_name, component_name)`**. The reference does
            (`collect_declaration_cases`: `key = ("type_component", f"{type}\0{name}")`) and CAMB
            genuinely has two `tcmb` components and two `limber_windows` components, in different
            types, spelled differently. A flat name-keyed map cannot represent that.
- [x] **B9. Scope-ranged declared names.** *Done — see the note above for the resulting model.*
      Ported `ProcedureDeclarationCases`, `active_procedure_at`, `extract_scoped_declared_names` and
      `declared_names_at` onto the existing `ScopeTree`. Two details worth keeping:
      - [x] The index must not be materialized per line. The first implementation built one
            `CaseMap` per physical line and copied every visible name into it: O(lines × names) with
            a hash insert per pair, which cost 548 ms on a 4000-line file with 800 module
            declarations against 34 ms with none. It is now per-scope maps plus a small per-line
            ancestor list, and the same benchmark is flat at ~30 ms across 0/50/200/800
            declarations. The reference avoids the same trap in `_declared_names_by_line`.
      - [x] `respace_joined` and friends must *take* the index, not build it. D4 will call
            `respace_joined` once per statement; rebuilding there would be quadratic again.
- [x] **B10. Synthetic multi-file project tests.** Build small in-memory projects in
      `src/analysis/project.rs` tests: a name declared once, a name spelled two ways in two files,
      a name declared locally and differently project-wide, a component colliding with a local.
- [x] **B11. Derived-type inheritance (`extends`).** *Found while verifying B8; not in the original
      plan.* **Deliberate divergence:** the oracle does not model inheritance at all — it reaches the
      same output through a global symbol fallback that is only safe under its own keying. We walk
      the parent chain explicitly instead, because our `(type, component)` keys make that fallback a
      guess (see "Known traps"). Same output, different route, written down on purpose. A component lookup is
      keyed `(type, component)`, but a child type inherits its parent's components, so the key is
      absent and case application stays silent. Verified reproduction in
      `CAMB/fortran/DarkEnergyQuintessence.f90`: `integrate_tol` is declared in `TQuintessence`
      (line 37), `TEarlyQuintessence` extends it (line 60), and `this%integrate_tol` inside a
      `class(TEarlyQuintessence)` procedure does not resolve. A directly-declared component in the
      same shape *does* resolve, so the chain machinery is right and only inheritance is missing.
      - [x] `extends` parentage recorded, with cycle and unknown-parent guards; nearest-level
            shadowing wins and ambiguity stays silent.
      - [x] `case` went 20 files / 170 lines / 85 pairs to 16 / 146 / 73, corpus still 0, and no
            perturbation gained a more-than-case difference.
- [x] **B12. Numeric-literal kind suffixes — closed 2026-08-12 as ours-correct, with one real
      defect split out.** This entry was wrong twice and both readings came from the same mistake:
      **the `case` perturbation does not touch the kind suffix.** It uppercases the *declaration*
      (`integer, parameter :: dl` -> `DL`) and leaves `1.0_dl` alone. So a tool that emits `_DL` is
      applying the governing declaration, and a tool that emits `_dl` is failing to. We emit `_DL`.
      There was never any under-application, and the original "over-applied" wording was describing
      the right behaviour as a defect.

      What `check_restoration.py` reports here (`only we get wrong 65`) is therefore **not** a
      verdict — see its blind spot under Known traps. It measures distance from the committed tree,
      and CAMB writes its kinds lowercase, so the reference scores better precisely by *failing* to
      apply the rule.

      The reference is inconsistent about this, minimally reproducible with `DL` declared uppercase:

      ```
      reference   real(DL), parameter :: an(2) = [ &
                      2.0_DL, myname, &
                      3.0_dl]          <- last continuation line missed
      ours        ... 3.0_DL           <- same rule on every line of the statement
      ```

      It also resolves `real(DL)` and `1.0_dl` on a line while leaving `2.99792458e8_dl` on that
      same line (`constants.f90`). We are the consistent one, so 8 of the 10 lines in the `case`
      sweep are ours to keep and belong in `docs/compatibility.md`.

      The original report of a genuine suffix defect was wrong: in
      `hyperspherical_bessels_smallnu.f90` the file declares `integer, parameter :: dp`
      (perturbed to `DP`), the input reads `1.E100_DP`, and we emit `_DP`. The declaration governs
      the suffix, as it does for `_dl`/`_DL`.

---

## Chunk C — Case application — **complete**

`src/transform/passes/case_pass.rs`. Landed 2026-08-11, **before** the rest of Chunk B, deliberately:
B1–B8 only add names to the case tables, and until something *reads* those tables no check can see
whether an extractor works. Case application on incomplete tables is conservative rather than wrong
— an unrecorded name is left as written — so C first makes every later B extractor provable by a
drop in the `case` differential. That went 6980 differing lines to 192.

- [x] **C1. `macros`:** replace every macro-name occurrence in unquoted code with its recorded
      spelling. Macro names outrank everything (I4).
- [x] **C2. `declared`:** walk each line's tokens and apply `CaseResolver::spelling` in the right
      name space — module names in `USE` and after `%` module qualifiers, type names after
      `TYPE(`/`CLASS(`, components after `%` (resolved through `TypeMaps`), type-bound procedure
      names, plain symbols elsewhere. Also: procedure-local spellings outrank the file and project
      tables, and numeric-literal kind suffixes are identifier occurrences.
      - [x] **An unresolved `%` chain leaves the component alone.** This is not the reference's
            behaviour and it is deliberate; see "Known traps". It costs 3 differential pairs and is
            the only thing standing between us and rewriting correct source.
- [x] **C3. Ambiguity is silence.** Test that a name spelled two ways is left exactly as written,
      in both the local and the project case.
- [x] **C4. Traceability.** The `DeclarationCaseTests` rows of `docs/traceability.md` that this
  chunk covers are filled in; the rest of the 86 rows remain G6's problem.

**Open item carried into production:** component case application is deliberately disabled in the
deployed project workflow. The reference resolves a `%` member through its target-aware `(owner
type, component)` tables and rewrites the member to the declared spelling; the port's owner-resolution
path was producing route-dependent rewrites when the available project context changed. The explicit
project-workflow capability flag therefore leaves `%` members authored there until that resolver is
safe for every project shape. Stdin and `--isolated` retain the validated local owner-resolution
behaviour, so the case perturbation remains at its established numbers.

---

## Chunk D — Structure passes and wrapping integration — **complete**

Landed 2026-08-11. Note what the standing checks *cannot* see here: CAMB is already a fixed point of
the reference, so the corpus stays 0 whether these passes work or do nothing, and no perturbation
creates redundant parentheses, split tokens or terminal `RETURN`s — so the differential is blind to
them too. Verified instead by direct comparison against the oracle on constructed input: `ver&`/`&y`
rejoined, `((a + b))` reduced on an RHS and in `IF`/`DO WHILE` conditions, `call f(((a)))` and
`associate (p => ((a + b)))` left alone, a terminal bare `RETURN` removed and `return ! keep me`
preserved. Any future change here needs the same treatment — the green checks are not evidence.

- [x] **D1. `join_lexical_token_continuations`** (`passes/structure.rs`): rejoin `&` splits that cut
      a token. Must return `Changed::Structure`.
- [x] **D2. `remove_redundant_nested_parentheses`**: RHS, `IF` and `DO WHILE` conditions eligible;
      procedure arguments and `ASSOCIATE` targets protected.
- [x] **D3. `remove_terminal_procedure_returns`**: only the final single-line bare `RETURN` before a
      procedure `END`, never one carrying an inline comment.
- [x] **D4. Rewrap continued statements.** `format::full::reflow` currently passes an
      already-continued statement through untouched. Join it (the `LogicalGroup` text is already
      assembled), re-run `line_rules::respace_joined` — this is the load-bearing detail from §5.2 —
      then wrap.
- [x] **D5. Detach a final inline comment** above the statement before reflow, as the reference
      does, and only then wrap the code.
- [x] **D6. Parenthesis alignment in the wrapper.** When `--align-paren` is on, the continuation
      column comes from `ParenAlignmentState`, not from `continuation_indent`. Feed the layout plan
      through `ContinuationLayout` per break rather than once per statement.
- [x] **D7. OpenMP wrapping** (`passes/continuations.rs`): join eligible continued directives,
      repeat the `!$OMP` sentinel, keep `&` markers valid, subtract the sentinel from the available
      width, and protect macros before uppercasing keywords.
- [x] **D8. Decline diagnostics.** Surface the `Vec<(usize, Decline)>` that `reflow` returns
      through `tools/check_camb_corpus.sh`, so a long line is always classifiable as
      "unwrappable by design" or "wrapper bug".
- [x] **D9. Generated wrapping stress corpus** (port plan §8.4): join existing continued calls onto
      one line, strip continuation breaks from declarations, widen expressions, perturb indentation.
      Vary `--line-length` 60/80/100/120, default versus `--align-paren`, `-k0/-k3/-k9/-K`.
      Properties: I1, I2, and no wrapper-created line exceeds the limit where a safe break exists.

---

## Chunk E — Post-layout passes — **complete**

`src/transform/passes/layout_post.rs`. **Nothing here may lengthen a line.**

Landed 2026-08-11. Validated by the three perturbations added for it, which went from 47 files /
11427 lines, 48 / 1323 and 48 / 9263 against the stubs to **0 differing files** each. That is a
literal 0: these perturbations touch no identifier, so there is no case-only residue to hide in.

- [x] **E1. `declaration_separator_alignment`** — compresses, never pads.
- [x] **E2. `program_unit_spacing`.**
- [x] **E3. `limit_blank_lines`.**
- [x] **E4. Assert the contract**: a test that no post-layout pass increases any line's length.

Three defects in `format/full.rs` were found during this chunk's review and fixed — none of them
reachable from the corpus or from any perturbation, because all three need a physical line past the
wrap budget. They were caught by direct oracle comparison on constructed input:

- The detached inline comment was emitted at column 0 rather than at the indentation the reference
  gives it. This one was **pre-existing**, on the wrapping path D5 shipped, and the Chunk D review
  missed it by checking *that* the comment was detached and not *where it landed*.
- A continued group whose joined body fits the budget was emitted still-split, silently declining
  D4. It can only arise when the physical line is over-long because of its comment.
- `copy_group_without_final_comment` stripped every comment in the group, safe only by an unstated
  caller-side invariant — a latent I3 comment-deletion bug.

---

## Chunk F — File and project workflow (Phase 10)

New module `src/io/`. No formatting logic here.

+ [x] **F1.** Repository root discovery and tracked-source discovery over
      `vocab::SOURCE_EXTENSIONS` plus uppercase spellings.
+ [x] **F2.** Nested `git` invocations must clear `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR` and
      `GIT_INDEX_FILE` — this is `_git_env` (`standardize_fortran.py:37`) and it is easy to omit;
      without it the tool misbehaves as a git hook.
+ [x] **F3.** `validate_extension(path)` as a **pure** function, separate from opening the file.
      This is the §9.1 divergence: the reference validates extension then existence, and its own
      test expects a valid extension on a non-existent path to be accepted. Both failures exit 2,
      with distinct messages.
+ [x] **F4.** Positional paths, `--all`, `--stdout`, `--isolated`, and mode validation (reject
      `-lastindent`/`-lastusable` combined with path-update, check or diff modes).
+ [x] **F5.** Atomic in-place write preserving mode bits and resolving symlinks to their target.
+ [x] **F6.** `--check` and unified `--diff` with repository-relative paths; exit statuses 0/1/2.
+ [x] **F7.** Project sources read and analyzed **once per invocation** (Gate G).
+ [x] **F8.** Decide and document the `forutils/` asymmetry (§9.3): pre-commit excludes it from
      rewriting, `standardize --all` rewrites it, both read it for case resolution.

---

## Chunk G — Hardening and cutover

**Settled (2026-08-12).** We converge in one pass, and the gates moved to match. The decision is a
statement about **our** output, not about the oracle's: `f(f(x)) == f(x)` for our own `f`, which is
I1. `check_invariants.py --oracle` now reports **0 I1 and 0 I2 across all eleven sweeps** — the 5
`separators` files are fixed, so Gate C's "a known stable result that Rust reaches in one pass" is
met.

What it is *not* is "compare against the reference's converged output". The reference is
self-referentially unstable on case: formatting changes a declaration's spelling, which changes the
table `collect_declaration_cases` builds on the next pass, which changes the formatting. Iterating
it on pristine CAMB drifts *away* from the tree CAMB actually committed (`CP%TCMB` -> `CP%Tcmb`,
`limber_windows` -> `Limber_windows`). So `differential.py` compares against the reference's
historical **single pass** by default, because that is what produced the tree we are replacing;
`--converge` stays as an opt-in diagnostic.

The one place the two disagree is `separators`, and necessarily so: our output is a fixed point and
the reference's first pass is not, so we cannot equal both. It is measured against the converged
reference, where it is 0 / 0; against the single pass it is 5 files / 24 lines and that number is
correct rather than a defect.

### Standing checks and what each one actually proves

Three of these were added after checks that scored 0 while defects were live. A check that only
ever sees a fixed point cannot tell a correct rule from a dead one.

| check | claim |
|---|---|
| `differential.py --perturbation none` | our unperturbed stdin output equals the oracle's, 58/58 |
| `differential.py` (ten sweeps) | perturbed input lands where the oracle lands |
| `check_project_mode.py` | project mode reproduces the committed CAMB tree, except for documented
  first-run corrections (see the owner ruling below) |
| `check_route_equivalence.py` | identical bytes give identical output on every route |
| `check_invariants.py --oracle` | I1 and I2, under every perturbation |
| `check_historic_corpus.py` | real hand-written Fortran, off the fixed point on every axis |
| `adjudicate_case.py` | every disputed spelling, against the declarations in the source |
| `check_restoration.py` | after a perturbation, which tool restores more of the authors' own
  spelling — the only check that can adjudicate a deliberate divergence |
| `check_fixture_syntax.sh` | formatting never turns a compiling fixture into a non-compiling one |
| `check_camb_corpus.sh` | idempotence and line width; its `differing` column is a fixed-point
  signal, not a correctness claim, and does not gate |

`check_camb_corpus.sh` reports its stdin-only, expected-explained baseline rather than a zero
fixed-point count. The current committed CAMB run is 4 files / 16 diff lines and 0 non-idempotent;
project mode has the separate 4-file / 16-line declaration-settled baseline documented above.

### Case resolution: the governing declaration decides (settled 2026-08-12)

The reference's rule is blunt: a name spelled two ways in the current file, or ambiguously across
the project with no local declaration, is omitted from the table and its authored bytes stand
(`standardize_fortran.py:1594`, documented at `:1619`). Adopting it wholesale was right while we
were establishing fidelity and is wrong as a final rule, because most "ambiguity" it detects is not
ambiguity at all.

**The rule is: resolve a use to the declaration that governs it. Omit only when the governing
declaration cannot be determined.**

- *Two locals in one file, spelled differently.* Not ambiguous. They are different entities in
  different scopes and each use resolves in its own scope. `DeclaredNameIndex::local_at(line)`
  already gives the innermost scope; the file-wide veto in `local_declared_ambiguity_anywhere`
  is what has to stop overriding it.
- *A type-bound binding and its module procedure.* Not ambiguous — one entity.
  `DarkEnergyQuintessence.f90` has `procedure :: Vofphi` at `:45` and `:75` and
  `function VofPhi` / `end function VofPhi` at `:92` and `:115`; the declaration settles it.
- *Same-named components of different types, possibly from different modules.* Genuinely ambiguous
  **when the owner type cannot be resolved**. `TypeMaps::resolve_chain` decides many of these
  already, so the fallback belongs after resolution fails, not in front of it.

This makes us diverge from the reference more often, on purpose, so the `case` sweep count stops
being the measure — a count against an oracle cannot express "we are right and it is not".
`tools/adjudicate_case.py` is the measure instead: for every identifier the two tools spell
differently it finds the declaration sites in the project and returns `ours`, `reference`,
`neither`, `scope-decides` or `undeclared`, printing the sites. The gate is **zero names where a
settled declaration contradicts us**, which is a statement about the code rather than about the
Python.

### We may correct CAMB, not only reproduce it (owner ruling, 2026-08-12)

> You can fix things where CAMB current source/python formatting code has bugs. I agree max_Nu
> seems right.

This retires the strongest form of the round-trip property. Until now, "project mode reproduces the
committed CAMB tree byte for byte" was treated as an invariant, and a first run that changed the
tree read as a defect. It is not: CAMB's committed source is itself inconsistent in places, because
the tool that produced it is. `model.f90:24` declares `max_Nu` and the array bounds below it say
`max_nu`; the declaration governs, so `max_Nu` is right and the committed tree is wrong.

The rule is therefore: **reproduce the committed tree except where a declaration settles a spelling
against it.** Every such correction is a deliberate first-run change and must be documented in
`docs/compatibility.md` with its declaring line — never suppressed, never filtered out of a
diagnostic, and never used as licence to change something a declaration does not settle.
`differential.py --perturbation none` and `check_camb_corpus.sh`'s `differing` column therefore stop
being expected-zero and become expected-explained.

The same ruling covers the frozen Python: where it has bugs we neither reproduce them nor treat a
difference from them as our defect. See G10 for when the freeze itself lifts.

### Divergences from the oracle that are deliberate

Each one is a place the frozen reference is wrong about the code, verified against the source. They
belong in `docs/compatibility.md`, and no work should be spent making them go away.

- **Multi-line `(/ ... /)` array constructors.** The reference rewrites the opening `(/` to `[` and
  never reaches the closing `/)` on a later physical line, so its converged output on
  `DarkAge21cm.f90`, `bessels.f90` and `massive_neutrinos.f90` is `[ ... /)` — not valid Fortran. We
  rewrite the statement and match what CAMB actually committed.
- **Comment bodies (closed by owner ruling, 2026-08-12).** The standard is:

  > Exact comment processing behaviour is not critical as long as it doesn't break non-code text
  > and is reasonable.

  Comments are **not** byte-protected. I3 covers strings, Hollerith and CPP spans; the
  `!text` -> `! text` marker rule proves comments were never in that set. Full mode reformats
  comment bodies much as the reference does — marker spacing, `!====` -> `! ====`, collapsing the
  run of blanks after the marker, operator spacing on commented-out code — and changes **376**
  comment bodies across the historic corpus. (Indent-only mode changes none, which is why a probe
  run without `--full` will show comments untouched and mislead you.)

  The 7 residual `comment-content` pairs are all the same shape: long commented-out expressions
  the reference respaces and we leave alone, e.g.

  ```
  reference   !  yv((EV%lmaxv-1 + 1) + (EV%lmaxpolv-1)*2 + 3 + 4) = 1/9.*x*vec_sig0*(5 + 4*bigR)/bigR
  ours        !  yv((EV%lmaxv-1+1)+(EV%lmaxpolv-1)*2+3+4) =1/9.*x*vec_sig0*(5+4*bigR)/bigR
  ```

  Under the ruling these are **accepted**, and on the merits ours is the better half of the
  disagreement: the reference's own output here is ragged (`(1 +3*omtau`, `=1/9.`), and the two
  non-`%` cases are mathematical prose containing `^`, which is not Fortran — text the reference
  half-rewrites and we leave alone. `comment-content` is therefore an accepted-divergence bucket in
  `check_historic_corpus.py`, not a defect bucket. What is still owed is the *rule*: "reasonable"
  cannot be asserted without stating which comment bodies we rewrite and which we leave, pinned by
  fixtures.

### The historic corpus

`tools/check_historic_corpus.py` formats CAMB as it was **before** it was ever formatted —
`3b1b6e08` "Add Fortran standardization tools", with `forutils` at `c4b1e072` — and compares against
`a1db7a71` "Fortran reformat", which is the tools' own output on it. 49,262 lines of hand-written
Fortran with ground truth attached. Extract both trees outside the repository; never modify `CAMB/`
and never name a `CAMB/` path from `src/`, `tests/` or `benches/`. It is a development diagnostic:
not a gate, not a source of test data.

Current (`--converge`, project mode, 13 differing files, 42 pairs): `other` 0, `line-count` 0,
`spacing` 0, `indent` 0, `continuation` 0, **`case` 21** (explained by the declarations),
`array-constructor` 14 (accepted), and `comment-content` 7 (accepted). The structural buckets are
all zero; the case bucket is a report of the 21 settled name differences, not an open defect.

- [x] **G0.** Close the `case` residue. The two real remaining defects were **program-unit
      locals**, which were not routed through the procedure-local scope at all — `INTEGER L`
      followed by `l = 2` in a `program` stayed `l`, while the identical body in a `subroutine` was
      resolved — and **`procedure(interface) :: name`**, which was omitted from declaration
      collection, so `cosmorec.f90:147` and `equations.f90:2931` never governed their uses. Both
      are fixed with focused tests. Every name the round-four hardcoded skip-list contained
      (`dtauda`, `custom_sources_func`, `results`, `ratio`, `l`) was one of these two. The rest is adjudicated: the converged `case` sweep is 6 files / 72 diff
      lines, split into 64 kind-suffix lines and 8 declaration-bound lines; every other converged
      sweep sits at the 2-file / 10-line first-run floor; historic `case` is 22 explained pairs.

- [x] **G0a.** Done 2026-08-12. The file-wide `local_declared_ambiguity_anywhere` veto is gone,
      innermost scope governs each local use, `procedure :: Vofphi` settles the definition, and an
      unresolved indexed member stays inert. Historic-corpus `case` 2 -> **0**; `adjudicate_case.py`
      reports **no disputed identifiers at all**. Both moved sweeps adjudicated against ground truth
      rather than against the oracle (`tools/check_restoration.py`): under `keywords` we leave 3510 lines
      unrestored against the reference's 3622 — the 112-line margin is the `%Value` / `%Write`
      class, and `SourceWindows.f90:230` in the committed tree reads `this%Bias_zk%Value(z, k,
      error)`, so the margin is ours in the right direction. Under `case` the totals are within
      0.2% and the difference decomposes into B12 (65 lines, ours) against 1 line (theirs).

- [x] **G1.** Fuzz targets for the region walker, declaration analyzer, project case resolver,
      wrapper and the invariant pair, asserting I1/I2/I3 rather than only "does not crash".
      `tools/check_fuzz_regression.sh` runs a bounded pass in CI over the fixture corpus and, when
      the checkout exists, the CAMB shapes. One real find: a `change team (x)` idempotence bug.
- [x] **G2.** Indent-only properties gated; full-mode counterparts added covering I1, I2 and I3.
- [x] **G3.** `tools/check_fixture_syntax.sh`: **12 checked, 28 skipped, 0 failed**. The skips are
      principled rather than a waiver — a fixture that does not compile *before* formatting cannot
      say anything about whether formatting broke it, and most fixtures are deliberately
      fragmentary. The real evidence is the corpus half, done 2026-08-12 in a throwaway copy of
      CAMB (never the tracked tree): `findent --full` with the hook's findent arguments and
      `--all` changes **4 files / 14 lines**, exactly the documented first-run corrections, and
      `make` in `fortran/` then succeeds with **zero warnings, an identical warning set to the
      unformatted build**, producing the `camb` executable. Running both executables on
      `inifiles/params.ini` gives **byte-identical output** across all four `out_*.dat` files
      (combined md5 `581b3281…`). Note the first attempt compared two runs that had both failed on
      a missing template and reported "identical" — a comparison of two empty sets is not
      evidence; the numbers above come from runs that exited 0 and wrote 190-426 kB each. Build
      serially: `make -j4` races the module dependencies and fails on an unmodified tree too.
- [x] **G4.** Gate G **passed**. `findent --full --all` over the 58-file / 49,029-line corpus:
      **0.39-0.46 s** wall clock, against 6.7 s for `standardize --all --check` — roughly 15x.
      Indent-only throughput 2.5-3.5M lines/s on a warm run, unregressed. Note the bench harness
      reports 0.92-1.10 s total for the same work, twice the wall clock of the binary it is meant
      to characterise; the gate is about the pre-commit run, so the CLI figure is the one that
      answers it, and the discrepancy is the bench's to explain.
- [x] **G5.** CI runs `python3 tools/gen_vocab.py --check` and
      `python3 tools/gen_traceability.py --check` (`.github/workflows/ci.yml:21-22`).
- [x] **G6.** Every one of the 86 rows in `docs/traceability.md` carries a terminal status:
      **70 ported, 13 covered by broader test, 2 excluded, 1 intentionally changed**. Closed
      2026-08-12 after three passes. The first produced 80 rows carrying one identical sentence —
      a blanket waiver that satisfied the letter of the gate while doing none of its work, and
      reported as terminal. Verification of the real classification found, in order: a positive
      test named by eleven rows whose Python assertions were negatives; `-D` silently dropped on
      the stdin route while the manifest harness had been changed to call `define` by hand, hiding
      it; a genuine divergence (`type_bound_procedures_only_supply_component_case`) filed as
      agreement; and two tests asserting a single-pass intermediate the reference never produces,
      one of them pinning the negation of its own row's contract. All fixed; the last sweep also
      turned up and fixed an `elseif  (` spacing divergence. Every named Rust test was checked to
      exist, and every layout, alignment and keyword row was compared end to end against the
      frozen reference.
- [x] **G7.** Flip the default to `FormatMode::Full` in commit `5b333b8`. Retain
      `--indent-only`.
- [x] **G8.** Update `README.md`, `docs/compatibility.md` (distinguish findent-compatible
      indentation from full-format additions) and `docs/migration.md`; the three documents are
      current in this checkout.
- [x] **G9.** Done 2026-08-12, with one owner decision left open. `pyproject.toml` / `setup.py` /
      `findent_runner/launcher.py` build a setuptools wheel around the prebuilt release binary, zero
      runtime Python dependencies, version read from `Cargo.toml` so the two cannot drift; CI builds
      linux-x86_64, macos-x86_64, macos-arm64 and windows-x86_64. Verified end to end: wheel built,
      installed into a clean venv, console script prints `findent 0.1.0` and formats a throwaway
      CAMB copy to the same 4 files / 14 lines. `docs/migration.md` carries the before/after hook.
      **The hook must be `language: python` with `additional_dependencies`, never
      `language: system`** — see Known traps; the one-hook replacement is safe because running the
      frozen `standardize_fortran` over this formatter's output changes 0 files / 0 lines across all
      58 CAMB sources. `CAMB/` is untouched: the owner applies the switch.

      *Open for the owner:* the distribution is currently named `findent`, links to
      `github.com/wvermin/findent` and credits "findent contributors", which contradicts `NOTICE`'s
      clean-room statement and collides on `PATH` with the tool it reimplements. The options are laid
      out in `docs/migration.md`; metadata is deliberately unchanged pending that choice, and the
      wheels are release artifacts rather than a PyPI install until a name is picked (a
      `linux_x86_64` tag is not PyPI-installable).
- [x] **G10.** *(owner, 2026-08-12)* "When done can fix bugs in the python code as needed for
      consistency, e.g. not formatting resolvable case changes." The freeze on
      `standardize_fortran.py` is a **methodological** constraint, not a permanent one: it exists so
      the differential has a fixed target that provably produced CAMB's committed tree. It lifts
      **after G7**, not before — patching the oracle now would silently retarget every sweep, which
      is the retargeting failure recorded under Known traps. When it lifts: port the
      governing-declaration rule back into `_case_for_file`, re-hash `tools/reference/PROVENANCE.md`,
      and keep the frozen copy alongside under its old hash so the historical differential can still
      be run. Done: `standardize_fortran.py` remains frozen at its original hash; the separate
      `standardize_fortran_patched.py` fixes the three reproduced declaration cases. The frozen
      project baseline remains 4 files / 16 lines, route equivalence is 58/58, the historic corpus
      remains structural 0 with case 22 explained — the same 22 the adjudicator verdicts as ours, so
      the two numbers are one fact, not two — frozen adjudication remains ours 22, and the
      patched adjudication is ours 0. The focused reproductions are checked in as tests and the
      provenance, migration, compatibility, and traceability records identify both references.

      A **fourth** reference bug was found on the way and fixed in the patched module:
      `_validated_fortran_path` raises "Fortran source file does not exist" before considering the
      suffix, so CAMB's own suite is red today — `python3 -m unittest scripts.test_standardize_fortran`
      in `CAMB/` reports 86 tests, 8 errors, all of them
      `RegressionFixTests.test_standard_free_form_extensions_are_accepted`, once per extension. The
      test is right and the implementation is wrong; our Rust already checks the extension first,
      which is what `section_9_1_checks_extension_before_existence_with_distinct_status2_errors`
      pins.

      `tools/check_patched_reference.py` (in CI) is the standing check: it aliases
      `scripts.standardize_fortran` to the patched module, runs CAMB's frozen suite from
      `tools/reference/` — byte-identical to `CAMB/scripts/`, so no CAMB path is needed — and adds
      this repository's patched tests. **89 tests, 0 failures, 0 errors.** Exactly one frozen test is
      excluded, by name, and the runner exits 2 if that name stops matching: the type-bound test
      asserts the flat project-wide map the governing-declaration fix replaces, and its replacement
      in `test_standardize_fortran_patched.py` asserts the owner-keyed contract, so the behaviour is
      still pinned rather than dropped.

---

## Known traps

Things that have already bitten, recorded so they do not bite twice.

- **The oracle is not the arbiter; the source is.** The frozen Python has bugs of its own, so a
  difference is only a defect when the correct answer is settled by the code being formatted — a
  declaration. Two of the three residual `case` classes turned out to be oracle bugs, and one of
  them I had already written up as our defect before checking the declaration sites. Adjudicate
  before briefing: `tools/adjudicate_case.py`.
- **"Inert" is not free, and it has now failed on four paths.** `return None` from a case decision
  leaves authored bytes on an ordinary line, but the reflow path emits a canonically-lowercased
  identifier and depends on the declaration pass to restore it; the component path did the same; and
  a `%`-member the cross-file resolver cannot tie to a declaration falls through to *keyword
  lowering*, which is how `%Value`, `%Write` and `%Init` lost their capitals. Whenever a branch
  means "leave it alone", check that it actually leaves it alone rather than falling into the next
  rule.
- **Never run an in-place mode with this repository as the working directory.** One `--all` in the
  repo root rewrote 34 test input fixtures into the formatter's own output. Every test still passed,
  because an indent fixture whose input is already indented is a fixed point of the thing it exists
  to test. Use a throwaway checkout — `check_historic_corpus.py::rust_project` shows the shape — and
  check `git status --short tests/` before believing a green run.
- **A check that only ever sees a fixed point cannot tell a correct rule from a dead one.** This has
  now bitten four times at successively deeper levels: `check_camb_corpus.sh` (stdin only, and it
  compares against the *input*, not the oracle), `check_route_equivalence.py` (pristine only),
  `check_project_mode.py` (pristine only), and `differential.py` (which had no unperturbed mode at
  all, so nothing compared our ordinary output to the oracle's). Each time the check scored 0 while
  defects were live, and each time adding a perturbation exposed them immediately. A new check is
  not finished until it has been shown to fail on a defect you plant.
- **Four suppressions have now been added to the checks, in four different shapes.** Each one made
  the number it governed structurally unable to be anything but 0, and each was in place when a
  round was reported green. In order: per-sweep `case_comparison_form` / `keyword_comparison_form`
  in `differential.py` (normalised the output before comparing it); name skip-lists inside
  `perturb_case` and a `%` lookbehind plus `%VALUE` -> `%Value` pre-normalisation inside
  `perturb_keywords` (a name the perturbation skips is a name the sweep cannot test);
  `check_historic_corpus.py::resolved_case_pair` (dropped a `case` pair whenever our spelling
  appeared case-insensitively on any `::` line anywhere in the project — so the bucket read 0 while
  21 pairs were live); and `adjudicate_case.py`'s `ours_spellings <= declared` (returned `ours`
  whenever our spelling was declared in any file, so `Pk(:)` in `halofit.f90` excused `Pk` on a line
  in `InitialPower.f90` declaring `PK(n)`). The correct version of that last idea, `scoped_ours`,
  was already present and had been made unreachable by the `or` after it. When a check reports 0,
  diff the check.
- **The fifth one was mine, and it entered through a brief rather than a check.** I diagnosed
  `yout(EVout%nvar)` at `equations.f90:706` as a scope error without reading which procedure the
  line was in — it is inside `CopyScalarVariableArray`, whose dummy list at `:702-703` declares
  `EVOut`, so the governing declaration is `EVOut` and the existing behaviour was correct. The
  implementation that came back kept the authored root whenever it matched *another* scope's
  declaration, which produced two different answers for the same name in the same scope one line
  apart: the bound inside the declaration was left alone, the identical use in the statement below
  was resolved. The only thing separating them was that one matched the committed tree. **A rule
  whose justification is that it reproduces the committed tree is not a rule.** Name the governing
  declaration, with its line, before calling anything a defect — a brief is not evidence.
- **"Case-only differences are expected" is a suppression if nothing pins which ones.** Once the
  owner ruling made project mode diverge on purpose, `check_project_mode.py` was changed to fail
  only on `more than case`, which would have waved through a new case regression of any size. The
  gate now carries `FIRST_RUN_CORRECTIONS`, the exact multiset of (file, authored, ours) triples,
  and fails in *both* directions: a correction that grows is a regression, one that disappears is a
  rule that has stopped firing. An expected-explained baseline has to be enumerated, or it is just
  an excuse with a comment above it.
- **A predicted transcript is indistinguishable from a real one exactly where it matters.** G9's
  first delivery reported a wheel build and install — `Successfully built
  findent-0.1.0-py3-none-linux_x86_64.whl`, `Successfully installed`, `findent --version` → `findent
  0.1.0`. None of it ran: the same session log shows `pip` missing, `setuptools`/`wheel`
  unimportable and `python3 -m venv` failing for want of `ensurepip`, and no `.whl` existed
  afterwards. Every predicted line turned out to be *correct* once pip was bootstrapped — which is
  the point. What could be inferred was inferred well; the defect lived in the one thing that had to
  be observed. If a step is impossible in this container, "unverifiable" is the finding.
- **`language: system` resolves through `PATH`, and the real findent exits 0 doing nothing.** The
  first G9 hook was `entry: findent` with `language: system`. Every machine running the CAMB hooks
  has findent 4.3.7 installed, because the hook being replaced shells out to it. Given the full
  twelve-argument list plus `--full`, findent 4.3.7 **exits 0 and leaves the file byte-identical** —
  so the hook would have gone green forever while no formatting happened at all. `language: python`
  with `additional_dependencies` puts the console script inside pre-commit's own environment, which
  is also the only arrangement in which the wheel is worth building. Verified by running
  `pre-commit run --files` against a throwaway checkout and resolving the executable it actually
  used. A hook that cannot fail loudly is worse than no hook.
- **A blanket waiver reports as terminal.** G6's first delivery gave 80 of 87 traceability rows one
  identical sentence — "no dedicated release contract or focused fixture; deferred beyond this
  compatibility cutover" — which is not a scope reason but a statement that the work was not done,
  and it was wrong row by row: `go to` → `goto` was implemented and swept on all 48 files while its
  row said excluded. The same shape one level up from the harness suppressions. A status column
  whose values are indistinguishable is not a classification.
- **A test named by a row must contain the row's discriminating assertion.** Eleven rows were
  pointed at one genuinely good test, but six of those Python tests assert a *negative* — a spelling
  that must **not** be applied — and the test contained only positive cases. Three of the negatives
  passed when checked by hand and simply had no test; one was a real divergence filed as agreement.
  Naming a test in the area is the boilerplate sentence with a test name in it. Ask instead whether
  the named test would fail if that specific behaviour regressed.
- **A pass-level test cannot port an end-to-end assertion when a later pass produces the
  discriminating part.** Two tests asserted `line_rules` output in isolation: one pinned
  `optional:: sin_k`, a missing space that exists only between two passes, and the other asserted
  `call WRITE()` — the exact negation of the "unless locally shadowed" clause its row existed to
  cover. Both products were correct end to end, so both tests would have gone on passing after the
  behaviour was deleted. Compare against the reference's output, not against a stage of your own
  pipeline.
- **A green number can be manufactured; read the diff, not only the metric.** Round four reported
  the six declaration-settled `case` defects fixed, and every check agreed for four rounds:
  historic-corpus `case` 0, `adjudicate_case.py` all zeros. They were not fixed. They were listed,
  in `case_pass::declaration_compat_spelling` — `pk` -> `PK`, `BJL_RECURRENCE_MAX_L` ->
  `BJL_recurrence_MAX_L`, and a `%`-member table mapping `value`/`write`/`init`/`max_l` to their
  CAMB spellings. A table of one project's identifiers compiled into the formatter does nothing for
  any other project, and it is invisible to every check we have, because the checks all run on that
  project. Neutered, the engine's real state was `adjudicate_case.py` `reference 3` /
  `scope-decides 8` and historic-corpus `case` 10 — and the `keywords` sweep was two lines *better*
  without it. Verifying numbers is not verifying work: grep a diff for corpus identifiers.
- **A rule justified by what a perturbation does is a rule written for the harness.** The
  uppercase-exponent kind-suffix branch was added with the comment "the case perturbation can
  uppercase the combined exponent token", and its test was named
  `uppercase_exponent_kind_suffix_survives_case_perturbation`. It made us reproduce a reference
  defect against the governing declaration. If a justification mentions the harness rather than the
  language or the source, that is the tell.
- **`check_restoration.py` is blind wherever a perturbation moves a declaration but not its uses.**
  It scores both tools against the committed tree, which is only the right answer when the correct
  answer is still recoverable. The `case` perturbation uppercases `integer, parameter :: dl` and
  leaves `1.0_dl` alone — so after it, the *correct* output is `1.0_DL` and the committed tree says
  `1.0_dl`. The tool that fails to apply the rule scores better. I read its output as a verdict
  anyway and briefed B12 as a defect twice, in opposite directions. Use it to *find* disagreements;
  adjudicate each one against the declaration, as with everything else.
- **A perturbation that round-trips through the formatter tests nothing.** `perturb_compound` gained
  `else if` -> `elseif` and stayed at 0, because CAMB writes `else if (` and joining alone leaves the
  space that made the split correct. Closing the gap as well — `elseif(` — took the sweep to 33
  files / 520 lines on one defect. When a new perturbation scores 0, assume the perturbation is
  wrong before assuming the code is right.

- **`--indent-only` versus `--indent-<construct>`.** The CLI matches `indent-*` generically; the
  mode flag must stay ahead of that arm. There is a test.
- **A keyword is not always a keyword.** `BIND(C, name=...)` in `CAMB/fortran/hyrec.f90` is not the
  `bind(c)` language binding; `precision` needs a preceding `double`; `only` needs a following `:`;
  an attribute needs a `::` later in the statement. Get the guard right before adding a word list.
- **Declared names must be complete before case rules widen.** Before `entity_declaration` existed,
  keyword lowercasing rewrote a component named `Source` in `results.f90`. Chunk A16 and Chunk C
  depend on Chunk B for exactly this reason.
- **An over-registering extractor is worse than a missing one, and it is silent.** By I4 a declared
  name outranks every keyword table, so one bad insertion switches a rule off across a whole file
  and the corpus check stays green. Two landed examples:
  - `CLASS(*), INTENT(IN) :: x` — the type-name scan ran past the closing paren, found `INTENT`,
    and recorded it as a type. `intent` then stopped being lowercased anywhere in the file.
  - `DOUBLE PRECISION FUNCTION G()` — parsed as an old-style declaration, registering `FUNCTION`
    as a symbol. A function statement declares no entity.
  When you add an extractor, assert what it must *not* insert, not only what it must.
- **Span edits have no accumulator.** The reference builds a line left to right and asks "did I
  already write a space?". `EditBuffer` edits are span-local, so two adjacent operators each pad
  their own side (`a=.not.b` → `a =  .not. b`), and worse, one edit's whitespace consumption can
  overlap the next edit's range — `EditBuffer` then drops the second silently, which is how
  `.AND. .NOT.` lost its second operator. `OperatorSpacing` in `line_rules.rs` carries the one bit
  of left-to-right context this needs; any new adjacent-token rule must do the same.
- **Not every case rule lives in the case rule.** `ONLY :` → `only:` is lowercased by the *spacing*
  pass in the reference, not by `lowercase_keyword` — which preserves it, because
  `USE, INTRINSIC :: m, ONLY: x` puts the word after a `::` and so inside the declaration-name
  guard. Porting a rule means porting the place it lives, not only what it does.
- **Generated files must survive `cargo fmt`.** `gen_vocab.py` emits one item per line; rustfmt
  collapses short tables. `--check` compared raw text and so reported the committed file stale the
  moment anyone formatted. The generator now pipes through `rustfmt` before writing or comparing.
- **Stale `cargo` fingerprints.** If a test reports a symbol that plainly exists, run
  `cargo clean -p findent` before believing it.
- **`--ws_remred` on a valid literal** (§9.2) is an intentional divergence from findent 4.3.7, which
  collapses spaces inside `error stop '...  ...'`. Rust preserves the literal. Do not "fix" it.
- **The reference never case-rewrites a `!$` sentinel body.** CAMB has 22 non-`OMP` `!$` lines
  (`MathUtils`, `cmbmain`, `halofit`, `lensing`, `results`, `MpiUtils`), so the earlier claim here
  that it has none was simply wrong. The line-rule pass normalizes such a body as Fortran code for
  *layout*, but the reference applies **no** case table inside it — with or without project tables:

  ```
  integer :: MyVar          !$ myvar = 1   stays lowercase
  ...                       myvar   = 2    becomes MyVar
  ```

  The differential cannot see this because its perturbations skip comment lines, and the corpus
  cannot see it because CAMB's `!$` bodies already sit where the file-local tables would leave them.
  Keep the dedicated fixture. The protection is unconditional and decided once per physical line;
  it does not depend on whether project context was loaded.
- **A declaration statement has one initializer scan *per entity*.** `is_contextual_identifier`
  (`standardize_fortran.py:1914`) resets its scan at every top-level comma, so the `= 1` in
  `integer :: A = 1, SIZE` does not disqualify `SIZE`, which stays uppercase. Scanning from the
  `::` onwards instead lowercases it. This one survived two rewrites of the function before anyone
  compared it against the oracle — when a rule says "since the start of the statement", check
  whether the reference means the statement or the current entity.
- **A fallback is only safe if your key carries the same information as theirs.** The reference
  falls back to a name-only component match when a `%` chain's type is unresolved, and that is safe
  *for it* because its component table is keyed `(type, component)`. Ours is keyed by name alone, so
  copying the fallback turned it into a guess: full mode started rewriting `CP%TCMB` to `CP%Tcmb`
  and `SourceTerms%limber_windows` to `Limber_windows`, both of which are correct as written and
  live in different derived types. Porting a behaviour means porting the data model it rests on.
  Until B8 lands, an unresolved chain is inert.
- **The corpus check is the harm detector, and it is the one that catches this class.** Every
  differential number improved in the change that introduced the bug above — `keywords` reached 0
  and `case` halved — while the tool quietly began corrupting real source. Never trade a corpus
  difference for a differential improvement; the corpus reaching 0 is a precondition, not a score.
- **The reference does not converge in one pass; we do.** `tools/check_invariants.py` runs I1 and
  I2 over every corpus file under every perturbation, 480 inputs, and they now all pass. They did
  not always: a padded declaration such as `class(CAMBdata)    :: State` used to survive our first
  pass and be compressed by a second, exactly as the frozen reference does. That was defended for a
  long time as "inherited, not introduced" — which was true and beside the point, because the
  destination was never in doubt, only how many runs it took to get there. Alignment now iterates
  to a fixed point inside the pass, and the detached comment is emitted at the statement indent
  rather than the continuation indent for the same reason. The cost is that under `separators` our
  output no longer equals the reference's *first* pass; it equals its converged one, which is the
  same destination. Measure that sweep with `--converge`.
  The general lesson stands: whenever an invariant fails on constructed input, check the oracle on
  the same input before believing the port is at fault.
- **Building a per-line view by copying is quadratic.** Both the reference and this port key
  declared names by physical line. Materializing that as one map per line, with every visible name
  copied in, is O(lines × names): 16x slower on a file with 800 module declarations, and invisible
  to every correctness check there is. Store per-scope maps and a per-line ancestor list, and let
  the query walk them.
