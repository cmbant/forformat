# File and project workflow

The command-line workflow discovers the repository with git rev-parse and
enumerates tracked free-form sources with git ls-files. Both commands pass
through one helper that removes GIT_DIR, GIT_WORK_TREE, GIT_COMMON_DIR, and
GIT_INDEX_FILE, so a hook's Git environment cannot redirect nested queries.

The `--all-files` mode follows the reference tool's intentional forutils/ asymmetry: tracked
sources in the current checkout, including `forutils/`, are included in rewrite, check, and diff
targets. Initialized submodule sources are not targets, but are included in the project
case-resolution context. The `--all` mode is recursive and includes submodule sources as targets.
Both modes have repeatable `--exclude=<glob>` and `--extend-exclude=<glob>` options that
remove matching tracked sources from targets and project context. An excluded source named
explicitly remains a formatting target; exclusions are not force exclusions.

`--show-files` prints the selected target paths and exits without reading or modifying source
files. It accepts explicit paths, `--all`, or `--all-files`; either bulk mode accepts one optional
directory, for example `forformat --all-files ./src --show-files`.

`--no-submodules` disables recursive submodule discovery entirely. The superproject's tracked
sources remain available for project context, but initialized submodule sources are neither
targets nor context. With `--all`, this makes target selection equivalent to `--all-files` while
retaining the recursive target-selection behavior.

Both `--all` and `--all-files` accept an optional directory: `forformat --all-files ./src`. In that
form the directory's Git checkout is discovered, configuration is loaded relative to that
directory, and only matching tracked free-form sources beneath it are selected. Without the
directory, the current checkout is used.

Project configuration accepts `exclude = ["..."]` and `extend-exclude = ["..."]` arrays in
`.forformat.toml` or `[tool.forformat]` in `pyproject.toml`. The default exclusion set is empty;
`exclude` replaces it and `extend-exclude` adds to it.

Because `exclude` selects a set rather than adding to one, it does not accumulate across sources
the way the other repeatable options do: a command-line `--exclude` discards the configuration
file's `exclude` entirely, exactly as it discards the built-in defaults. `--extend-exclude` is the
additive spelling and stacks on top of both. This is implemented in `parse`
([`cli.rs`](../src/cli.rs)), which drops the generated `--exclude=` arguments before merging
configuration into argv, using the command-line-only parse to decide.

Patterns match repository-relative paths:
`*` does not cross `/`, `**` does, and `?` matches one non-`/` character. A trailing `/` matches a
directory prefix and all descendants. A leading `/` anchors at the repository root; patterns
without it are tried at each path-component boundary. Relative paths are normalized to `/` before
matching.

All project source bytes are loaded into memory before analysis. One
ProjectContext is then built and shared by every target in the invocation.
In-place replacement writes a same-directory temporary, copies the original
mode bits, fsyncs, and renames over the resolved symlink target.
