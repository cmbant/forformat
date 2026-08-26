# File and project workflow

The command-line workflow discovers the repository with `git rev-parse` and, when one is available,
enumerates tracked free-form sources with `git ls-files`. Both commands pass through one helper that
removes `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, and `GIT_INDEX_FILE`, so a hook's Git
environment cannot redirect nested queries.

`--all-files` selects tracked sources in the current checkout. Initialized submodule sources are not
targets, but are included in the project case-resolution context. `--all` is recursive and includes
initialized submodule sources as targets. Both modes have repeatable `--exclude=<glob>` and
`--extend-exclude=<glob>` options that remove matching tracked sources from targets and project
context. An excluded source named explicitly remains a formatting target; exclusions are not force
exclusions.

`--project-context=<path>` identifies the Git checkout for project analysis and, when PATH is a
tracked source file, identifies the source whose stdin bytes replace the stale on-disk copy. It does
not restrict the analysis scope. Repeatable `--context-path=<directory>` options do that. For
command-line options, relative directories are resolved from the repository root in a Git checkout,
or from the current working directory outside Git. Relative directories in configured
`context_paths` are resolved from the directory containing the configuration file. Thus a config at
`/project/.forformat.toml` with `context_paths = ["src"]` selects `/project/src` even when invoked
from another directory. Eligible tracked sources beneath the union are selected in Git; without
Git, eligible Fortran files are discovered recursively from the filesystem. In both cases each
directory must exist, exclusions are applied afterward, and directory symlinks are not followed
during filesystem discovery. With no `context_paths`, all eligible tracked sources retain the
existing behavior. Explicit formatting targets, bulk target selection, config discovery, and
anonymous stdin identity are unaffected by `context_paths`. Context paths change semantic context,
not explicit formatting targets. `--project-context=<source-file>` remains the explicit form for
stdin file identity and stale on-disk shadowing.

A Fortran `INCLUDE` line in a source that project analysis reads is resolved from disk, relative to
the directory of the including file, and its declarations are merged into the scope containing the
`INCLUDE`. This resolution is deliberately **not** filtered by `context_paths`, `exclude`, or
`extend-exclude`: the fragment is part of the text of a file already selected for analysis, so
dropping it would mis-analyse that selected file rather than narrow the project. A fragment
therefore contributes even when it lives outside every `--context-path` or matches an `--exclude`
pattern, and an absolute include path is read as written. This models absolute paths and paths
relative to the including source only; it does not emulate arbitrary compiler `-I`/include-directory
search paths. Missing, unreadable, or unanalyzable fragments are ignored conservatively rather than
guessed from another directory. Only files a fragment is actually included by can see it; it never
becomes a project-wide source in its own right. The one exception is a fragment that defines a whole
program unit rather than a list of declarations: a module or submodule written inside a fragment is
registered project-wide, because after inclusion that module exists exactly as if it had been
written inline. Its own scope tree is not grafted into the including file, so its line numbers never
take part in scope lookup there. `--isolated` performs no project analysis at all and so resolves no
includes.

The policy order is repository discovery, tracked-source enumeration, `context_paths` filtering,
then `exclude`/`extend-exclude` filtering, followed by project analysis. In Git, exclusions are
repository-relative. Outside Git, each explicit context directory is resolved from the current
working directory, eligible sources are recursively discovered using the existing source-extension
set, exclusions are evaluated relative to that context root, and the union is deduplicated before
project analysis. No `.gitignore` semantics are invented outside Git. `--isolated` disables project
context entirely and is rejected with `--context-path`.

`--show-files` prints the selected target paths and exits without reading or modifying source files.
It accepts explicit paths, `--all`, or `--all-files`; either bulk mode accepts one optional directory,
for example `forformat --all-files ./src --show-files`.

`--no-submodules[=BOOL]` disables recursive submodule discovery entirely when true. The bare
spelling means true, and `--no-submodules=false` overrides a true project setting. The
superproject's tracked sources remain available for project context, but initialized submodule
sources are neither targets nor context. With `--all`, this makes target selection equivalent to
`--all-files` while retaining the recursive target-selection behavior. It is also available as
`no_submodules = true` in project configuration.

Both `--all` and `--all-files` accept an optional directory: `forformat --all-files ./src`. In that
form the directory's Git checkout is discovered, configuration is loaded relative to that directory,
and only matching tracked free-form sources beneath it are selected. Without the directory, the
current checkout is used.

Project configuration accepts `context_paths = ["..."]`, `no_submodules = true`,
`exclude = ["..."]`, and `extend_exclude = ["..."]` in `.forformat.toml` or `[tool.forformat]` in
`pyproject.toml`. `project-context` remains command-line-only. The default exclusion set is empty;
`exclude` replaces it and `extend_exclude` adds to it. Command-line `--context-path` values replace
configured `context_paths` rather than accumulating with them.

Because `exclude` selects a set rather than adding to one, it does not accumulate across sources the
way the other repeatable options do: a command-line `--exclude` discards the configuration file's
`exclude` entirely, exactly as it discards the built-in defaults. `--extend-exclude` is the additive
spelling and stacks on top of both.

Patterns match repository-relative paths: `*` does not cross `/`, `**` does, and `?` matches one
non-`/` character. A trailing `/` matches a directory prefix and all descendants. A leading `/`
anchors at the repository root; patterns without it are tried at each path-component boundary.
Relative paths are normalized to `/` before matching.

Every file target is read before formatting and classified by an evidence-based fixed/free detector.
Strong fixed-form evidence wins; otherwise clear free-form syntax is accepted. When a filename is
available, modern suffixes such as `.f90` and `.F90` add a strong free-form prior, while bare `.f`
and `.F` remain content-driven. Anonymous stdin has no filename prior, so it is formatted when the
bytes contain positive free-form evidence and remains conservatively fixed when the content is
ambiguous. Fixed-form sources are skipped: their bytes are not written, they do not make `--check`
fail, and `--stdout` returns the original bytes. Each skipped target receives a
`forformat: <path>: fixed-form source, skipped` diagnostic on stderr. Use `-ifree` or
`--input-format=free` when a source uses free form despite a legacy-looking layout; this forces
free-form handling and bypasses detection. `-iauto` and `--input-format=auto` select the default
automatic behavior. `--query-format` prints one `free` or `fixed` result per input without
formatting or modifying it.

All project source bytes are loaded into memory before analysis. One `ProjectContext` is then built
and shared by every target in the invocation. In-place replacement writes a same-directory
temporary, copies the original mode bits, fsyncs, and renames over the resolved symlink target.

Every target is formatted before any of them is written, so a formatting failure leaves the tree
untouched rather than half rewritten. The diagnostic names the input the same way a skip or a
declined wrap does — `forformat: <path>: <reason>`, or `<stdin>` for an anonymous buffer — because
that path is the only thing distinguishing the one failing file from the rest of the selection.
`--exclude` is the way to proceed past a file the formatter refuses.
