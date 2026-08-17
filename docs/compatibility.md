# Rust formatter compatibility

The formatter supports both stdin/stdout and path-based workflows for free-form Fortran. The
library core is byte-oriented and preserves source spelling and non-trailing body bytes while
replacing leading indentation and trimming trailing spaces/tabs. A missing final line terminator is
added using the terminator on the preceding physical line (LF for a one-line unterminated file);
existing LF, CRLF, and mixed terminators are preserved.

Supported compatibility options include the global and per-construct indentation controls, start
indent, continuation policies, labels, includes, OpenMP free-form sentinels, CPP branch snapshots,
`findentfix:` directives, maximum indentation, `END` refactoring, whitespace reduction, and the
`last-indent`/`last-usable` queries described in `history/findent-port-plan.md`.

The Rust release intentionally diverges from legacy findent in three ways:

- Fixed-form *output* requests (`-ifixed`, `-ofixed`, and the fixed format long options) fail with
  status 2 instead of being silently accepted. Fixed-form *input* is detected and skipped rather
  than rejected: see below.
- Unknown options fail with status 2, making misspelled options visible.
- `FINDENT_FLAGS` is not read; configuration comes only from the command line or library API.

Input format detection follows findent's own `determine_fix_or_free` and is enabled by default, so
`-iauto` and `--input-format=auto` are accepted as the default rather than rejected. A source the
detector calls fixed-form is left byte-identical and reported on stderr; `-ifree` forces free-form
handling, and `--query-format` reports the detector's verdict without formatting. The port is
checked against `findent -q` over the whole corpus, currently 4,992 files with no disagreement.

The accepted format is free-form only. The parser is deliberately a shallow structural classifier,
not a full Fortran semantic parser. Unknown or incomplete statements are emitted conservatively.
One narrow legacy recovery is retained for editor-like input: `su broutine` is treated as a
subroutine boundary, and a comma-prefixed external procedure may affect a matching explicit END
fallback without opening a procedure body. Both behaviors are fixture-backed and isolated from
generic malformed-END handling.

## Accepted `--indent-only` divergences

`--indent-only` is otherwise a byte-for-byte contract against `findent -ifree`, and a difference is
treated as a bug in this crate. Three families are the exception: findent 4.3.7 is demonstrably
wrong and reproducing it would mean writing the defect into this crate. Each is reduced to a
standalone case below, and each is excluded from the corpus oracle count on that basis. Seven files
across the five corpus checkouts are affected.

- **`ELSE <construct-name>`.** A named ELSE is standard (F2018 R1105 `else-stmt` is
  `ELSE [if-construct-name]`), but findent's lexer joins the two words, matches neither `else` nor
  `else if`, and leaves the statement at body depth instead of dedenting it to its IF.

  ```fortran
  IF_G : IF (gamma_only) THEN
     x = 1
     ELSE IF_G          ! findent; forformat dedents this to the IF
     y = 2
  END IF IF_G
  ```

  Q-E `PW/src/exx_bp.f90`, `PW/src/exx_std.f90`, `PW/src/newd_acc.f90`.
- **A module whose name begins with a keyword.** findent's keyword-greedy lexer reads
  `MODULE function_types` as a module-procedure heading rather than a module named
  `function_types`, and stops indenting the module body entirely. Renaming the module to `m` makes
  findent indent it correctly, which is what identifies this as a lexer defect rather than a policy.
  CP2K `tools/Fun2D/function_types.f90`, `tools/Fun2D/functions.f90`.
- **A macro or CUDA attribute prefix before FUNCTION/SUBROUTINE.** findent opens no frame for
  `PURE_ARRAY_EQ FUNCTION array_eq_i(arr1, arr2)` (a CPP macro standing in for `PURE`) or for
  `attributes(global) subroutine k(a)` (CUDA Fortran), so their bodies and matching `END` are left
  unindented. CP2K `src/dbt/tas/dbt_tas_util.F`, Q-E `upflib/ylmr2_gpu.f90`.

Everything else that still differs is input that is not valid free-form Fortran — fypp template
bodies, files whose `#ifdef` branches open and close constructs asymmetrically, and one source using
Matlab operators. Those, and the current per-checkout counts, are tracked in
[`outstanding-issues.md`](outstanding-issues.md).

## Intentional full-format divergences

`--indent-only` is the findent-compatible indentation contract. Full mode adds lexical normalization
and wrapping; its reviewed differences from the reference are collected here:

- Multiline `(/ ... /)` array constructors are rewritten as complete, valid `[ ... ]` constructors;
  the reference can change only the opening delimiter on a later continuation.
- Comment bodies are changed only for the narrow, provably assignment-shaped comment rule. The
  reference also respaces seven nested or non-Fortran comment expressions; Rust preserves them.
