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
cargo build --release && ./tools/check_camb_corpus.sh   # must report 0 differing, 0 non-idempotent
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

Current state, 48 files under those globs:

| Perturbation | Differing files | Differing lines | Differing pairs | More than case |
|---|---|---|---|---|
| all nine except `case` | 2 | 6 | 3 | **0** |
| `case` | 16 | 146 | 73 | **0** |

I1 (`f(f(x)) == f(x)`) and I2 (`indent_only(full(x)) == full(x)`) hold across all 48 files, and
across all 480 perturbed inputs except the 5 `separators` cases described under "Known traps",
where the reference does not converge in one pass either.

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
- [ ] **B12. Numeric-literal kind suffixes are over-applied.** 46 of the 73 remaining `case` pairs
      are `_dp` where the reference keeps the author's spelling and we impose the declared one.
      Over-application is the dangerous direction, so this wants a real answer rather than a
      tolerance. It does not show on the corpus — CAMB declares its kind parameters lowercase — so
      it is only visible under the `case` perturbation.

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

**Open product decision, needs an owner before G1 and G8.** On input that is not already a fixed
point, the reference pipeline takes two passes to settle, and we faithfully reproduce its first
pass — so I1 is not universally true (5 files under `separators`; see "Known traps"). Two options,
and they are not equivalent:

- **Match the single pass** (what we do now). Faithful to the tool being replaced, including its
  wobble: a developer formatting a fresh file sees the hook modify it twice, exactly as today.
  I1 and I2 become "holds on fixed points", not invariants.
- **Iterate internally to a fixed point** (cap 2–3 passes). Makes I1 and I2 true by construction,
  and the output equals the reference's *converged* answer — the same destination, one step sooner.
  Costs a second pass only when the first changed something, and requires `differential.py` to
  compare against converged reference output rather than a single pass, which changes the
  acceptance numbers for every chunk.

The corpus does not distinguish them: CAMB is a fixed point, so both produce identical output on
every real file today.

Evidence for the second option: the port plan's own **Gate C** already requires "a known stable
result that Rust reaches **in one pass**". Current behaviour does not meet that gate on the 5
`separators` files. Either the gate is relaxed or the behaviour changes; leaving both as they stand
is the one outcome that is definitely wrong.

Evidence for the second option: the port plan's own **Gate C** already requires "a known stable
result that Rust reaches **in one pass**". Current behaviour does not meet that gate on the 5
`separators` files. Either the gate is relaxed or the behaviour changes; leaving both as they are is
the one outcome that is definitely wrong.

- [ ] **G1.** Fuzz targets for the region walker, declaration analyzer, project case resolver and
      wrapper, plus the I1/I2 invariant pair.
- [ ] **G2.** Gate `tests/properties.rs:29`, `:38`, `:94`, `:107` on indent-only mode and add the
      full-mode counterparts.
- [ ] **G3.** `gfortran -ffree-form -ffree-line-length-none -fopenmp -fsyntax-only` over
      self-contained fixtures before and after formatting; the CAMB build for the real corpus.
- [ ] **G4.** Benchmark full mode and project analysis against Gate G: the 58-file / 49k-line
      corpus **well under 1 s**, against 6.7 s for `standardize --all --check`; indent-only
      throughput must not regress from ~3M lines/s.
- [ ] **G5.** CI: add `python3 tools/gen_vocab.py --check` and
      `python3 tools/gen_traceability.py --check` so generated files cannot go stale.
- [ ] **G6.** Every one of the 86 rows in `docs/traceability.md` carries a terminal status.
- [ ] **G7.** Flip the default to `FormatMode::Full` in one reviewable commit. Retain
      `--indent-only`.
- [ ] **G8.** Update `README.md`, `docs/compatibility.md` (distinguish findent-compatible
      indentation from full-format additions) and `docs/migration.md`; document every intentional
      divergence.
- [ ] **G9.** Build the Python wheel wrapping the prebuilt binaries and switch the CAMB hooks to
      it. Evaluate pre-commit's native `language: rust` support first — if it suffices, it is much
      less machinery.

---

## Known traps

Things that have already bitten, recorded so they do not bite twice.

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
  Keep the dedicated fixture. A guard for this that is conditional on project context being loaded
  is wrong twice over: it fires on the wrong route and misses the plain stdin case.
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
- **The reference does not always converge in one pass, and neither do we — on purpose.**
  `tools/check_invariants.py` runs I1 and I2 over every corpus file under every perturbation, 480
  inputs. Everything passes except I1 on 5 files under `separators`, where a padded declaration such
  as `class(CAMBdata)    :: State` survives the first pass and is compressed by the second. Running
  the frozen reference twice on the same input reproduces it exactly, on the same line: the
  non-idempotence is **inherited, not introduced**, and our output equals the reference's first
  pass. Do not "fix" this by changing the alignment rule — that would break fidelity to the oracle
  while pretending to restore an invariant. It is a product decision, recorded under Chunk G.
  The same shape appears in the detached-comment indent: the reference emits the comment at the
  continuation indent, findent then re-indents it to the statement indent, and the reference's
  second pass agrees. Whenever an invariant fails on constructed input, check the oracle on the same
  input before believing the port is at fault.
- **Building a per-line view by copying is quadratic.** Both the reference and this port key
  declared names by physical line. Materializing that as one map per line, with every visible name
  copied in, is O(lines × names): 16x slower on a file with 800 module declarations, and invisible
  to every correctness check there is. Store per-scope maps and a per-line ancestor list, and let
  the query walk them.
