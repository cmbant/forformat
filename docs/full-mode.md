# Full-mode guide

Full mode combines lexical normalization, declaration-driven case handling, wrapping, and the
findent layout engine. The compatibility boundary is in
[`compatibility.md`](compatibility.md).

## Validation loop

Run the local verification bar while changing a normalization or wrapping rule:

```sh
./tools/check_local.sh
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

`src/transform/passes/line_rules/` applies token-span edits without rebuilding lines from token
spellings. Its ordered rules are:

- lowercase genuine Fortran keywords, intrinsics, specifiers, and real-literal exponent markers;
- apply project and local declaration case while leaving macros and components protected;
- expand `endif` and `blockdata`, compact `go to` to `goto`, and normalize multiword keywords;
- normalize `if (`, `dimension(`, `type(`, `select type (`, parenthesized statements, `) then`,
  `COMMON /blk/`, and old-style declaration spacing;
- normalize delimiters, commas, arithmetic and assignment operators, and dotted operators;
- modernize `(/ ... /)` to `[ ... ]` outside `FORMAT`, add output-item spacing, and apply the
  narrow comment rule.

A continuation line has no statement context by itself. `LineState` carries facts between physical
lines and exposes them to each stage through `LineContext`, including declaration, named-argument,
FORMAT, and open-group state. When wrapping rejoins a statement, the joined text runs through the
appropriate rules again.

### Style controls

The full-mode style controls are available as long options, TOML keys, and fields of
`StyleConfig`. Their defaults preserve the existing output: recognized keywords and intrinsic
spellings are lowercase, legacy relational operators are modernized, and only binary `*`, `/`,
and `**` are compact.

`compact-multiplicative` governs binary `*`, `/`, and `**` only. With it enabled
those operators are compact; with it disabled they have one space on both sides.

The other style switches are independent booleans: `relational-symbols` modernizes legacy dotted
relational operators, `array-brackets` converts `(/ ... /)` constructors, `split-compound-keywords`
splits run-together compound keywords, `join-goto` contracts `go to`, `strip-empty-args` removes
empty `subroutine` definition argument lists, and `remove-redundant-parens` and
`remove-terminal-return` gate their named cleanup passes. `program-unit-spacing` controls the
program-unit separator pass; `max-blank-lines` caps ordinary blank runs. `delimiter-spacing`,
`comment-spacing`, and `continuation-markers` independently control delimiter/comment/continuation
normalization. A value of `0` disables only the named behavior.

The compact item-leading `=` rule also covers named arguments, declaration and I/O specifiers,
and `do concurrent(i=1:n)`, including when the item begins on a continuation line. `*` in
`character*8`, assumed-size declarations, and `write(*, *)`, `/` in `FORMAT` and `(/ ... /)`,
real-exponent signs, delimiters, and protected regions are not expression-operator spacing.
Adjacent edits produce at most one space, so `append=.not. ready` stays compact at the equals
boundary.

`keyword-case` applies to recognized language keywords, intrinsic names, specifiers, intrinsic
dotted words, and real-literal `e`/`d` markers. It does not change declared or unresolved
identifiers, macros, components after `%`, user-defined dotted operators, literals, preprocessor
bytes, or ordinary comment text. Reserved OpenMP directives are the one exception: they have their
own switch, `openmp-case`, which holds them at upper case by default however `keyword-case` is set
— see [the options reference](options.md#fullnormalization-style).
`split-compound-keywords=1` performs the existing compound-keyword split, while `join-goto=1`
contracts `go to` to `goto`. Interior whitespace in recognized multiword
keyword pairs is always collapsed. With `keyword-case=preserve`, authored letters are retained
while splitting or joining, so `EnDiF` becomes `EnD iF` and `Go   To 10` becomes `GoTo 10`. Setting
either boolean to `0` disables only that switch's named behavior; it does not turn the formatter
into a general byte-preserving mode. The other enabled style controls still apply.

For example:

`x = a*b/c**2 + d` becomes `x = a * b / c ** 2 + d` when
`--compact-multiplicative=0`.

The per-line gates live in `passes::line_rules`; the line-count gates for redundant parentheses,
terminal returns, and continuations live in `transform::pipeline`; program-unit spacing and blank
caps are post-layout passes. This ownership keeps `--indent-only` on its early engine return and
keeps continuation-sensitive facts in `LineState` and `LineContext` rather than inferring statement
kind from a continuation line.

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

`--max-blank-lines=0` removes program-unit separators even with
`--program-unit-spacing=1`, because the cap runs after the separator pass.

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

Full-line comment separators whose body is a run of one repeated non-whitespace character are not
prose to reflow. With wrapping enabled, if their final laid-out line exceeds `--line-length`, the
repeated run is truncated to the budget only when the comment prefix and at least one separator
character fit. This covers shapes such as `! ----------------`, `!CCCCCCCC`, and `!   ########`.
Ordinary prose comments and inline comments are left unchanged, and disabling wrapping leaves
separator comments at their authored length.

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

### SELECT TYPE and SELECT RANK association scopes

Project-aware name normalization models program units, interfaces, derived types, `BLOCK`, and
association constructs. `SELECT TYPE` and `SELECT RANK` association names are scoped to the select
construct. A `TYPE IS` or `CLASS IS` guard refines a `SELECT TYPE` association to the exact visible
derived-type entity for that branch, so same-named types in unrelated modules cannot contribute each
other's members. `SELECT RANK` retains the selector's exact derived-type identity across rank guards.
The no-`=>` form resolves its selector in the enclosing scope on the opening statement and keeps that
outer declaration's spelling; the association becomes active only after selector evaluation.
Default, intrinsic-type, or ambiguous guards stay conservative rather than borrowing an unrelated
project type. The casing pass carries this branch state directly; `ScopeTree` does not expose each
select guard as a general-purpose lexical scope to other analysis consumers.