- A kind suffix follows its governing declaration, including exponent literals. On continuation
  lines the reference can miss that declaration application; Rust applies it consistently. Numeric
  kinds such as `_8` and undeclared names are inert.

  For example, with `real(DL), parameter :: an(2) = [ &` and a declared `DL`, Rust applies the
  declaration to every continuation line:

  ```fortran
  2.0_DL, myname, &
  3.0_DL
  ```

  The legacy normalizer emits `3.0_dl` on that final continuation line. The eight-line
  `constants.f90` split is retained as an adjudicated divergence, not normalized away.
- An exponent kind suffix follows its own governing declaration even when the exponent token was
  perturbed to uppercase. Thus a declaration `dp` governs `1.E100_DP` as `_dp`; the reference's
  two-line `constants.f90` continuation defect is retained as an adjudicated divergence.
- The findent-oracle and Rust agree byte-for-byte on the resolved governing-declaration cases: owner-keyed
  type-bound bindings and old-style/typed local entities.
- Conditional `!$` sentinels retain the authored sentinel boundary spacing while their Fortran-like
  body is normalized, including declaration-driven identifier casing.
- `--ws_remred` on a valid literal leaves the literal bytes intact. A legacy heuristic can
  treat the quote after `error stop` as code and reduce spaces inside that literal.

These are full-format policy choices, not indentation compatibility claims. The array-constructor,
comment, sentinel, kind-suffix, governing-declaration, valid-literal, and type-bound-procedure cases
are pinned by checked-in fixtures and focused tests described below.

### Fixture fixed point

Project-mode behavior is exercised by checked-in multi-file fixtures. A run must leave fixture bytes
stable, and a difference that is not settled by the governing declaration remains authored rather
than borrowing a spelling from another scope. The fixed-point checks compare raw bytes; in
particular, CRLF-to-LF changes are failures rather than text-mode equivalents.

## Regression checks

The in-tree suite covers byte handling, newline preservation, compact `END` forms, semicolon
statements, keyword identifiers, `findentfix`, CPP branch restoration, labeled `DO`, idempotence,
and preservation. When findent 4.3.7 is available, run
`tools/differential_free.sh target/release/forformat`; it compares the retained non-fixed legacy
fixtures `progfree.f`, `progfree1.f`, and `progfree-dos.f` with the oracle and reports the
intentional preservation-boundary differences. Large real-world inputs are verified against the
checked-in fixtures rather than an external tree. The
complete legacy shell suite is not a normal Rust test dependency because it includes fixed-form,
relabeling, editor-wrapper, and other explicitly excluded features.

## Legacy free-form audit

The retained free-form portions of legacy tests 11, 14, 15, 16, 19, 20, and 24 pass against
findent 4.3.7 as the oracle. Their supported rows pass; the
reduced behavior is represented by the manifest and focused fixtures. Test 10's
free-form label rows are represented by `label_matrix`; its six-column rows are fixed-form and are
excluded. Test 18 exercises the `wfindent` wrapper and generated/reference files, so it remains an
out-of-scope wrapper/dependency test.

The remaining shell-suite differences are classified rather than hidden:

- fixed-form rows in tests 10, 11, 14, 15, 16, 19, and 20 are rejected by the Rust contract;
- relabeling and `query-relabel` rows in test 24 are post-MVP;
- malformed option concatenation in test 20 is rejected as an invalid Rust option;
- test 20's `WHERE`, derived-type, and `SELECT TYPE/RANK` rows are byte-exact after the shared
  trailing-horizontal-whitespace normalization; and
- test 24's malformed continued-string case is the documented conservative whitespace divergence.

The full `test24` parenthesis-alignment fixture set is also checked in. Its only intentional difference
is the two-line alignment of a deliberately malformed continued string whose quote state spans
the physical group; Rust keeps the quote-aware interpretation instead of aligning parentheses
inside that literal. The valid nested-parenthesis and label-left rows remain byte-exact.

Tests 26 and 27's legacy `doit` calls pass the human-readable case description as an option and are
therefore not executable differential commands as written. Their direct free-form features were
checked independently and are covered by the `fortran2023` and `structures` manifest cases.

## Manifest traceability

The checked-in manifest runner in `tests/manifest.rs` executes every case in
`tests/manifests/core.manifest` without requiring `/opt/findent`. Each case records input, expected
stdout, expected stderr, status, CLI arguments, oracle provenance, category, support/exclusion
status, and an allowed-normalization declaration.

