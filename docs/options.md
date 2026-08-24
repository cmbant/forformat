# CLI and configuration options

This is the primary user-facing reference for normal `forformat` options. `forformat --help` is a
compact terminal summary; this page explains the option families, defaults, project-configuration
mapping, and interactions. Legacy findent spellings that are only compatibility aliases are kept in
[migration.md](migration.md) and [compatibility.md](compatibility.md) rather than exhaustively listed
here.

Options configure formatter **policy** rather than exposing every internal pass. Full and
normalize-only mode perform the safe normalization required by their format contracts, including
project/local declaration-driven casing and lexical continuation handling. Canonicalize-only keeps
canonicalization transformations that do not require presentation layout while deliberately
preserving authored presentation whitespace and physical layout. Canonicalize-and-indent applies
that same canonicalization policy and then the existing indent-only layout. The switches below
control the behaviours intended to be user choices.

## Quick examples

Format one file in place:

```sh
forformat src/module.f90
```

Check every tracked free-form source without rewriting anything:

```sh
forformat --all-files --check
```

Use four-space indentation while keeping module and procedure bodies at the surrounding level:

```sh
forformat --indent=4 --indent-module=0 --indent-procedure=0 src/module.f90
```

Uppercase recognized language words and wrap to a 100-column budget:

```sh
forformat --keyword-case=upper --line-length=100 src/module.f90
```

Canonicalize language spelling without reformatting authored whitespace:

```sh
forformat --canonicalize-only src/module.f90
```

Canonicalize spelling and then apply findent-compatible indentation, without wrapping:

```sh
forformat --canonicalize-and-indent src/module.f90
```

Repack existing continuations against the current line-length policy:

```sh
forformat --rewrap --line-length=100 src/module.f90
```

Format an editor buffer from stdin while using the rest of the checkout for declaration context:

```sh
forformat --stdin --project-context=src/module.f90 < src/module.f90
```

Limit semantic context without changing the explicit formatting target:

```sh
forformat --stdout src/module.f90 --context-path=src --context-path=modules
```

## Modes

| CLI | Configuration | Effect | Default |
| --- | --- | --- | --- |
| `--full` | `mode = "full"` | normalization, wrapping, and structural layout | yes |
| `--indent-only` | `mode = "indent-only"` | findent-compatible layout; style controls are ignored | no |
| `--normalize-only` | `mode = "normalize-only"` | normalization only; no layout or wrapping | no |
| `--canonicalize-only` | `mode = "canonicalize-only"` | canonical transformations without whitespace or layout normalization | no |
| `--canonicalize-and-indent` | `mode = "canonicalize-and-indent"` | canonical transformations followed by findent-compatible indentation; no wrapping or full-mode post-layout alignment | no |

Mode switches are valueless on the command line. In TOML use the single `mode` key. Mode is one
setting rather than a combination, so when more than one mode switch is given the last one wins
outright and no part of an earlier mode survives.

