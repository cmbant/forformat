# Free-Form Findent Rust Conversion Plan

Status: **completed and superseded — historical record.** This is the plan the indent-only port was
executed against; it is kept for the rationale behind decisions the code still encodes, and for the
`§` references that source comments make to it. It is not a work queue. Current design lives in
[`../design.md`](../design.md); the compatibility boundary lives in
[`../compatibility.md`](../compatibility.md).

Baseline: findent 4.3.7 in `/opt/findent`, BSD-3-Clause, © 2015–2025 Willem Vermin
Target: a fast, standalone, free-form-only Rust formatter with intentional compatibility boundaries
Execution model: coordinated AI agents working through isolated, reviewable work packages

## 1. Objective

Reimplement the useful free-form behavior of findent in safe, maintainable Rust while preserving its
successful core design:

1. Assemble physical free-form lines into logical statements.
2. Perform quote-, comment-, continuation-, and parenthesis-aware lexical scanning.
3. Classify only the statement structure needed for indentation and selected transformations.
4. Maintain construct, label, and preprocessor-branch state.
5. Re-emit the original source with leading indentation changed and all other text preserved unless
   an explicit transformation is enabled.

Two non-negotiable product properties, in addition to compatibility:

- **Standalone.** A single statically-linkable executable with no C toolchain, no generated-code
  build step, and no runtime data files.

This is not a full Fortran parser project. The production classifier is handwritten Rust.

## 2. Firm product boundaries

### In scope for the compatibility release

- Free-form Fortran input and output only.
- Input from stdin and output to stdout.
- Logical statements continued with `&`.
- Blank lines, comments, statement labels, construct names, semicolon-separated statements, strings,
  legacy Hollerith constants, and dot operators.
- Structural indentation for procedures, modules, submodules, interfaces, types, `IF`, `DO`,
  `SELECT`, `WHERE`, `FORALL`, `ASSOCIATE`, `BLOCK`, `CRITICAL`, `CHANGE TEAM`, `ENUM`, and related
  branch/end statements.
- Shared-label/labeled `DO` bookkeeping, including multiple loops ending at one label.
- Free-form OpenMP conditional handling. The free-form sentinel findent recognizes is exactly
  `!$ ` — including the trailing space. Test the near-misses (`!$`, `!$omp`, `!$	`).
- `! findentfix: <statement>` comment directives (see 2.1) — the documented user escape hatch.
- CPP branch-state snapshots, including the `#endif`-with-`#else` distinction (see 5.5). COCO and
  FYPP support is decided at the end of Phase 2.
- Preprocessor line continuation: CPP directive lines continue with a trailing `\`; COCO and FYPP
  directive lines continue with a trailing `&`. The generation type is latched from the first line
  of each directive sequence (`findentclass.cpp:105-122`).
- Configurable construct and continuation indentation, including `-C-` /
  `--indent_contains=restart`, which is a *mode*, not a negative indent.
- Continuation-leading `&` handling and optional parenthesis alignment.
- Statement-label placement and include indentation.
- Maximum indent (`-M`, default 100, `0` = unlimited).
- Optional `END` refactoring.
- `--last-indent`, `--last-usable`, and redundant-whitespace reduction if they pass their individual
  compatibility gates.
- Unknown or incomplete statements must be preserved without panicking.

### 2.1 `findentfix` directives

Verified against the oracle:

```console
$ printf 'program p\nx=1\n!  findentfix: do\ny=2\nend do\nend program\n' | findent -ifree
program p
   x=1
!  findentfix: do
      y=2
   end do
