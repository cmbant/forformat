# Full-mode guide

Full mode combines lexical normalization, declaration-driven case handling, wrapping, and the
findent layout engine. The historical design rationale is in
[`history/full-mode-port-design.md`](history/full-mode-port-design.md); the compatibility boundary
is in [`compatibility.md`](compatibility.md).

## Validation loop

Run the focused Rust suite while changing a normalization or wrapping rule:

```sh
cargo test --release
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --release --no-deps
./tools/check_fixture_syntax.sh target/debug/forformat
./tools/check_fuzz_regression.sh
./tools/check_cli_contract.sh target/debug/forformat
```

The property suite exercises I1, `full(full(x)) == full(x)`, and I2,
`indent_only(full(x)) == full(x)`, over the checked-in fixtures and protected case/spacing
mutations. The route test sends every fixture through stdin, an isolated file, and a project file.
The normal release checks also cover package and wheel behavior.

When a failure exposes a formatting bug, reduce it to a small Fortran shape, add a fixture and a
manifest row when the behavior belongs in the golden suite, and assert the first-pass fixed point.
Keep findent golden outputs unchanged unless the compatibility contract itself changes.

## Pipeline

The stages are deliberately ordered:

```text
bytes -> normalization (steps 1-15) -> wrapping (16) -> layout engine -> post-layout (17-20)
```

Normalization may change line contents and sometimes line count. The document is re-analyzed after
each structural pass, so later rules see current statement and scope boundaries. Wrapping receives
the layout plan's first-line and continuation columns. The engine owns every emitted column, which
makes I2 structural rather than empirical. I1 remains an obligation of every pass.

No pass may rewrite protected literal, Hollerith, preprocessor, or ordinary comment bytes except
the narrow documented commented-assignment rule. Edits are span-based and byte-oriented, so source
that is not valid UTF-8 remains supported.

## Per-line normalization

`src/transform/passes/line_rules.rs` applies token-span edits without rebuilding lines from token
spellings. Its ordered rules are:

- lowercase genuine Fortran keywords, intrinsics, specifiers, and real-literal exponent markers;
- apply project and local declaration case while leaving macros and components protected;
- expand `endif` and `blockdata`, compact `go to` to `goto`, and normalize multiword keywords;
- normalize `if (`, `dimension(`, `type(`, `select type (`, parenthesized statements, `) then`,
  `COMMON /blk/`, and old-style declaration spacing;
- normalize delimiters, commas, arithmetic and assignment operators, and dotted operators;
- modernize `(/ ... /)` to `[ ... ]` outside `FORMAT`, add output-item spacing, and apply the
  narrow comment rule.

A continuation line has no statement context by itself. `LineOptions::continued_*` carries facts
such as declaration, named-argument, FORMAT, and open-group state into the per-line chain. When
wrapping rejoins a statement, the joined text runs through the appropriate rules again.

Examples of the intended shapes:

```fortran
GO   TO 10                         ! becomes: goto 10
DOUBLE   PRECISION :: X            ! becomes: double precision :: X
WRITE( UNIT = 1 )'value'           ! becomes: write(unit=1) 'value'
real :: values = (/ 1, 2 /)        ! becomes: real :: values = [1, 2]
```

`FORMAT` edit descriptors keep `/)` because it closes a format list, not an array constructor.
String literals, Hollerith payloads, preprocessor directives, and comments retain their protected
contents.

## Declaration and case engine

`src/analysis/declarations.rs` extracts scoped module, type, procedure, component, argument, macro,
and kind-suffix facts. `CaseResolver` applies the nearest governing declaration, then project
evidence where it is unambiguous. Local declarations outrank project declarations; ambiguous
spellings remain untouched. Components and type-bound procedures are resolved through their owner
type, so an unresolved `%` chain never falls through to a keyword rule.

Numeric kind suffixes are identifier occurrences. A declaration such as `real(DL)` governs
`1.0_DL` on every continuation line; numeric kinds such as `_8` and unknown names are inert.

## Wrapping and layout

Wrapping joins a logical statement, detaches a trailing comment, selects a safe break, and emits
continuation markers. It never splits a token, Hollerith payload, or continued literal. A long
literal may split only at a whitespace boundary inside its content. If no safe break exists, the
whole statement is retained and a decline diagnostic explains why.

The available width is measured from the text the engine will emit. Normalization can widen a
statement, the engine can move it, and declaration-separator alignment can add owed space around
`::`; the wrapper accounts for those effects. Parenthesis alignment and continuation indentation
come from the active layout plan, not a fixed column.

Post-layout passes align declaration separators, align selected trailing comments, enforce program
unit blank-line policy, cap blank runs, and remove trailing horizontal whitespace. If a post-layout
pass changes width, the text is laid out again before the final output is accepted.

## File and project workflow

The CLI supports stdin, explicit paths, `--all`, repeatable `--exclude` and `--extend-exclude`,
`--stdout`, `--isolated`,
`--check`, and `--diff`.
Explicit files in a Git checkout use declarations from the tracked free-form sources; `--isolated`
and stdin use only the input buffer. `--project-context` lets a stdin buffer borrow declarations
from a checkout while excluding its stale on-disk target.

Nested Git queries clear hook environment variables. Extension validation happens before opening a
file, and workflow failures use distinct status-2 diagnostics. In-place replacement preserves mode
bits and updates a symlink target rather than replacing the link.

## Code map

| Concern | Module |
|---|---|
| Protected regions and lexical state | `src/source/regions.rs` |
| Tokens and statement assembly | `src/source/tokens.rs`, `src/source/logical_statement.rs` |
| Layout planning and emission | `src/format/planner.rs`, `src/format/engine.rs` |
| Wrapping | `src/format/wrapping.rs` |
| Full-mode driver | `src/format/full.rs` |
| Mutable document and re-analysis | `src/transform/document.rs` |
| Pass order | `src/transform/pipeline.rs` |
| Normalization passes | `src/transform/passes/` |
| Scope and declaration analysis | `src/analysis/` |
| CLI and file workflow | `src/cli.rs`, `src/io/` |

## Durable maintenance rules

- A pass is complete or inert; half-applied normalization is not acceptable.
- Every new rule needs a fixture or focused test proving I1.
- A continuation rule must receive statement facts explicitly; it must not infer them from the
  continuation's first token.
- Any new post-layout width change must be included in emitted-width measurement.
- A behavioral claim should be explained in terms of Fortran syntax or formatter invariants, not a
  private input tree.
