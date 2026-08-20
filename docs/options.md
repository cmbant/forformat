# CLI and configuration options

This is the primary user-facing reference for normal `forformat` options. `forformat --help` is a
compact terminal summary; this page explains the option families, defaults, project-configuration
mapping, and interactions. Legacy findent spellings that are only compatibility aliases are kept in
[migration.md](migration.md) and [compatibility.md](compatibility.md) rather than exhaustively listed
here.

Options configure formatter **policy** rather than exposing every internal pass. Full and
normalize-only mode perform the safe normalization required by their format contracts, including
project/local declaration-driven casing and lexical continuation handling. Canonicalize-only keeps
only token/spelling canonicalization and deliberately preserves authored presentation whitespace and
physical layout. The switches below control the behaviours intended to be user choices.

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
| `--canonicalize-only` | `mode = "canonicalize-only"` | canonical token/spelling changes without whitespace or layout normalization | no |

Mode switches are valueless on the command line. In TOML use the single `mode` key.

Canonicalize-only preserves indentation, incidental horizontal whitespace, comments, blank-line
structure, continuation layout, trailing whitespace, and each physical line's original LF/CRLF
terminator. Canonical replacements may still contain whitespace that is intrinsic to the replacement
spelling: for example `enddo` becomes `end do`, `endmodule` becomes `end module`, and `go to` may
become `goto`. This is therefore a promise not to make **whitespace-only formatting edits**, not a
promise that the byte count of every whitespace run can never change as part of a token rewrite.

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

These controls affect `--full` and `--normalize-only`. Canonicalize-only applies controls that change
canonical token/spelling, but ignores presentation-only whitespace, blank-line, continuation-marker,
and alignment effects. `--indent-only` deliberately ignores these style controls.

| CLI / TOML key | Values | Default | Effect |
| --- | --- | --- | --- |
| `--keyword-case` / `keyword_case` | `lower`, `upper`, `preserve` | `lower` | recognized keyword/intrinsic spelling |
| `--relational-symbols` / `relational_symbols` | boolean | true | `.eq.` etc. become symbolic operators |
| `--array-brackets` / `array_brackets` | boolean | true | `(/ ... /)` becomes `[ ... ]` where safe |
| `--compact-multiplicative` / `compact_multiplicative` | boolean | true | compact binary `*`, `/`, and `**` |
| `--join-goto` / `join_goto` | boolean | true | `go to` becomes `goto` |
| `--split-compound-keywords` / `split_compound_keywords` | boolean | true | `endif` becomes `end if` and similar |
| `--strip-empty-args` / `strip_empty_args` | boolean | true | remove empty subroutine definition `()` |
| `--remove-redundant-parens` / `remove_redundant_parens` | boolean | true | remove safely redundant nested parentheses |
| `--remove-terminal-return` / `remove_terminal_return` | boolean | true | remove a bare terminal procedure `return` |
| `--program-unit-spacing` / `program_unit_spacing` | boolean | true | canonical blank-line separators around program units |
| `--max-blank-lines` / `max_blank_lines` | integer or `preserve` | `2` | cap consecutive blank lines |
| `--delimiter-spacing` / `delimiter_spacing` | boolean | true | normalize delimiter spacing |
| `--comment-spacing` / `comment_spacing` | boolean | true | normalize the gap before trailing `!` |
| `--continuation-markers` / `continuation_markers` | boolean | true | normalize continuation markers and OpenMP sentinels |
| `--uppercase-single-l[=BOOL]` / `uppercase_single_l` | boolean | false | uppercase a lone identifier `l` |

`--max-blank-lines=0` can remove separators inserted by `--program-unit-spacing=true`, because the
blank-line cap runs afterward. Canonicalize-only does not perform either blank-line transformation.

### Whitespace and alignment

| CLI | Configuration | Default | Meaning |
| --- | --- | --- | --- |
| `--ws-remred[=N]` | `ws_remred = N/BOOL` | `0` | reduce redundant whitespace; bare means enabled |
| `--align-declarations=BOOL` | `align_declarations = BOOL` | true | align/shrink declaration `::` runs |
| `--align-comments=BOOL` | `align_comments = BOOL` | false | align/shrink trailing-comment runs |

When declaration or comment alignment owns its corresponding gap, `--ws-remred` leaves that gap for
the alignment pass instead of collapsing it first. Canonicalize-only bypasses structural layout and
therefore does not run these whitespace/alignment effects.

### END completion

`-Rr` and bare `--refactor-end` complete END definition statements. `-RR` or
`--refactor-end=upcase` also uppercases the completed END spelling. Boolean values explicitly enable
or disable the transformation. `--refactor-procedures` is an accepted compatibility alias; prefer
`--refactor-end`. The configuration key is `refactor_end`.

END completion is also available in canonicalize-only mode. In that mode the scope-aware END text is
replaced in place while retaining the authored leading indentation, trailing horizontal whitespace,
comment gap, and line terminator.

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
retain their authored continuation layout. `--rewrap=false` restores the normal overflow-only policy;
a later `--no-wrap` disables the wrapping stage entirely.

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