end program
```

A comment matching `{blank}*!{blank}*findentfix:` is parsed as a Fortran statement and fed to the
indentation engine while the comment line itself is emitted verbatim. This is how users repair
findent's own misclassifications, so it is load-bearing, not decorative. Two debug toggles share the
prefix and must at minimum be recognized and ignored rather than parsed as Fortran:
`findentfix:p-on` and `findentfix:p-off`. See `lexer.l:417-437`, `fortran.cpp:406,516`,
`fortran.h:238`.

### Explicitly out of scope

- Fixed-form input and output; automatic fixed/free detection; free↔fixed conversion.
- Column-6 continuation rules, fixed-form labels/comments/debug lines, tab-column conventions, and
  fixed OpenMP sentinels.
- `--continuation`, `-ifixed`, `-ofixed`, `-iauto`, and `--query-fix-free` behavior.
- ESOPE/`SEGMENT` support (`--indent_segment`).
- `wfindent`, `--safe`, editor payload generation (`--vim_*`, `--gedit_*`, `--emacs_*`),
  self-reproducing archives (`--selfrep`), embedded README/changelog output, and `--makefdeps`.
- A full AST, name resolution, type checking, semantic validation, or compiler-grade diagnostics.

Unsupported fixed-form requests must fail with exit status 2 and a concise message; they must never
be silently accepted. `-ifree`, `-ofree`, and `-osame` may be accepted as compatibility no-ops.

This is a **deliberate divergence**: findent's help text states "errors are silently ignored", and
the oracle confirms it — `findent -ifree --no-such-flag` formats normally and exits 0, and
`-ifixed` on free-form input is accepted. The contract must therefore also decide the *unknown flag*
case independently. Recommended: unknown flags are a hard error (exit 2), because silent acceptance
of a typo'd flag is the failure mode users report against findent today. Record both divergences in
`docs/compatibility.md`.

### Post-MVP options

Separate milestones that must not complicate the formatter core:

- Relabeling and relabel reports (`relabel.cpp` is 1,021 lines on its own).
- Dependency extraction (`--deps`).
- COCO and FYPP preprocessing if not included in the first compatibility release.
- `-L` / `--input_line_length` legacy input-line truncation, including the `g` (gfortran tab
  convention) suffix.

## 3. Baseline and source of truth

### 3.1 Size baseline

Measured against the 4.3.7 tree:

| Scope | Lines |
|---|---:|
| All handwritten `.cpp` + `.h` (excludes generated lexer/parser) | 9,031 |
| — of which explicitly out of scope (`fixed.cpp`, `relabel.cpp`, `docs.cpp`, `selfrep.cpp`, `makeman.cpp`) | 2,342 |
| Free-form-relevant `.cpp` subset | 4,473 |
| — formatter core proper (`fortran` 1,208, `free` 736, `line_prep` 606, `findentclass` 304, `findentrun` 208, `fortranline` 141, `pre_analyzer` 77, `prop` 51) | 3,331 |
| — CLI (`flags.cpp`) | 704 |
| — utility (`functions.cpp`) | 324 |
| Flex rules (`lexer.l`) | 725 |
| Bison grammar (`parser.y`) | 627 |
| Generated C++ (`lexer.cpp`, `parser.cpp` — **not** a porting target) | 8,648 |

**The porting target is roughly 3,300 lines of formatter core plus ~1,350 lines of lexer/grammar
rules to re-express as recognizers.** This is a small project, and the phase and agent structure in
Section 7 is sized accordingly.

### 3.2 Oracle

The behavioral oracle is `/opt/findent/src/findent` version 4.3.7, always invoked with `-ifree` and
with `FINDENT_FLAGS` unset. The C++ executable is used to create and investigate goldens, but
ordinary Rust CI must not require it.

Behavioral priority, highest first:

1. The committed compatibility contract in this document.
2. Curated golden fixtures generated from findent 4.3.7.
3. Existing free-form tests under `/opt/findent/test`.
4. Current source behavior.
5. Documentation where it conflicts with none of the above.

### 3.3 Licence and attribution — resolve in Phase 0

findent is **BSD-3-Clause**, © 2015–2025 Willem Vermin (`/opt/findent/COPYING`, `AUTHORS`). This is
permissive: porting logic and reusing fixtures is allowed, provided the copyright notice and the
three-clause disclaimer are retained in source and in binary distributions, and provided the
author's name is not used to endorse the derivative.

Settle this in Phase 0, not at release: translation begins in Phase 1, so the `NOTICE` /
`LICENSE-THIRD-PARTY` text and the README attribution paragraph must be carried from the first
commit.

## 4. Parser and crate decision

### 4.1 Decision: handwritten, no parser generator

Use a handwritten scanner and ordered statement classifier. Do not parse lexical structure with
regular expressions alone. Do not adopt `tree-sitter-fortran`, and do not spend an agent-week
spiking it — the project's own criteria already determine the outcome:

- It requires a generated-C build step and a C toolchain, which directly contradicts the standalone
  requirement in Section 1 and adds a cross-compilation burden for the musl and Windows targets.
- It parses expressions, which this project explicitly does not want, and would add ~1 MB to a
  binary targeted at under 2 MB.
- It cannot serve as an oracle for indentation decisions, only for statement boundaries, so it
  would not discharge testing work either.
- Editor-buffer input — half-written statements mid-keystroke — is the primary use case, and error
  recovery in a generated grammar is harder to constrain than in ordered recognizers.

Record this as `docs/adr/0001-parser-strategy.md` with the reasoning above. If a later agent
measures classifier complexity that genuinely justifies a parser-combinator crate, `winnow` may be
proposed then. LALRPOP, Chumsky, and nom are not planned dependencies.

### 4.2 Classifier output

```rust
struct StatementInfo {
    kind: StatementKind,
    class: StatementClass,
    construct_name: Option<Span>,
    entity_name: Option<Span>,
    statement_label: Option<LabelRef>,
    referenced_labels: SmallVec<[LabelRef; 3]>,
    payload: StatementPayload,
}

enum StatementClass {
    Definition,
    Executable,
    EndDefinition,
    Neutral,
}
```

Expressions and most statement tails remain opaque, balanced spans. Classification must try
assignment/identifier interpretations before keyword interpretations so keywords used as identifiers
do not create false structural matches. The oracle confirms this is required in practice:

```fortran
integer :: if, do, type
if = 1          ! assignment, not a construct
do = 2          ! assignment, not a loop
if (if > 0) then
   do = do + 1
