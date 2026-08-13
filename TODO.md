# Rust conversion TODO

Last audited against `RUST_CONVERSION_PLAN.md` and findent 4.3.7 on 2026-08-10.

## Audit snapshot — do not treat this as release-ready

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --all-targets`, and `cargo build --release` pass. The test suite currently has 81
  tests: 65 unit and 16 integration/property/semantic tests (2 compatibility, 1 manifest, 12
  property, and 1 semantic test).
- The retained three-fixture differential runner passes only after its one documented normalization:
  leading whitespace on `findentfix:` comments. Default trailing horizontal whitespace now matches
  the oracle. Strict comparison
  reports no other difference in those three fixtures, but this is a smoke test, not a compatibility
  corpus.
- The latest repeat of the release throughput check measured **3.19 M lines/s / 65.1 MB/s** on the
  mixed workload; the continuation-heavy and preprocessor-heavy workloads measured **3.71 M /
  70.2 MB/s** and **3.69 M / 53.2 MB/s**. Runs vary with container CPU availability. The stripped
  binary is **404,216 bytes** with a reported **6.0 ms average startup** in the latest check.
  A supplementary GNU-time run measured approximately **1,700 KiB** peak RSS on empty input, **2,200 KiB** on
  `equations.f90`, and **39,100 KiB** while formatting a 21.3 MB repeated-equations stream
  (about **1.9x** input) after owned-source and streaming-group reductions. The `<3x` RSS
  aspiration now passes for the representative large stream. Per the maintainer's direction,
  throughput is informational rather than a release blocker; keep the tool fast and avoid
  pathological regressions, but prioritize compatibility work over tuning.
- The checked-in manifest is a dependency-free **golden runner** with 53 traceable cases and
  provenance/category/support/normalization metadata. It is still not a complete compatibility
  corpus.
- GitHub Actions covers MSRV/stable checks, size, benchmark execution, target `cargo check`,
  deterministic fixture-prefix regression, CLI contracts, release startup/RSS reporting, actual
  `cargo package` verification, static musl/Windows artifact builds, persisted release metrics, and
  post-upload checksum verification. It still does not enforce a benchmark threshold or RSS budget;
  startup is informational per maintainer direction. All three target `cargo check --all-targets` jobs pass
  locally after installing the standard libraries; local x86_64 and aarch64 musl release builds are
  reproducible (the latter via the installed `rust-lld`) and the x86_64 artifact passes the
  equations/CLI audit. A local `cargo-xwin` check also proves reproducible Windows MSVC linking; a
  real CI run is still required for hosted artifact upload and hosted-runner RSS evidence.
- `format_to` writes directly to the caller's sink, but source assembly and classification still own
  many per-line/logical-statement allocations; `SourceBuffer::new(&[u8])` also clones the whole input.
- A 57-configuration free-form matrix over all 36 gitignored `fortran/*.f90` files completed
  successfully and Rust output was idempotent in every case. All configurations are now
  byte-exact against findent 4.3.7 except the one valid-literal `--ws_remred` defect, which
  appears in the two standalone whitespace rows and the two stress rows that enable it. No
  formatting configuration returned an error.

### Contract corrections discovered in this audit

- The oracle and `/opt/findent/test/test24.sh` treat bare `--ws_remred` as enabled, equivalent to
  `--ws_remred=1`. Current Rust behavior agrees, and Plan section 6 now records the corrected
  supported optional-value contract for `--align_paren` and `--ws_remred`.
- The stated default preservation rule has an exception for a labeled line: label padding following
  the digits can change under `label_left`. This is now explicit in the compatibility docs and
  covered by a label-aware preservation property; the wording "every byte outside leading
  whitespace" is no longer used as a blanket rule.
- COCO and FYPP are deferred from the first compatibility release. Keep their current safe grouping
  behavior, but do not advertise semantic support or spend release-critical work on them.

### Progress since the previous audit

#### Compatibility regressions found in the full local Fortran corpus (2026-08-10)

- **Resolved Rust bugs — 2026-08-10 full-corpus audit:** parenthesis alignment now models the
  oracle's first-item, trailing-ampersand, nested/empty-delimiter, continuation, and label-origin
  rules; `CONTAINS` restart replaces only the active indentation entry while retaining frames; and
  `-RR` skips replacement for an `ABSTRACT INTERFACE` frame. The reduced `compat_regressions` fixture
  covers all three behaviors and the 57 × 36 matrix is byte-exact for them.
- **Confirmed original-code bug — `--ws_remred`:** findent 4.3.7 treats a single-quoted literal
  following an alphanumeric token plus whitespace as non-string code (for example,
  `error stop '...  ...'`). Its `remred` heuristic uses the previous nonblank character to avoid a
  Hollerith-like quote, so it collapses spaces inside this valid Fortran character literal. Rust
  preserves the literal exactly, which follows the option's documented "outside strings" contract
  and avoids changing an error message. Keep this as an intentional, fixture-backed divergence;
  do not regress Rust to match the oracle's lexical error.

- The default release binary now matches findent 4.3.7 byte-for-byte on the full `equations.f90`
  input (3,289 lines / 133,190 bytes). `tools/check_equations.sh` verifies the expected output,
  idempotence, and—when installed—the oracle comparison. The generated `equations.f90.indented`
  in this workspace is current; the source/output remain local corpus files.
- Fixed a load-bearing classifier error where a `PROCEDURE(...) :: name` declaration opened a
  procedure frame. Added a regression fixture and manifest row; this was the cause of the original
  equations divergence at `CONTAINS`.
- Fixed query output so `--last-indent` and `--last-usable` emit only the requested number, not
  formatted source followed by the number.
- Fixed exact free-form OpenMP emission to discard stale body indentation after the `!$ ` sentinel,
  normalize documented near-miss comments like `!$\tfoo`, and made `--ws_remred` carry quote state
  across continued physical lines. Added focused tests and updated the lexical oracle golden.
- Unignored the shipped Fortran manifest fixtures so a clean checkout can run the manifest and
  semantic tests. The full suite is now 65 unit tests plus 16 integration/property/semantic tests.
- Added an oracle-matched construct matrix covering the supported branch, loop, selection, type,
  interface, procedure, and coarray-control families, plus CPP directives embedded in continued
  free-form statements and the `-Ia` start-indent case.
- Added oracle-backed CLI/layout rows for include-left, label-left, `-K`, continuation handling,
  and the default free-form layout. Added assembler unit coverage proving directive continuations
  remain non-Fortran groups while embedded comments/CPP lines remain in the surrounding logical
  statement.
- Matched the oracle's comma-prefixed procedure boundary and added a procedure-prefix matrix;
  comma-free prefixes still open frames, while declaration-style attribute lists remain opaque.
- Added explicit CLI alias/error tests and a manifest row for the documented unknown-option
  divergence (Rust exits 2; findent 4.3.7 silently ignores unknown flags).
- Captured the full free-form `--ws_remred` case. Well-formed strings match the oracle; the
  malformed continued-string case is documented as a conservative intentional divergence.
- Fixed raw-versus-clamped stack depth for inferred start indentation with `-M`; `CONTAINS` now
  matches the oracle under `-Ia -M5`, and restart mode has a checked-in fixture.
- Added explicit `-i-`/`--indent=none` and bare `--ws_remred` manifest rows, plus a CI/runtime CLI
  contract script covering fixed-form rejection, invalid options, query output, and broken pipes.
- Replaced one-line parenthesis detection with stateful nested-parenthesis tracking across physical
  continuation lines. Added reduced `Test026`/`Test027` fixtures, including label-left interaction.
- Added malformed explicit-`END` recovery that removes the nearest definition frame without losing an
  active construct, plus a reduced legacy fixture for that recovery boundary.
- Added the free-form critical/change-team controls from legacy test11 as a checked-in fixture and
  expanded the classifier matrix to cover every public structural family plus labels and names.
- Expanded properties to unknown-statement stability, case-insensitive indentation depth, and
  deterministic arbitrary-byte totality and non-ASCII comment/string transparency; CI now runs
  those checks through the fuzz-regression script.
- Added fixture-wide ASCII case mutation checks so structural indentation depth is stable across
  keyword casing, with the mutation included in the deterministic fuzz regression.
- Added direct stack-boundary tests for repeated underflow attempts, mismatched closes, max-indent
  clamping, raw-depth recovery, and empty-label operations.
- Added release verification for binary size, checksum reporting, startup measurement, optional RSS
  (including an explicit `FORFORMAT_TIME_BIN` override),
  CLI contracts, actual Cargo package verification, deterministic package-archive hashing, and
  complete tests over the unpacked package crate. When the local `equations.f90` corpus is present,
  the verifier also builds the unpacked release binary and runs the full equations check through it.
- Added an owned-source formatting entry point for stdin callers so the CLI reuses its input `Vec`
  instead of cloning it into `SourceBuffer`; the streaming equivalence property covers both APIs.
- Added a callback-based logical-group visitor so the formatter processes one group at a time rather
  than retaining the complete assembled corpus; the collected API remains for tests and callers.
  This reduced the representative RSS from 127,940 KiB to 39,100 KiB and restored the release
  throughput measurements above without changing oracle output.
- Added CI jobs that build/upload static `x86_64` and `aarch64` musl artifacts plus the Windows MSVC
  release artifact; all three target `cargo check --all-targets` jobs and the local `x86_64` musl
  release link now pass here. Local `rust-lld` aarch64-musl and `cargo-xwin` Windows MSVC release
  links are also reproducible; hosted artifact upload remains CI-only evidence.
- Verified the aarch64-musl release artifact with two clean `rust-lld` links: both are ELF64 AArch64,
  have no program interpreter, and hash to the same 434,320-byte binary.
- Verified two clean Windows MSVC release links through `cargo-xwin`; `/Brepro` plus final
  `/DEBUG:NONE` removes path-sensitive CodeView data and produces the same 212,480-byte PE binary.
- Added a dependent CI artifact-verification job that downloads every uploaded static/Windows
  artifact and runs its filename-bearing SHA256 sidecar; the release job also uploads its startup/
  RSS measurement output for hosted-runner review.
- Expanded totality/idempotence checks to every checked-in fixture and truncated prefix; added
  label-padding preservation, Hollerith reduction bypass, and direct emitter regressions for labels,
  alignment, replacement, comments, OpenMP, continuations, and whitespace reduction.
- Corrected raw-versus-visible stack depth for `-M` branch transitions, continuation policies,
  disabled OpenMP sentinels, and label-aware parenthesis alignment; a 609-case fixture/option
  differential sweep is now byte-exact against findent 4.3.7.
- Restored CPP, COCO, and FYPP branch event recognition for `#`, `??`, and `#:` directives,
  including compact `??if(foo)` spelling, while keeping full COCO/FYPP semantics out of the release
  contract.
- Added narrow legacy recovery for the `su broutine` editor typo and for comma-prefixed external
  procedure END fallback behavior, with focused classifier and manifest fixtures.
- Reran the retained free-form legacy shell rows against the release binary. Tests 11, 14--16,
  and 19--20, 24 have no remaining supported behavioral failures; the direct test26/test27 feature
  cases also pass. Fixed-form, relabel, wrapper/dependency, malformed-option, and documented
  preservation/normalization rows are classified in `docs/compatibility.md`.
- Added a checked-in per-construct long-option matrix covering the test16 indentation controls and
  wired it into the manifest, prefix properties, and compatibility traceability table.
- Added a broader malformed explicit-END matrix covering mismatched definition ends inside active
  constructs, compact END spellings, and later sibling recovery.
- Added a nested CPP alternate-branch fixture covering both `#else` paths and nested `#endif`
  restoration; it matches findent 4.3.7 byte-for-byte.
- Added a combined label-left/include-left/leading-ampersand/continuation-indent fixture to cover
  an option interaction not represented by single-option rows.

## Next-agent queue

Each work item needs a small reviewable change, a committed reduced fixture for every behavior it
changes, and the relevant targeted test plus `cargo fmt --check`, strict clippy, and the full test
suite. Do not change expected oracle output without recording the oracle command/version and a
reviewed reason.

### Wave 1 — evidence and isolated contracts (can run in parallel)

- [x] **A1: corpus and manifest owner** — Own `tests/manifests/`, `tests/fixtures/`,
  `tests/expected/`, `tests/manifest.rs`, `tools/capture_oracle.sh`, and the traceability section in
  `docs/compatibility.md`. The manifest now has 53 traceable cases covering core and lexical/OpenMP
  behavior, construct and procedure matrices, CPP continuation, CLI layout, END refactoring,
  queries, whitespace reduction, fixed-form rejection, unknown-option diagnostics, engine-state
  options, Fortran 2023 prefixes, nested legacy structures, labels, indent disabling, bare optional
  values, nested parenthesis alignment, critical/change-team controls, malformed-END recovery,
  per-construct long-option indentation, malformed-END recovery matrices, nested CPP branch
  snapshots, combined layout interactions, full Test026/Test027 alignment, shared-label DO across
  nested CPP, a consolidated legacy construct matrix, and the two legacy procedure-recovery
  boundaries. The reduced cases are byte-exact against the pinned
  4.3.7 oracle or carry an explicit divergence classification.
  Remaining legacy rows are either represented by the consolidated matrix or explicitly classified
  as fixed-form, relabel, dependency, editor-wrapper, malformed legacy CLI, or malformed-string
  preservation boundaries; keep those classifications explicit rather than silently omitting them.
  Add manifest fields for source test, oracle version/command, category, supported/excluded status,
  allowed normalization, args, stdout, stderr, and exit status. Classify every omitted case as
  fixed-form, relabeling, dependency extraction, editor wrapper, or another explicit non-goal.
  **Current evidence:** the checked-in 609-case option sweep is byte-exact, the per-construct long-option
  matrix is checked in, and the retained free-form rows
  from tests 11, 14--16, 19--20, 24, 26, and 27 pass. Remaining legacy differences are fixed-form
  rows, documented malformed-string preservation, malformed CLI token
  concatenation, relabeling, or wrapper/dependency behavior. The 2026-08-10 audit's reduced
  parenthesis, `CONTAINS`, abstract-interface, and valid-literal whitespace cases are checked in
  with oracle provenance and explicit divergence metadata.

- [x] **A2: scanner and assembler owner** — Own `src/source/` and focused source tests. Complete
  quote/doubled-quote, Hollerith, semicolon/comment, nested delimiter, leading `&`, embedded blank/
  comment/CPP line, and malformed-editor-buffer coverage. Verify that a preprocessor/directive
  continuation cannot accidentally absorb an ordinary code line and that CPP state is not skipped
  when directives occur near continued source. Keep byte spans valid and avoid a scanner regression
  for arbitrary non-UTF-8 comments/strings.
  Current evidence includes quote/Hollerith/comment scanning, continued strings, embedded
  comment/CPP lines, directive continuation grouping, leading ampersands, malformed-byte totality,
  and a broad fixture-prefix regression. The remaining matrix is focused on additional
  blank/comment/directive permutations and malformed editor buffers with valid span assertions.
  **Done:** focused malformed-editor, span-boundary, directive/source-boundary, quote/doubled-quote,
  Hollerith, and non-UTF-8 tests are green; retained oracle cases in this area are green or classified.

- [x] **A3: CLI and documentation-contract owner** — Own `src/cli.rs`, CLI tests, and
  `docs/migration.md`; coordinate plan wording changes with the maintainer. Exhaustively snapshot
  supported and rejected options, including attached/separated shorts, `-i` overloads, `-K`,
  `-Rr`/`-RR`, `--` termination, `_`/`-` aliases, optional long values, exit status, stderr, empty
  input, and broken pipes. Resolve the bare optional-value wording using oracle evidence (bare
  `--ws_remred` is enabled); do not silently assign semantics to untested non-zero numeric values.
  The manifest now covers bare optional `--ws_remred` and indent disabling; the CI CLI contract
  script covers fixed-form/invalid-option status, query output, help/version output, and broken
  pipes. Unit tests cover attached/separated values, aliases, optional values, and `--` termination.
  The exhaustive spelling matrix now covers every documented long-option family, attached/separated
  overload, aliases, optional values, missing values, rejection diagnostics, and `--` termination.
  Keep migration wording tied to fixture-backed behavior rather than mere parser acceptance.
  **Done:** every supported CLI family has a parser test and the runtime contract script checks
  status, stderr, queries, help/version, and broken pipes.

### Wave 2 — formatter behavior (start only after Wave 1 fixtures establish expected results)

- [x] **B1: classifier owner** — Own `src/classify/`. Give every supported `StatementKind` positive,
  negative, mixed-case, whitespace, label-bearing, construct-name, and malformed-input tests. Verify
  procedures/module procedures, interfaces, types, legacy `STRUCTURE`/`UNION`/`MAP`, `SELECT TYPE`/
  `RANK`, `WHERE`, `FORALL`, `ASSOCIATE`, `BLOCK`, `CRITICAL`, `CHANGE TEAM`, `ENUM`, and all end/
  branch forms. Preserve assignment-first behavior and `findentfix:p-on`/`p-off` inertness.
  Current evidence includes all construct families in the construct/advanced/legacy fixtures,
  procedure declarations and prefixes, assignment-first negatives, compact END forms, the narrow
  `su broutine` recovery, and a table-driven positive spelling for every public structural family.
  The negative/mixed-case/label/name matrix now covers assignment lookalikes, compact-end prefixes,
  malformed numeric labels, and malformed construct names; the malformed digit-prefix case also has
  an engine-level assertion. **Done:** classifier coverage satisfies the Plan gate M2 within the
  conservative malformed-input boundary.

- [x] **B2: engine and preprocessor owner** — Own `src/format/engine.rs`, `stack.rs`, and
  `preprocessor.rs`. Validate nested and shared-label `DO` closure, named loops, explicit/malformed
  ends, `CONTAINS` restart mode, start-indent/`-Ia`, max-indent, query results, and CPP alternate
  branch snapshots including `#endif` after `#else`. The continued-source-around-CPP, `-Ia`, `-M`,
  `CONTAINS` restart, procedure-prefix, OpenMP continuation, and named/shared-label closure rows now
  pass. A malformed-END recovery matrix and nested CPP `#else`/`#endif` snapshot are now
  fixture-backed; remaining work is labeled closure variants around mixed directive/source groups.
  **Current evidence:** raw/clamped max-indent behavior, CPP/COCO/FYPP event snapshots, OpenMP
  disabled mode, continuation-none, `CONTAINS` restart, query output, procedure recovery, and
  malformed-END recovery and nested CPP branch restoration are fixture-backed; the checked-in
  option sweep is clean. **Done when:** all structural families and
  query cases imported by A1 match the oracle or have a reviewed documented boundary; malformed
  input cannot corrupt later indentation state. **Done:** `CONTAINS` restart preserves definition
  frames and has a direct stack regression plus the reduced full-corpus oracle fixture; the full
  option matrix remains clean.

- [x] **B3: emitter and transformation owner** — Own `src/format/emitter.rs`, `continuation.rs`,
  and `src/transform/`. Add direct emitter tests independent of the engine for label positioning,
  label padding, include-left, comments, blanks, exact/near-miss OpenMP sentinels, continuation
  indentation, leading ampersands, parenthesis alignment, mixed terminators, `--ws_remred`, and END
  refactoring. Make the preservation boundary precise: default mode may replace leading indentation,
  trim trailing horizontal whitespace, and change documented label padding; it must retain other
  line-body bytes. Explicit transforms need their own byte-level contract. Test Hollerith bypass for
  both whitespace reduction and alignment.
  Direct emitter coverage now covers labels, comments, OpenMP boundaries, continuations,
  parenthesis-targeted alignment, replacement, and whitespace reduction; the property suite covers
  the label-padding exception and Hollerith bypass. The full `Test031` fixture is present and its
  malformed-string difference is an approved intentional divergence. The combined
  `cli_layout_combo` fixture covers include/label/continuation interaction, and direct emitter
  tests cover mixed terminators and leading-ampersand policy. **Current evidence:** the checked-in
  609-case option sweep is clean and direct emitter tests cover the main policies. **Done:**
  parenthesis alignment and abstract-interface END refactoring match the oracle across the full
  corpus; `ws_remred_valid_literal` records the reviewed valid-string oracle defect.

### Wave 3 — hardening and release gates

- [x] **C1: properties and fuzz owner** — Expand properties for totality, idempotence, preservation
  (including label exception), stack safety, unknown-statement stability, case tolerance, span
  validity, and arbitrary-byte transparency. The five fuzz targets exist; seed them with all
  fixtures and truncated editor buffers, add a reproducible short fuzz-regression command, and run
  it in CI without making normal developers require `cargo-fuzz`. The deterministic regression now
  exercises every checked-in fixture and five truncated prefixes through assembly and formatting;
  it also asserts label-aware preservation and Hollerith safety. Current properties additionally
  cover unknown-statement stability, case-insensitive indentation depth, streaming equivalence,
  deterministic arbitrary-byte totality, and non-ASCII transparency. The required CI regression
  runs the prefix, arbitrary byte, non-ASCII transparency, unknown-statement, case-mutation,
  source/logical-span, and stack-boundary checks. **Done:** the deterministic regression is the
  required CI gate; deeper `cargo-fuzz` campaigns remain optional maintenance work.

- [x] **C2: performance owner** — Profile the release benchmark before changing architecture. Keep
  the current whole-input design unless profiling identifies a concrete regression; speed is
  informational and compatibility has priority. If optimizing, reduce logical-statement,
  classifier word/payload, and grouping allocations as separate reviewable changes. The benchmark
  now includes mixed, continuation-heavy, and preprocessor-heavy corpora. Current evidence is 3.19 M lines/s / 65.1
  MB/s mixed, 3.71 M / 70.2 MB/s continuation-heavy, and 3.69 M / 53.2 MB/s preprocessor-heavy
  on the latest repeat;
  the binary is 404,216 bytes with approximately 6.0 ms average startup in the latest container run;
  speed is informational per maintainer direction. Supplementary local GNU-time evidence is approximately 1,700
  KiB empty, 2,200 KiB for `equations.f90`, and 39,100 KiB for a 21.3 MB repeated-equations stream
  (about 1.9x input) after the owned-source and streaming-group reductions. The size/RSS budgets pass
  locally; the startup shortfall is recorded rather than hidden, and hosted-runner confirmation remains.

- [ ] **C3: CI and release owner** — Keep existing MSRV/stable, fmt, clippy, tests, size, target
  checks, deterministic fuzz regression, CLI contract, startup/RSS reporting, semantic `gfortran`
  smoke when available, deterministic package verification, and the static musl/Windows artifact
  jobs. Static and Windows jobs now compare hashes from two clean target builds and upload checksum
  sidecars with the artifacts, and a dependent job verifies those sidecars after upload. Local target
  checks and reproducible x86_64/aarch64 musl/Windows artifacts pass; remaining work is to inspect
  one hosted artifact/metrics run and confirm the hosted RSS measurement.

## Release gates still open

- [x] M1 lexical/assembly corpus is complete and arbitrary malformed bytes are total.
- [x] M2 each supported statement kind has a complete recognizer matrix.
- [x] M3 checked-in structural, emitter, CLI, and query fixture matrices are green.
- [x] M4 every in-scope oracle mismatch is byte-exact, intentionally documented per fixture, or
  explicitly unsupported; the 53-case manifest and 57 × 36 full-corpus matrix cover the retained
  legacy corpus. The only remaining in-scope difference is the reviewed valid-literal
  `ws_remred` oracle defect, documented in `docs/compatibility.md` and the manifest.
- [ ] M5 properties/fuzz regression, semantic compilation, size/RSS release budgets, and artifact
  checks are enforced in CI; throughput and startup are informational measurements. Local
  properties/fuzz regression, semantic smoke, size, all-target checks, x86_64-musl reproducible
  artifact evidence, complete tests over the unpacked package crate, and the unpacked release
  package's exact `equations.f90` check are complete. The remaining hosted evidence is artifact
  upload and hosted RSS. Local RSS is measured and the representative repeated-stream result is
  within <3x; hosted confirmation remains open.
- [ ] Only after M4/M5: update `RUST_CONVERSION_PLAN.md` from `Status: proposed` to an actively
  scheduled/completed status. Do not declare a compatibility release before then.

## Completed foundation — do not redo

- [x] Standalone Rust crate, byte-oriented source buffer, free-form stdin/stdout CLI, fixed-form
  rejection, licence/attribution, parser-strategy ADR, and release profile.
- [x] Basic physical-line handling, continuation/semicolon grouping, findentfix recognition,
  assignment-first core classification, typed indentation stack, labeled-DO bookkeeping, CPP branch
  snapshots, and conservative unknown fallback.
- [x] Initial `format_to` direct-sink API, END/whitespace transformations, last-indent/last-usable
  API fields, semantic smoke test, initial golden/property tests, benchmark executable, CI workflow,
  and five `cargo-fuzz` target definitions.
- [x] Intentional first-release exclusions recorded: fixed form, relabeling, dependency extraction,
  editor payloads, `FINDENT_FLAGS`, and full COCO/FYPP semantics.
