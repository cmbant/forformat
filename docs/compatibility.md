# Rust formatter compatibility

This crate formats free-form Fortran from stdin to stdout. The library core is byte-oriented and
preserves source spelling and non-trailing body bytes while replacing leading indentation and
trimming trailing spaces/tabs. A missing final line terminator is added using the terminator on the preceding
physical line (LF for a one-line unterminated file); existing LF, CRLF, and mixed terminators are
preserved.

Supported compatibility options include the global and per-construct indentation controls, start
indent, continuation policies, labels, includes, OpenMP free-form sentinels, CPP branch snapshots,
`findentfix:` directives, maximum indentation, `END` refactoring, whitespace reduction, and the
`last-indent`/`last-usable` queries described in `RUST_CONVERSION_PLAN.md`.

The Rust release intentionally diverges from legacy findent in three ways:

- Fixed-form requests (`-ifixed`, `-ofixed`, `-iauto`, and fixed format long options) fail with
  status 2 instead of being silently accepted.
- Unknown options fail with status 2, making misspelled options visible.
- `FINDENT_FLAGS` is not read; configuration comes only from the command line or library API.

The accepted format is free-form only. The parser is deliberately a shallow structural classifier,
not a full Fortran semantic parser. Unknown or incomplete statements are emitted conservatively.
One narrow legacy recovery is retained for editor-like input: `su broutine` is treated as a
subroutine boundary, and a comma-prefixed external procedure may affect a matching explicit END
fallback without opening a procedure body. Both behaviors are fixture-backed and isolated from
generic malformed-END handling.

## Regression checks

The in-tree suite covers byte handling, newline preservation, compact `END` forms, semicolon
statements, keyword identifiers, `findentfix`, CPP branch restoration, labeled `DO`, idempotence,
and preservation. When the 4.3.7 reference installation is available, run
`tools/differential_free.sh target/release/findent`; it compares the retained non-fixed legacy
fixtures `progfree.f`, `progfree1.f`, and `progfree-dos.f` with the oracle and reports the
intentional preservation-boundary differences. For the large real-world input, run
`tools/check_equations.sh target/release/findent equations.f90 equations.f90.indented`; it checks
the generated output, idempotence, and exact oracle agreement when `/opt/findent` is present. The
complete legacy shell suite is not a normal Rust test dependency because it includes fixed-form,
relabeling, editor-wrapper, and other explicitly excluded features.

## Legacy free-form audit

The retained free-form portions of legacy tests 11, 14, 15, 16, 19, 20, and 24 were rerun against
the release binary on 2026-08-09 with findent 4.3.7 as the oracle. Their supported rows pass; the
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

The full `test24` parenthesis-alignment corpus is also checked in. Its only intentional difference
is the two-line alignment of a deliberately malformed continued string whose quote state spans
the physical group; Rust keeps the quote-aware interpretation instead of aligning parentheses
inside that literal. The valid nested-parenthesis and label-left rows remain byte-exact.

Tests 26 and 27's legacy `doit` calls pass the human-readable case description as an option and are
therefore not executable differential commands as written. Their direct free-form features were
checked independently and are covered by the `fortran2023` and `structures` manifest cases.

## Manifest traceability

The checked-in 53-case manifest runner in `tests/manifest.rs` executes cases from
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
| Full Test026/Test027 parenthesis corpus and malformed-string boundary | `align_legacy_full`, `align_legacy_full_label0` |
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

COCO (`??`) and FYPP (`#:`) directive recognition is currently retained only for safe grouping and
branch continuation. They are not part of the first compatibility release's supported semantic
contract; CPP behavior is the supported preprocessor feature. Full COCO/FYPP behavior is deferred
until it has a fixture corpus and a separate compatibility decision.