Canonicalize-only preserves indentation, incidental horizontal whitespace, comments, blank-line
structure, continuation layout, and each physical line's original LF/CRLF terminator. It does not
preserve whitespace at end of line: that is invisible rather than a formatting choice, so every mode
removes it — see [Whitespace and alignment](#whitespace-and-alignment). Canonical replacements may
still contain whitespace that is intrinsic to the replacement
spelling: for example `enddo` becomes `end do`, `endmodule` becomes `end module`, and `go to` may
become `goto`. Other enabled canonicalization transforms, such as safely redundant-parenthesis
removal, may also change syntax without being whitespace formatting. This is therefore a promise not
to make **whitespace-only formatting edits**, not a promise that only character case can change.

Canonicalize-and-indent is defined as the exact composition of canonicalize-only followed by
indent-only with the same settings. Canonicalization therefore still preserves authored interior
spacing and physical line structure; the second stage then owns the same leading indentation and
trailing-whitespace changes as `--indent-only`. It does not run the wrapper, declaration/comment
alignment, program-unit spacing, or blank-line limiting that belong to full mode.

## Selecting input and output

| Option | Configuration | Meaning |
| --- | --- | --- |
| positional `PATH ...` | — | format explicit files in place |
| positional `DIR` (single argument) | — | equivalent to `--all-files DIR`: format that directory's tracked sources |
| `--stdin` | — | read source from stdin; this is also the default when no path is supplied |
| `--stdout` | — | write one explicit file's formatted result to stdout instead of replacing it |
| `--all-files [DIR]` | — | select tracked sources owned by the checkout; submodules provide context only |
| `--all [DIR]` | — | select tracked sources recursively, including initialized submodule sources |
| `--check` | — | do not rewrite; exit 1 when a selected file would change |
| `--diff` | — | print unified diffs; exit 1 when a selected file would change |
| `--show-files` | — | print selected target paths without reading or formatting them |
| `--query-format` | — | print `free` or `fixed` for each selected input and exit |
| `--input-format=auto` | `input_format = "auto"` | use automatic fixed/free detection; default |
| `-ifree`, `--input-format=free` | `input_format = "free"` | force free-form handling and bypass detection |
| `-ofree`, `-osame`, `--output-format=free`, `--output-format=same` | `output_format = "free"` or `"same"` | accepted compatibility spellings; output remains free form |

Explicit fixed-form input or output requests (`-ifixed`, `-ofixed`, `--input-format=fixed`,
`--output-format=fixed`) are unsupported. Automatic detection may still classify an input as fixed;
that source is then left unchanged.

`--stdout` requires exactly one explicit path. `--check`, `--diff`, and `--show-files` require
explicit paths, `--all`, or `--all-files`. Query modes cannot be combined with rewrite/check/diff
modes.

## Project context and file discovery

| Option | Configuration | Meaning | Default |
| --- | --- | --- | --- |
| `--project-context=PATH` | — | associate stdin with the Git project containing `PATH`; a source-file path also shadows that on-disk file | none |
| `--context-path=DIR` | `context_paths = ["..."]` | limit semantic project context; repeatable on CLI | whole eligible project |
| `--isolated` | — | disable project scanning for explicit files | false |
| `--no-submodules[=BOOL]` | `no_submodules = true/false` | omit initialized submodules from targets and context | false |
| `--exclude=GLOB` | `exclude = ["..."]` | select the exclusion set for bulk targets and project context | empty set |
| `--extend-exclude=GLOB` | `extend_exclude = ["..."]` | add exclusions without replacing the selected set | empty |

`--project-context` identifies a project or stdin file identity; it does **not** restrict which files
supply semantic context. Use `--context-path` for that. Command-line context paths replace configured
`context_paths` rather than accumulating with them.

`--exclude` is also replacement-style: any command-line `--exclude` discards configured `exclude`
patterns. `--extend-exclude` is additive. Explicit formatting paths are not force-excluded even when
they match a pattern.

Patterns use `/` separators. `*` stays within one component, `**` crosses components, `?` matches
one non-separator character, a trailing `/` matches a directory prefix, and a leading `/` anchors at
the repository root.

Neither `--context-path` nor the exclusion options restrict Fortran `INCLUDE` resolution. A fragment
named by a source that is already being analyzed is read from disk relative to that source, because
it is part of that source's text; absolute paths are also honored. Compiler include-directory (`-I`)
search paths are not modeled, and missing/unreadable/unanalysable fragments are left unresolved
rather than guessed. An include fragment is never selected as a project source in its own right. Use
`--isolated` to disable project analysis, and with it include resolution, entirely.

See [file-workflow.md](file-workflow.md) for repository discovery, non-Git context discovery,
submodule behaviour, symlinks, and write semantics.

## Indentation and structural layout

### Global and continuation controls

| CLI | Configuration | Values | Default |
| --- | --- | --- | --- |
| `-iN`, `--indent=N` | `indent = N` | non-negative integer | `3` |
| `-i-`, `--indent=none` | `indent = "none"` | preserve authored leading indentation | off |
| `-IN`, `--start-indent=N` | `start_indent = N` | non-negative integer | `0` |
| `-Ia`, `--start-indent=auto` | `start_indent = "auto"` | infer starting indentation | off |
| `-MN`, `--max-indent=N` | `max_indent = N` | non-negative integer; `0` means unlimited | `100` |
| `-kN`, `--indent-continuation=N` | `indent_continuation = N` | non-negative integer | `3` |
| `-k-`, `--indent-continuation=none` | `indent_continuation = "none"` | disable continuation indentation | off |
| `--indent-continuation=default` | `indent_continuation = "default"` | use the normal continuation policy | on |
| `-K`, `--indent-ampersand[=BOOL]` | `indent_ampersand = BOOL` | boolean | false |
| `--align-paren[=N]` | `align_paren = N/BOOL` | bare means `1`; `0` disables | `0` |
| `--label-left=BOOL` | `label_left = BOOL` | boolean | true |
| `--include-left=BOOL` | `include_left = BOOL` | boolean | false |
| `--openmp=BOOL` | `openmp = BOOL` | boolean | true |

Setting `--indent=N` also resets the per-construct indents, `contains`, continuation indent, case
indent, and entry indent. Later, more-specific CLI options override that reset. Configuration files
apply the global `indent` before specific `indent_*` keys regardless of TOML key ordering, so the
specific values win there as well.

### Per-construct indentation

All long forms accept a non-negative integer. Hyphens and underscores are equivalent in TOML keys.

| Long option | Short | Config key | Default |
| --- | --- | --- | --- |
| `--indent-associate=N` | `-aN` | `indent_associate` | `3` |
| `--indent-block=N` | `-bN` | `indent_block` | `3` |
| `--indent-case=N` | `-cN` | `indent_case` | `2` |
| `--indent-do=N` | `-dN` | `indent_do` | `3` |
| `--indent-entry=N` | `-eN` | `indent_entry` | `2` |
| `--indent-enum=N` | `-EN` | `indent_enum` | `3` |
| `--indent-forall=N` | `-FN` | `indent_forall` | `3` |
| `--indent-if=N` | `-fN` | `indent_if` | `3` |
| `--indent-interface=N` | `-jN` | `indent_interface` | `3` |
| `--indent-module=N` | `-mN` | `indent_module` | `3` |
| `--indent-procedure=N` | `-rN` | `indent_procedure` | `3` |
| `--indent-select=N` | `-sN` | `indent_select` | `3` |
| `--indent-type=N` | `-tN` | `indent_type` | `3` |
| `--indent-where=N` | `-wN` | `indent_where` | `3` |
| `--indent-critical=N` | `-xN` | `indent_critical` | `3` |
| `--indent-changeteam=N` | — | `indent_changeteam` | `3` |

`-CN` / `--indent-contains=N` sets the indentation after `CONTAINS`; `-C-` or
`--indent-contains=restart` restarts indentation from the enclosing program-unit level. The config
key is `indent_contains` and accepts either a non-negative integer or `"restart"`.

## Full/normalization style

The style controls in this table affect `--full` and `--normalize-only`. Canonicalize-only and the
canonicalization stage of canonicalize-and-indent apply controls that do not amount to
presentation-only whitespace/layout changes; `--indent-only` deliberately ignores this table.

| CLI / TOML key | Values | Default | Effect |
| --- | --- | --- | --- |
| `--keyword-case` / `keyword_case` | `lower`, `upper`, `preserve` | `lower` | recognized keyword/intrinsic spelling |
| `--openmp-case` / `openmp_case` | boolean | true | uppercase reserved OpenMP sentinels and directive words |
| `--relational-symbols` / `relational_symbols` | boolean | true | `.eq.` etc. become symbolic operators |
| `--array-brackets` / `array_brackets` | boolean | true | `(/ ... /)` becomes `[ ... ]` where safe |
| `--compact-multiplicative` / `compact_multiplicative` | boolean | true | compact binary `*`, `/`, and `**` |
| `--join-goto` / `join_goto` | boolean | true | `go to` becomes `goto` |
| `--split-compound-keywords` / `split_compound_keywords` | boolean | true | `endif` becomes `end if` and similar |
| `--strip-empty-args` / `strip_empty_args` | boolean | true | remove empty subroutine definition `()` |
| `--remove-redundant-parens` / `remove_redundant_parens` | boolean | true | remove safely redundant nested parentheses |
| `--normalize-semicolons` / `normalize_semicolons` | boolean | true | drop semicolons that separate no pair of statements |
| `--remove-terminal-return` / `remove_terminal_return` | boolean | true | remove a bare terminal procedure `return` |
| `--program-unit-spacing` / `program_unit_spacing` | boolean | true | canonical blank-line separators around program units |
| `--max-blank-lines` / `max_blank_lines` | integer or `preserve` | `2` | cap consecutive blank lines |
| `--delimiter-spacing` / `delimiter_spacing` | boolean | true | normalize delimiter spacing |
| `--comment-spacing` / `comment_spacing` | boolean | true | normalize the gap before trailing `!` |
| `--continuation-markers` / `continuation_markers` | boolean | true | normalize continuation markers and OpenMP sentinels |
| `--uppercase-single-l[=BOOL]` / `uppercase_single_l` | boolean | false | uppercase a lone identifier `l` |

`--max-blank-lines=0` can remove separators inserted by `--program-unit-spacing=true`, because the
blank-line cap runs afterward. Canonicalize-only and canonicalize-and-indent do not perform either
blank-line transformation.

`--openmp-case` governs the reserved OpenMP directive sentinel and the directive words after it.
Uppercase directives over otherwise lowercase Fortran is the near-universal convention, so it
defaults to true and `!$omp parallel do private(i)` becomes `!$OMP PARALLEL DO PRIVATE(i)` whatever
`--keyword-case` says. Setting `--openmp-case=false` hands directives back to `--keyword-case`,
which is also how `--keyword-case=preserve` reaches them: pass both to leave an authored `!$OmP`
exactly as written.

Both settings draw the same boundaries. Macro names are never re-cased, and neither is a clause's
argument list: `PRIVATE(shared)` keeps the declared name as written even though `shared` is itself a
clause name elsewhere, because the argument is the user's program and not the formatter's to respell.
The exception is the handful of clauses whose argument grammar is a fixed vocabulary rather than a
list of names — `default`, `schedule`, `dist_schedule`, `proc_bind`, `order` — where the kind is
cased too, along with any modifiers a colon separates from it. The vocabulary ends there, because
what follows the kind is an expression: `schedule(dynamic, chunk)` becomes
`SCHEDULE(DYNAMIC, chunk)`, and a chunk size named after a kind keeps its spelling, so
`schedule(dynamic, static)` becomes `SCHEDULE(DYNAMIC, static)`. A comma before the colon separates
two modifiers rather than handing over to an expression, and both are cased:
`schedule(monotonic, simd: static, n)` becomes `SCHEDULE(MONOTONIC, SIMD: STATIC, n)`. Clauses that take a list
keep their modifiers as written for the same reason the list itself is kept: `MAP(to: a)` is not re-cased, since telling a
modifier from a name there needs the clause-by-clause grammar of the whole specification.
Unreserved sentinels such as `!$acc` are ordinary comments to this
formatter and keep their authored spelling under every setting. A conditional-compilation `!$ ` line
is not a directive at all — it is ordinary Fortran that only an OpenMP compiler sees — so its body
follows `--keyword-case` like any other statement and `--openmp-case` never touches it.

Both settings also apply to a directive the wrapper has split, which repeats the sentinel on each
physical line in the spelling normalization chose.

Directive spelling is canonicalization rather than presentation, so `--openmp-case` applies in every
normalizing mode, canonicalize-only and canonicalize-and-indent included, and is independent of
`--continuation-markers`. Repeating the sentinel across a split directive, dropping a body-leading
`&`, and the canonical blank after the sentinel are the presentation half, and those follow the
whitespace and continuation-marker policy as usual.

`--normalize-semicolons` keeps exactly one `;` between each adjacent pair of non-empty statements
and drops the rest, so `;;call a();;; call b();;` becomes `call a(); call b()`. Semicolons inside
character literals, Hollerith payloads, preprocessor lines, and `findentfix` comments are part of
their statement rather than separators and are never removed, and a separator that genuinely divides
two statements across a continuation is kept. The surrounding spacing is left alone; this is a
token-level rather than a whitespace transformation, so it stays active in canonicalize-only and
canonicalize-and-indent modes.

### Whitespace and alignment

| CLI | Configuration | Default | Meaning |
| --- | --- | --- | --- |
| `--reduce-whitespace[=N]` | `reduce_whitespace = N/BOOL` | `0` | reduce redundant body whitespace; bare means enabled |
| `--align-declarations=BOOL` | `align_declarations = BOOL` | true | align/shrink declaration `::` runs |
| `--align-comments=BOOL` | `align_comments = BOOL` | false | align/shrink trailing-comment runs |

Whitespace at end of line is removed in **every** mode, including `--indent-only`,
`--canonicalize-only`, and `--canonicalize-and-indent`, and is not governed by any switch: it is
invisible, so it is never the formatting choice that those modes exist to preserve. Interior
whitespace is a different matter and is preserved wherever the mode preserves presentation. The one
exception is whitespace that is not really trailing: blanks inside a character literal or a Hollerith
payload are payload bytes — `3Hab ` promises three characters — and are kept in every mode.

`--reduce-whitespace` is an emission/layout control rather than a normalize-only text pass: it
applies when the layout emitter runs (full, indent-only, and the indentation stage of
canonicalize-and-indent), and is inactive in normalize-only and canonicalize-only modes.
Declaration/comment alignment runs only in full mode. When one of those alignment passes owns its
corresponding gap, `--reduce-whitespace` leaves the gap for the alignment pass instead of collapsing
it first.

For findent command-line compatibility, `--ws_remred` (equivalently `--ws-remred`) remains an alias
for `--reduce-whitespace`; the legacy `ws_remred` TOML key is accepted for the same reason. Prefer
`--reduce-whitespace` and `reduce_whitespace` in new forformat configuration.

### END completion

`-Rr` and bare `--refactor-end` complete END definition statements. `-RR` or
`--refactor-end=upcase` also uppercases the completed END spelling. Boolean values explicitly enable
or disable the transformation. `--refactor-procedures` is an accepted compatibility alias; prefer
`--refactor-end`. The configuration key is `refactor_end`.

END completion is also available in canonicalize-only and canonicalize-and-indent modes. In those
modes the scope-aware END text is replaced in place before any combined-mode indentation, while
retaining the authored comment gap and line terminator. Whitespace left at end of line is removed,
as it is in every mode.

## Wrapping

| CLI | Configuration | Default | Meaning |
| --- | --- | --- | --- |
| `--wrap[=BOOL]` | `wrap = BOOL` | true | enable statement reflow in full mode |
| `--no-wrap[=BOOL]` | `no_wrap = BOOL` | false | negated compatibility spelling |
| `--rewrap[=BOOL]` | `rewrap = BOOL` | false | repack eligible authored continuations through the normal wrapper |
| `--line-length=N` | `line_length = N` | `120` | emitted line-length budget |

The line length is a budget, not a guarantee. A statement with no safe break point is emitted long
rather than split unsafely. Wrapping uses the active indentation and parenthesis-alignment plan when
choosing continuation columns.

`--rewrap` asks full mode to reconsider existing safe continuation breaks even when every authored
physical line already fits. The statement is joined logically first and then handed to the same
fixed-point wrapper used for ordinary over-budget statements: it may collapse to one line when the
joined form fits, or receive a completely fresh set of breaks at the active line-length budget.
Groups that the existing wrapper cannot safely reflow, including protected/comment-bearing shapes,
retain their authored continuation layout. `--rewrap=false` restores the normal overflow-only policy.
Rewrap never enables wrapping: if wrapping is disabled by `--no-wrap` or `--wrap=false`, in any CLI
order or through TOML, the rewrap policy is simply inactive. Turning wrapping off is a coherent
policy within full mode, so that combination is accepted rather than rejected.

Asking a mode that never wraps to rewrap is a different matter and is an error: `--rewrap` together
with `--indent-only`, `--normalize-only`, `--canonicalize-only`, or
`--canonicalize-and-indent` is rejected in any order, and so is the equivalent pair of TOML keys,
rather than accepted as a flag that quietly does nothing. `--rewrap=false` asks for nothing and is
accepted in every mode.

## Preprocessor definitions

`-D NAME[=VALUE]` and `--define=NAME[=VALUE]` are repeatable. In TOML, use `define`/`defines` as a
string or an array of strings:

```toml
defines = ["USE_MPI", "REAL_KIND=8"]
```

Macro names participate in case resolution; the value is retained for preprocessor evaluation.

## Query and compatibility options

This section records compatibility spellings that are useful alongside normal workflows; it is not
an exhaustive findent alias list. See [migration.md](migration.md) for legacy command-line mapping.

`-lastindent` / `--last-indent` prints the final indentation and exits. `-lastusable` /
`--last-usable` prints the final usable indentation and exits. These are command-line query modes,
not project settings.

Long option names are case-insensitive and treat `_` as `-`, so `--align_paren` and
`--align-paren` are equivalent. Legacy attached short values such as `-i4`, `-k3`, and `-a2` are
supported; separated short values are accepted as well.

Unknown options are errors. `FINDENT_FLAGS`, relabeling, dependency generation, editor wrappers,
`--safe`, and `--selfrep` are not implemented. See [migration.md](migration.md) and
[compatibility.md](compatibility.md) for the findent boundary.

## Configuration

Configuration discovery starts from the relevant project/input directory and walks upward. The first
applicable configuration is used in this order at each directory:

1. `.forformat.toml`
2. `.findent.toml` (compatibility spelling)
3. `[tool.forformat]` in `pyproject.toml`

A standalone file uses top-level keys; `pyproject.toml` uses the table:

```toml
[tool.forformat]
mode = "full"
indent = 4
indent_module = 0
indent_procedure = 0
line_length = 100
keyword_case = "upper"
context_paths = ["src", "modules"]
exclude = ["vendor/"]
extend_exclude = ["**/generated-*.f90"]
```

Use `--config=PATH` to select a file explicitly and `--no-config` to disable discovery. These two
options cannot be combined. CLI scalar options take precedence over configuration. Repeatable
`--define` and `--extend-exclude` values accumulate; command-line `--exclude` replaces configured
`exclude`; command-line `--context-path` replaces configured `context_paths`.

Relative configured `context_paths` are resolved from the configuration file's directory, not from
the process working directory.

The following workflow/query settings are intentionally command-line-only and are rejected as TOML
keys: `all`, `all-files`, `check`, `config`, `diff`, `isolated`, `last-indent`, `last-usable`,
`no-config`, `project-context`, `query-format`, `stdin`, `stdout`, and `show-files`.

## Boolean syntax

Boolean-valued options accept `true`/`false`, `yes`/`no`, or `1`/`0`. Optional boolean switches such
as `--wrap`, `--rewrap`, `--indent-ampersand`, and `--no-submodules` use the bare spelling as `true`.

Negated options apply the value to the *negated state*: `--no-wrap` disables wrapping, while
`--no-wrap=false` explicitly leaves wrapping enabled.

## Help and version

`-h` / `--help` prints the compact command-line summary. `-v` / `--version` prints the installed version.