end if
```

findent 4.3.7 indents this correctly; a keyword-first classifier would not.

### 4.3 Dependencies

Keep the tree shallow — every dependency is startup time, binary size, and audit surface:

- `lexopt` or a handwritten argument parser for the CLI. **Not `clap`.** `clap` costs roughly
  300–500 KB and measurable startup time, and Section 6 already requires a bespoke compatibility
  parser for findent's overloaded `-i`, which is most of what `clap` would have provided.
- `memchr` for scanning (SIMD line/quote/comment splitting; this is the hot loop).
- `bstr` only if byte-string ergonomics prove worth it over plain `&[u8]` helpers.
- `thiserror` for library error types.
- `proptest` and `cargo-fuzz` for properties and fuzz targets (dev-dependencies only).
- `insta` only if ordinary checked-in fixture files prove less readable (dev-dependency only).

Pin the MSRV in `Cargo.toml` and CI. Gate dependency additions on the binary-size and startup
measurements in Section 4.5, not on taste.

### 4.4 Text representation: bytes, not `String`

**The core operates on `&[u8]`, not `str`.**

findent is byte-oriented and passes arbitrary bytes through unchanged:

```console
$ printf 'program p\n! caf\xe9 comment\nx=1\nend program\n' | findent -ifree | od -c
0000000   p r o g r a m   p \n !   c a f 351
0000020     c o m m e n t \n       x = 1 \n
0000040   e n d   p r o g r a m \n
$ echo $?
0
```

Latin-1 comments are extremely common in older Fortran codebases. Rejecting or lossily rewriting
them would make the tool unusable on exactly the corpus that most needs reindenting. A byte core is
strictly better on three axes:

1. **Correctness/compatibility** — byte-for-byte passthrough matches the oracle, and comments and
   string literals in any single-byte encoding survive.
2. **Safety** — it deletes the entire "never slice at an unvalidated UTF-8 boundary" risk class,
   along with the corresponding property test.
3. **Speed** — no UTF-8 validation pass over the input, and `memchr` operates natively on bytes.

All syntactically significant characters in Fortran (`!`, `&`, `;`, `'`, `"`, `(`, `)`, `[`, `]`,
`.`, digits, and the keyword alphabet) are ASCII, and UTF-8 is self-synchronizing, so a byte scanner
cannot mistake a multi-byte sequence for a delimiter. Case-insensitive keyword matching uses
`eq_ignore_ascii_case`, which is correct precisely because keywords are ASCII.

Public API convenience wrappers may accept and return `&str` where the caller already has valid
UTF-8, but they must be thin adapters over the byte core.

### 4.5 Performance and binary budget

Targets are set now, from the measured C++ baseline, so the architecture is designed toward them
rather than tuned at the end.

**Measured baseline** (findent 4.3.7, this container, 220,004-line / 3.49 MB generated free-form
file with continuations, comments containing `!`/`&`/`;`/quotes, and nested constructs):

| Metric | findent 4.3.7 |
|---|---|
| Throughput | 0.75 s wall → **~293k lines/s, ~4.6 MB/s**, single-threaded |
| Startup (empty input, 200 invocations) | ~2.0 ms per invocation |
| Binary size | 10.3 MB (2.38 MB of it embedded `.inc` payloads that are out of scope) |

**Targets for the Rust build**, enforced by a CI benchmark and a size check:

| Metric | Target | Rationale |
|---|---|---|
| Throughput | Informational aspiration: ≥ 3 M lines/s | Keep the implementation comfortably fast and record measurements, but throughput is not a release blocker; correctness and compatibility take priority. |
| Startup | Informational aspiration: < 1 ms | Editors invoke the binary per reindent; record this latency, but it is not a release blocker when compatibility and correctness take priority. |
| Stripped binary | < 2 MB | No embedded payloads, no generated parser, no C runtime data. |
| Peak RSS | < 3× input size | Whole-input buffering is fine; per-line `String` copies are not. |

**Design consequences:**

- Read the entire input in one `read_to_end` into a single `Vec<u8>`. Do not read line by line.
  findent itself streams via `getline` (`findentrun.cpp:48`), but `-Ia`, `--last-indent`, and
  `--last-usable` all need whole-input context anyway, and one buffer is both simpler and faster.
- `PhysicalLine` must not own a `String` — that is one heap allocation per input line, 220,000 of
  them on the benchmark file. Use ranges into the single input buffer (see 5.1).
- `LogicalGroup` must not clone physical lines into a `Vec<PhysicalLine>`. Store a line-index range.
- Write output through a single `BufWriter` with a large buffer; emit indentation from a shared
  static blank run rather than building pad strings.
- Release profile: `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.
- Standalone: primary target `x86_64-unknown-linux-musl` (fully static, no libc dependency), plus
  `aarch64-unknown-linux-musl` and `x86_64-pc-windows-msvc`. No `build.rs` that invokes a C
  compiler — this is what rejecting a parser generator buys.
- Do not parallelize. The formatter is inherently sequential (indent state threads through the
  file), the workload is I/O-shaped at these speeds, and threads would cost more startup than they
  save.

## 5. Target architecture

Library-first:

```text
src/
  lib.rs
  config.rs
  error.rs
  source/
    buffer.rs          # owned input bytes + line index
    physical_line.rs
    logical_statement.rs
    scanner.rs
  classify/
    mod.rs
    recognizers.rs
    statement.rs
    findentfix.rs
  format/
    engine.rs
    stack.rs
    preprocessor.rs
    emitter.rs
    continuation.rs
  transform/
    refactor_end.rs
    whitespace.rs
  cli.rs
  main.rs
tests/
  fixtures/
  manifests/
  differential.rs
  idempotence.rs
  preservation.rs
benches/
  throughput.rs
tools/
  capture_oracle.sh
  classify_diff.rs