| Capability | Manifest or test coverage |
| --- | --- |
| Core structural indentation and `IF` branches | `core` |
| Strings, semicolons, OpenMP sentinel | `lexical`, `openmp_disabled` |
| Fixed-form rejection and status 2 | `fixed_rejected` |
| Procedure declaration does not open a frame | `procedure_declaration` |
| Parenthesis continuation alignment | `align_paren` |
| `last-indent` / `last-usable` query output | `last_indent`, `last_usable` |
| Redundant whitespace transformation | `ws_remred` |
| Bare optional values and indent disabling | `ws_remred_bare`, `indent_none` |
| Supported construct matrix | `constructs` |
| Continued source around CPP directives | `cpp_continuation`, `cpp_continuation_indent` |
| Nested CPP alternate branch snapshots | `cpp_nested` |
| Include/label/ampersand CLI layout | `cli_layout`, `cli_include_left`, `cli_label_left`, `cli_indent_ampersand` |
| Combined label/include/continuation layout | `cli_layout_combo` |
| END completion and case policy | `refactor_end`, `refactor_end_upper` |
| Procedure prefix/attribute boundary | `procedure_matrix` |
| SELECT TYPE/RANK, named/shared-label, legacy structure families | `advanced_constructs` |
| Unknown-option diagnostic divergence | `unknown_rejected` |
| Conservative malformed-string whitespace boundary | `ws_malformed_string` |
| Maximum-indent/start-indent state and branch clamping | `engine_max_indent_start`, `constructs_max_indent` |
| CONTAINS restart state | `engine_contains_restart` |
| OpenMP continuation/start-indent policies | `openmp_continuation_default`, `openmp_continuation_k`, `openmp_continuation_k9`, `continuation_none` |
| Fortran 2023 procedure prefixes | `fortran2023` |
| Nested legacy STRUCTURE/UNION/MAP | `structures` |
| Legacy split `su broutine` recovery and comma-prefixed external END fallback | `legacy_split_procedure`, `legacy_orphan_procedure_end` |
| Nested parenthesis alignment and label-left interaction | `align_nested`, `align_nested_label_right` |
| Full Test026/Test027 parenthesis fixtures and malformed-string boundary | `align_legacy_full`, `align_legacy_full_label0` |
| Compatibility audit regressions: first-item parenthesis target, CONTAINS frame retention, abstract-interface END guard | `compat_regressions` |
| Shared-label `DO` closure across nested CPP branches | `labeled_cpp_do` |
| Consolidated free-form legacy construct matrix | `legacy_free_matrix` |
| Critical/change-team indentation controls | `legacy_controls` |
| Malformed explicit-END recovery | `malformed_end_recovery` |
| Malformed explicit-END matrix | `malformed_end_matrix` |
| Per-construct long-option indentation matrix | `construct_options` |
| Label placement with global/start indentation | `label_matrix_left0`, `label_matrix_left1` |
| Arbitrary bytes, preservation, idempotence | `tests/properties.rs` |
| Scanner quote/Hollerith/newline spans | `src/source/{buffer,scanner}.rs` unit tests |
| Classifier families, keyword assignments, `TYPE IS` | `src/classify/recognizers.rs` unit tests |
| CLI aliases, attached/separated values, optional values | `src/cli.rs` unit tests |

The table is intentionally additive: legacy features that are not represented here remain release
work, rather than being implied by a broad capability claim.

## Whitespace boundary

The Rust library's default contract replaces leading indentation and trims trailing spaces and tabs
from each emitted physical line, matching findent 4.3.7's free-form emission. Other source spelling
and body bytes are preserved when transformations are disabled, including spaces inside strings and
comments. `--ws_remred` remains the explicit opt-in for broader redundant-whitespace reduction, and
Hollerith-bearing statements bypass it.
Malformed or ambiguous continued string expressions are reduced conservatively; this is an
intentional divergence from findent 4.3.7's legacy handling of the `Test031` malformed-string case.
There is one additional reviewed divergence: findent 4.3.7's `remred` heuristic treats the quote
after `error stop ` as code and collapses spaces inside the valid character literal. Rust keeps the
literal unchanged; `ws_remred_valid_literal` records this oracle defect explicitly.

`--ws_remred` also diverges from legacy findent around the two column-alignment options,
`--align-declarations` (default on) and `--align-comments` (default off): each owns the one gap it
aligns — the whitespace before a declaration's `::` and before a trailing comment, respectively —
and `--ws_remred` leaves that gap alone whenever the corresponding option is enabled, rather than
collapsing it before the alignment pass sees the authored spacing. Legacy findent has no equivalent
alignment options, so this precedence is Rust-only behavior with no oracle to diverge from.

COCO (`??`) and FYPP (`#:`) directive recognition is currently retained only for safe grouping and
branch continuation. They are not part of the first compatibility release's supported semantic
contract; CPP behavior is the supported preprocessor feature. Full COCO/FYPP behavior is deferred
until it has checked-in fixtures and a separate compatibility decision.