fuzz/
```

### 5.1 Physical source model

One owned buffer; every line is a view into it.

```rust
struct SourceBuffer {
    bytes: Vec<u8>,          // the entire input, verbatim
    lines: Vec<PhysicalLine>,
}

struct PhysicalLine {
    span: Range<u32>,        // content, excluding the newline
    newline: Newline,        // Lf | CrLf | None (last line, no terminator)
    kind: PhysicalLineKind,
    code_span: Range<u32>,
    comment_span: Option<Range<u32>>,
    omp: Option<OpenMpSentinel>,
}
```

`u32` offsets halve the struct size against `usize`; reject inputs over 4 GiB with a clear error.
No `String`, no per-line allocation.

**Newline policy**, resolved against the oracle:

- Line terminators are preserved per line, exactly as read. findent does not normalize; CRLF input
  produces CRLF output, verified byte-for-byte with `od -c`. A file with mixed terminators keeps its
  mixture.
- A final line with no terminator gets one added on output. Verified: `printf 'program p\nx=1'`
  emits `x=1\n`.
- A lone `\r` is not a line terminator.

**Encoding policy:** arbitrary bytes pass through unchanged (Section 4.4). There is no error case
for encoding.

### 5.2 Shared lexical scanner

One scanner provides the facts used by assembly, classification, transformation, and emission:

- Quoted strings with doubled-quote escaping, both `'` and `"`.
- Comments beginning with `!` only outside strings.
- Semicolons only outside strings and Hollerith constants.
- Nested `()` and `[]` spans.
- Leading numeric statement labels.
- Dot operators.
- Free-form continuation markers and leading continuation `&`.
- Hollerith constants, retained for compatibility even though they are legacy syntax. Both
  `--ws_remred` and `--align_paren` are documented as *ignored* for statements containing a
  Hollerith; the scanner must expose that fact to those passes.
- Preprocessor directive lines, their generation type (CPP / COCO / FYPP), and their continuation
  character (`\` for CPP, `&` for COCO/FYPP).
- `findentfix:` comment directives and the `p-on`/`p-off` toggles.
- Source-span mapping from normalized tokens back to physical text.

Do not independently reimplement quote/comment scanning in multiple modules. This is the hot loop —
use `memchr`/`memchr2` to jump between candidate delimiters rather than examining every byte.

### 5.3 Logical statement assembler

```rust
struct LogicalGroup {
    lines: Range<u32>,              // index range into SourceBuffer::lines
    statements: SmallVec<[LogicalStatement; 1]>,
    directives: SmallVec<[DirectiveEvent; 0]>,
}
```

The assembler must preserve comment, blank, and preprocessor lines embedded in continuations. A
logical group may contain multiple semicolon-separated statements; all statements update formatter
state, but the physical group is emitted once. Verified oracle behavior:

```console
$ printf 'program p\nif(a)then;x=1;y=2;end if\nend program\n' | findent -ifree
program p
   if(a)then;x=1;y=2;end if
end program
```

The `IF`/`END IF` pair inside one physical line nets to zero indent change and the line is emitted
once, unmodified apart from leading whitespace.

The assembler must also join continued *preprocessor* directives: a CPP line ending in `\` continues
to the next line, a COCO or FYPP line ending in `&` does. The generation type is latched at the
first line of the sequence and applies to the whole sequence.

### 5.4 Ordered classifier

Small recognizers over scanner tokens. Each returns `NoMatch`, `Match(StatementInfo)`, or
`MalformedKnownStatement`. **Order is part of the compatibility contract:**

1. `findentfix:` directive extraction (the payload then re-enters at step 2).
2. Leading label and construct-name handling.
3. Assignment/pointer-assignment ambiguity check.
4. Definition and end-definition statements.
5. Opening constructs.
6. Branch/middle statements.
7. Closing constructs.
8. Label-bearing control and I/O statements where relabel metadata matters.
9. Includes, `USE`, modules, and submodules where dependency metadata matters.
10. `Unknown` fallback.

Keep expressions opaque. Store balanced argument/control spans only when a later feature needs their
original content.

**Scope discipline:** steps 8 and 9 collect metadata whose only consumers (`--relabel`, `--deps`)
are post-MVP. Building it now means the classifier has to keep it correct for free, for a feature
that may never land. Keep `statement_label` and `referenced_labels` — they are cheap and
`--last-usable` touches labels anyway — and drop `--deps` metadata until the feature is scheduled.

### 5.5 Indentation engine

The engine consumes statement events and owns no output text. Its state:

- Construct/indent stack with typed frames.
- Routine/type/interface metadata stack for optional `END` refactoring.
- Labeled-`DO` stack, including the shared-label case:

  ```console
  $ printf 'program p\ndo 100 i=1,10\ndo 100 j=1,10\n100 continue\nend program\n' | findent -ifree
  program p
     do 100 i=1,10
        do 100 j=1,10
  100 continue
  end program
  ```

  One label closes both loops, and the label line goes to column 0 under the free-form default
  `--label_left=1`.
- Previous significant statement for delayed module-procedure handling.
- First-code-line and starting-indent state (`-Ia`).
- Preprocessor branch snapshots containing all structural state.
- Max-indent clamp (`-M`, default 100, `0` = unlimited).

Model transitions as explicit functions and unit-test them without the emitter. Unexpected closing
statements must recover conservatively: never underflow, never panic, never manufacture negative
indentation.

**Preprocessor rule.** findent's `Pre_analyzer` (`pre_analyzer.cpp:38-77`) keeps an `ifelse_stack`
of flags and distinguishes two endif outcomes:

- `#if` / `#ifdef` / `#ifndef` → push `0`, snapshot all structural state.
- `#elif` → restore branch-entry state; stack unchanged.
- `#else` → restore branch-entry state, and set the top flag to `1`.
- `#endif` → pop; the result is `PRE_ENDIFE` if the popped flag was `1` (this `#if` had an `#else`),
  otherwise `PRE_ENDIF`. The engine's merge behavior differs between the two cases.

An `#endif` with an empty stack must be tolerated (findent's `if (!ifelse_stack.empty())` guards).
Verified oracle behavior for the alternate-branch case:

```console
$ printf 'program p\n#if defined(X)\nif (a) then\n#else\nif (b) then\n#endif\nx=1\nend if\nend program\n' | findent -ifree
program p
#if defined(X)
   if (a) then
#else
   if (b) then
#endif
      x=1
   end if
end program
```

Both branches open an `IF` at the same level, and the state does not double-count at `#endif`.

This logic remains necessary even in a free-form-only implementation.

### 5.6 Pure emitter

```rust
fn emit_group(
    buf: &SourceBuffer,
    group: &LogicalGroup,
    decisions: &[IndentDecision],
    config: &FormatConfig,
    out: &mut impl Write,
) -> Result<(), FormatError>;
```

Writing into a sink rather than returning owned strings avoids allocating a vector of `String` per
group.

The default emitter changes leading indentation only. It preserves keyword case, internal spacing,
comments, strings, directive spelling, and physical continuation layout. Labels, OpenMP sentinels,
leading `&`, continuation indentation, and parenthesis alignment are explicit policies rather than
incidental branches.

Transformations such as whitespace reduction and `END` refactoring run as named, separately tested
passes.

### 5.7 Public library API

```rust
pub fn format_source(source: &[u8], config: &FormatConfig)
    -> Result<FormatResult, FormatError>;

pub fn format_to(source: &[u8], config: &FormatConfig, out: &mut impl Write)
    -> Result<FormatMeta, FormatError>;
```

`FormatResult` contains formatted bytes plus optional diagnostics and last-indent/last-usable
metadata. `format_to` is the allocation-light path the CLI uses. CLI behavior wraps this API.

## 6. CLI compatibility contract

Canonical Rust long options plus selected findent aliases. Initial supported surface:

- Global indent, no-indent (`-i-`), start-indent (`-I<n>`), automatic start-indent (`-Ia`).
- Per-construct indent overrides: `-a -b -d -f -E -F -j -m -r -s -t -w -x`, `--indent_changeteam`.
- Case/contains/entry negative-indent controls: `-c -C -e`.
- `-C-` / `--indent_contains=restart` — a mode, not a number.
- Continuation indentation (`-k<n>`, `-k-`, `-kd`), leading-ampersand indentation (`-K`), and
  parenthesis alignment (`--align_paren[=<n>]`).
- Label position (`-l<n>`, default 1 for free form), include position (`--include_left`),
  maximum indent (`-M<n>`, default 100), and free OpenMP handling (`--openmp=<n>`).
- `END` refactoring (`-Rr`, `-RR`) and redundant-whitespace reduction (`--ws_remred[=<n>]`).
- Last-indent (`-lastindent`) and last-usable (`-lastusable`) queries.
- Help and version.

Legacy `-i` is overloaded: `-i3`, `-i-`, `-ifree`, `-ifixed`, and `-iauto` have different meanings.
Implement this through one compatibility value parser, with focused tests for attached values and
error diagnostics. Do not define competing arguments for the same short option.

Legacy optional-value flags require explicit parsing tests. For the supported formatting flags,
`--align_paren` and `--ws_remred` are enabled when bare, disabled with `=0`, and enabled with
`=1`; this matches findent 4.3.7 (including the bare `--ws_remred` behavior verified by test24).
`--query_relabel` remains out of scope and must not acquire partial semantics. Prefer
`--option=value` so the next positional token is never consumed accidentally.

Long options accept `_` or `-` interchangeably (`--indent_do` ≡ `--indent-do`); test both spellings
of at least one flag per family.

The CLI must define and test:

- Exit statuses (0 success; 2 unsupported fixed-form request; 2 unknown flag per Section 2).
- stdout/stderr separation.
- Broken-pipe behavior — exit quietly rather than panicking on `EPIPE`, which matters because
  editors close the pipe on cancel.
- Empty input.
- The newline policy in 5.1.
- Environment handling. findent parses `FINDENT_FLAGS` before command-line flags and marks a subset
  of flags `[NO_ENV]`. Recommended: do **not** read `FINDENT_FLAGS`, and document the omission — it
  is a hidden-global-state footgun, and every oracle capture already unsets it. If compatibility is
  later required, it must reimplement the `[NO_ENV]` filter, not just prepend the string.

## 7. Delivery phases and agent work graph

Six phases, roughly ten agent-packages, sized for a ~3,300-line core. No agent begins a package
until its declared inputs are merged. Agents must not share ownership of the same production file
during a parallel wave.

### Phase 0 — Contract and corpus

Two agents in parallel:

- **A0 Contract agent:** freeze the supported-flag matrix, exit statuses, the newline and encoding
  policies from 5.1 and 4.4, the divergence list (fixed-form rejection, unknown-flag rejection, no
  `FINDENT_FLAGS`), the licence/attribution text, and ADR 0001 recording the parser decision.
- **A1 Oracle agent:** manifest-driven differential harness; pin oracle version, command, locale,
  and environment; extract free-form fixtures from the legacy suite; build the feature/test
  traceability table.

Deliverables: `docs/compatibility.md`, `docs/adr/0001-parser-strategy.md`, `NOTICE`, fixture
manifest schema, initial goldens, traceability table.

**Fixture sourcing.** The legacy suite is `test/testN.sh` plus the `*.ref` files in `test/`; the
`testN.sh.tmpdir/` directories are empty build artifacts and contain nothing. Mapping:

| Tests | Content | Use here |
|---|---|---|
| 1–9, 21–23 | gfortran compile round-trips (`-compile`), no findent flags | Adapt as Phase 4 semantic smoke tests only |
| 10 | `--label_left`, `-I0`, `-i6` | **Port** |
| 11 | `--last_indent`, `-Ia`, `--indent_critical`, `--indent_changeteam` | **Port** (free-form parts) |
| 12 | `--deps`, `--makefdeps`, `--last_usable` | Port `--last_usable` only; rest is post-MVP |
| 13 | `--help`, `--manpage`, `--version` | Reference only — output text differs by design |
| 14 | `--continuation`, `--max_indent`, `--query_fix_free`, `-ofixed` | **Port** `--max_indent` and `--start-indent=a` only |
| 15 | `--indent=none`, `--refactor_end`, `--input_line_length`, format flags | **Port** `-i-` and `-Rr`/`-RR`; `-L` is post-MVP |
| 16 | The full per-construct indent matrix, `--openmp=0`, `-C-` | **Port — highest value fixture in the suite** |
| 17, 25 | `--selfrep`, misc | Out of scope |
| 18 | `-i5` misc | **Port** |
| 19 | `-I0`, `-i3`, format forcing | **Port** (free-form parts) |
| 20 | `--include_left`, long/short alias equivalence across the indent matrix | **Port — covers the `_`/`-` alias rule** |
| 24 | `--align_paren`, `--indent_ampersand`, `-k`/`-kd`/`-k-`, `--ws_remred`, `--relabel` | **Port** all but `--relabel` |
| 26, 27 | free-form cases | **Port** |

Gate M0: reviewers can tell exactly what compatibility means; every unsupported fixed-form path has
a specified error; every retained flag maps to at least one planned test.

### Phase 1 — Foundation

- **B0 Scaffold agent:** crate, library API shell, CLI skeleton, CI, `cargo fmt`/`clippy`, MSRV,
  release profile and musl target from 4.5, and the size/startup CI checks.
- **B1 Scanner agent:** `SourceBuffer`, `PhysicalLine`, and the shared byte scanner.
- **B2 Harness agent:** Rust golden runner, preservation helpers, property scaffolding, and
  `benches/throughput.rs` with the Section 4.5 baseline recorded.

Then **B3 Assembler agent:** continuation assembly, semicolon splitting, and preprocessor-directive
continuation, based on B1.

Gate M1:

- Scanner/assembler round-trip arbitrary input, including non-UTF-8 bytes, with no unintended text
  changes.
- Quotes, comments, Hollerith, nested delimiters, semicolons, `&`, CRLF, missing final newline, and
  `\`/`&` directive continuation each have focused tests.
- Malformed input cannot panic.
- Throughput on the benchmark file is recorded; scanning alone should already exceed the 3 M lines/s
  target, since it is the cheapest stage.

### Phase 2 — Classification

Split by non-overlapping recognizer modules:

- **C0 Definitions agent:** programs, procedures, modules, submodules, interfaces, types,
  `CONTAINS`, matching ends.
- **C1 Executable constructs agent:** `IF`, `DO`, `SELECT`, `WHERE`, `FORALL`, `ASSOCIATE`, `BLOCK`,
  teams, critical, enum, branches, ends.
- **C2 Ambiguity/labels/fix agent:** assignment-first behavior, keyword identifiers, construct
  names, statement labels, labeled `DO`, `GOTO`, label-bearing I/O and calls, and `findentfix:`
  directive parsing.

One integration agent merges ordered dispatch only after module-level tests pass.

Gate M2:

- Every supported `StatementKind` has positive, negative, mixed-case, whitespace, and malformed
  tests.
- The classifier always returns a result, using `Unknown` when needed.
- The keyword-as-identifier corpus from 4.2 does not affect the indentation stack.
- `findentfix:` directives classify identically to the equivalent bare statement, and `p-on`/`p-off`
  are inert.

Decision point at the end of Phase 2: include COCO/FYPP in the first release, or defer.

### Phase 3 — Engine, emitter, CLI

- **D0 Engine agent:** typed construct stack, transitions, labeled-`DO` lifecycle, max-indent clamp.
- **D1 Preprocessor agent:** CPP branch snapshots, restore, and the `PRE_ENDIF`/`PRE_ENDIFE`
  distinction from 5.5.
- **D2 Emitter agent:** leading indentation, labels, comments, blank lines, OpenMP sentinels,
  physical layout preservation, continuation indentation, and parenthesis alignment.
- **D3 CLI agent:** the flag surface from Section 6, the overloaded `-i` parser, fixed-form
  rejection, exit statuses, broken-pipe handling.

Integrate through statement events and `IndentDecision`; emitter code must not mutate engine state.

Gate M3:

- Broad structural fixtures match the oracle within declared scope.
- Semicolon statements update state correctly while emitting once.
- Unbalanced ends and incomplete editor buffers recover without panic.
- CPP alternate branches do not leak indentation state.
- All supported flags have CLI snapshots and library-level behavior tests; combinations are covered,
  not only individual flags.
- End-to-end throughput on the benchmark file meets the Section 4.5 target, or the shortfall is
  diagnosed with a profile.

### Phase 4 — Transformations and differential hardening

- **E0 END-refactor agent:** typed-stack-based end completion and case policy (`-Rr`, `-RR`).
- **E1 Whitespace agent:** `--ws_remred`, guarded around strings and skipped for Hollerith-bearing
  statements.
- **E2 Query agent:** `--last-indent` and `--last-usable`.
- **E3 OpenMP agent:** free-form conditional/directive fixtures, including sentinel near-misses.

Then differential hardening:

- Port the fixtures identified in the Phase 0 table.
- Add mutations for keyword case, whitespace, construct names, comments, strings containing `!`,
  `&`, or `;`, semicolon statements, continuation placement, labels, CPP branches, `findentfix:`
  comments, and non-UTF-8 bytes in comments and strings.
- Triage every oracle mismatch as (1) Rust bug, (2) intentional documented divergence, or (3)
  unsupported input. Reduce every bug to a committed fixture before fixing it.
- Compile syntax-valid formatted fixtures with `gfortran -ffree-form -ffree-line-length-none`,
  adding `-cpp`, `-fopenmp`, or `-fcoarray=single` where relevant. This is a semantic smoke test,
  not a substitute for byte-for-byte formatting tests.

Gate M4: the agreed fixture manifest is green, intentional divergences are documented, and no
supported mismatch is unclassified.

### Phase 5 — Robustness and release

- Fuzz scanner, assembler, classifier, engine, and whole formatter separately. Seed the corpus with
  the fixture set and with truncated/mid-keystroke variants.
- Run idempotence and preservation properties over generated and corpus inputs.
- Confirm the Section 4.5 measurements in CI: throughput and startup are informational, while
  stripped size and peak RSS remain release budgets and fail the build on regression.
- Produce reproducible static release binaries and checksums for the three targets.
- Document migration from findent, especially removed flags and the three deliberate divergences.

Gate M5: the release acceptance criteria in Section 10 are met.

### Phase 6 — Optional features

Separate proposals only, after M5: relabeling as a whole-input source-span transform; dependency
extraction; COCO/FYPP if deferred; `-L`/`--input_line_length`; `FINDENT_FLAGS` if demanded.

## 8. AI-agent operating protocol

Each agent receives a bounded task containing owned files/directories, required inputs and merged
milestone, behavioral examples and relevant legacy source locations, the exact tests and commands
required before handoff, and explicit non-goals.

Each agent must return a small reviewable commit, tests added or updated, commands run and their
results, known deviations and follow-up risks, and no unrelated formatting or dependency changes.

Coordinator rules:

1. Maintain the feature/test traceability table and dependency graph.
2. Give parallel agents disjoint file ownership.
3. Merge foundations before dependent packages begin.
4. Require a reduced regression fixture for every compatibility bug.
5. Reject broad rewrites that combine scanner, classifier, engine, and emitter changes without
   independently testable boundaries.
6. Use a dedicated integration agent after each parallel wave.
7. Keep oracle-derived outputs reviewed and committed; never regenerate all goldens silently.
8. Record architectural decisions in ADRs rather than burying them in agent prompts.
9. Reject any new dependency that is not accompanied by its measured cost against the Section 4.5
   budget.

Review pairing: scanner changes reviewed by the assembler/emitter owner; classifier changes by the
engine owner; engine changes by the differential-test owner; CLI changes against the compatibility
contract by an agent that did not implement them.

## 9. Testing strategy

### Golden differential tests

Each manifest entry specifies fixture, arguments, expected stdout, stderr, status, category, and
support/exclusion rationale. Capture with an equivalent of:

```sh
env -u FINDENT_FLAGS LC_ALL=C /opt/findent/src/findent -ifree ARGS < input.f90
```

Locale and newline policy are fixed at capture. Goldens are byte-for-byte unless the manifest
explicitly declares normalization.

### Unit tests

Scanner state transitions and spans; continuation assembly including directive continuation; one
classifier recognizer at a time; indentation stack transitions; preprocessor snapshot/restore
including `PRE_ENDIFE`; label-stack transitions including the shared-label case; emitter policies
independent of engine state; CLI parsing, especially overloaded attached `-i` values, optional
values, and `_`/`-` alias equivalence.

### Properties

- **Idempotence:** `format(format(source)) == format(source)` for default formatting and all
  idempotent option sets.
- **Preservation:** with transformations disabled, every byte outside leading whitespace is
  unchanged, except that spacing between a statement label's digits and its body may change under
  `label_left`. This is the strongest guard in the suite; run it over the whole corpus and normalize
  only that documented label-padding boundary.
- **Totality:** arbitrary byte input returns output or a structured error, never a panic.
- **Stack safety:** indentation never becomes negative and stacks never underflow.
- **Unknown stability:** inserting an unknown statement does not invent a construct transition.
- **Case tolerance:** keyword case does not change classification.
- **Span validity:** every stored span is ordered and in bounds.
- **Byte transparency:** input containing arbitrary non-ASCII bytes in comments and string literals
  round-trips unchanged.

### Performance tests

A dependency-free release benchmark over the 220k-line corpus, plus continuation-heavy and
preprocessor-heavy variants, recorded in CI without a hard throughput threshold. A startup
benchmark over empty input and a `cargo bloat`-style size check on the stripped release artifact
remain required release measurements; RSS and static-artifact jobs provide the remaining release
evidence.

## 10. Release acceptance criteria

- `cargo fmt --check`, strict `cargo clippy`, unit, integration, property, and selected fuzz
  regression tests pass.
- All supported golden manifest entries pass byte-for-byte.
- All mismatches are classified and documented.
- Default formatting is idempotent over the retained corpus.
- With transformations off, formatting changes only leading whitespace, documented label padding,
  and the documented final-newline addition.
- No panics on malformed, truncated, or non-UTF-8 source input.
- Fixed-form flags fail explicitly with exit 2 and are documented as unsupported.
- Throughput and startup are recorded on the documented runner as informational measurements; the
  stripped binary size and peak RSS remain release budgets. The 3 M lines/s throughput aspiration
  is non-blocking unless a change causes a clear pathological regression.
- Static binaries produced for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, and
  `x86_64-pc-windows-msvc`, with no C toolchain in the build graph.
- Library and CLI documentation cover every supported option and every intentional incompatibility.
- BSD-3-Clause attribution obligations are satisfied in source, binary, and README.

## 11. Primary risks and mitigations

| Risk | Mitigation |
|---|---|
| A "simple" scanner is duplicated in several modules | One shared span-producing scanner is a hard architecture rule. |
| Keyword-as-identifier cases are misclassified | Assignment-first ordered recognition plus the negative corpus in 4.2. |
| Preprocessor branches corrupt stack state | Snapshot all formatter state; test nested/alternate branches and both endif outcomes independently. |
| Source spelling is accidentally normalized | Byte spans into one immutable buffer, pure emitter, preservation property. |
| Non-ASCII source is rejected or mangled | Byte-oriented core (4.4) plus the byte-transparency property. |
| The performance goal is designed away by allocation-per-line data structures | The Section 4.5 budget is CI-enforced from Phase 1, not measured at the end. |
| A dependency quietly breaks the standalone or startup goal | Coordinator rule 9; no `build.rs` invoking a C compiler; musl target in CI from Phase 1. |
| AI agents create incompatible local abstractions | Milestone interfaces are merged before parallel dependent work begins. |
| Golden parity preserves an actual findent bug | Triage every mismatch; document intentional improvements instead of blindly copying. |
| `findentfix` is discovered late and forces classifier rework | In scope from Phase 2, owned by C2 alongside the ambiguity rules it exists to repair. |
| Relabeling contaminates the core architecture | Deferred; must be a source-span-based transform in a separate proposal. |
| Scope expands back toward a full parser | `StatementInfo`, opaque expressions, and the explicit non-goal remain review gates. |

## 12. First execution wave

Three agents:

1. **Contract:** flag matrix, exit statuses, divergence list, newline/encoding policy (already
   resolved in 5.1 and 4.4 — transcribe and test them), licence/attribution, ADR 0001.
2. **Oracle:** fixture extraction per the Phase 0 table, manifest-driven harness, traceability table.
3. **Scanner prototype:** `SourceBuffer` + byte scanner with round-trip and byte-transparency tests,
   and the throughput benchmark wired up against the recorded C++ baseline.

Do not begin the indentation engine until the scanner, logical-statement model, classifier contract,
and representative oracle corpus have passed M0/M1 review.

## 13. Reference material

- Design narrative: `/opt/findent/doc/README`, "Findent: how?" section.
- Logical statement assembly and structural state: `/opt/findent/src/fortran.cpp` (1,208 lines).
- Free-form assembly and emission: `/opt/findent/src/free.cpp` (736 lines).
- Normalization: `/opt/findent/src/line_prep.cpp` (606 lines).
- Preprocessor branch logic: `/opt/findent/src/pre_analyzer.cpp`, `findentclass.cpp:105-122`.
- `findentfix` handling: `/opt/findent/src/lexer.l:417-437`, `fortran.cpp:406,516`.
- Lexer and shallow grammar: `/opt/findent/src/lexer.l` (725), `/opt/findent/src/parser.y` (627).
- Classifier result: `/opt/findent/src/prop.h`.
- Legacy CLI: `/opt/findent/src/findent.1`, `/opt/findent/src/flags.cpp` (704 lines).
- Legacy tests: `/opt/findent/test/test*.sh` plus `*.ref` — the `.tmpdir/` directories are empty
  build artifacts.
- Rust/Fortran reference implementation: <https://github.com/PlasmaFAIR/fortitude>.
- `tree-sitter-fortran`, rejected in ADR 0001: <https://github.com/stadelmanma/tree-sitter-fortran>.
