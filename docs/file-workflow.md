# File and project workflow

The command-line workflow discovers the repository with git rev-parse and
enumerates tracked free-form sources with git ls-files. Both commands pass
through one helper that removes GIT_DIR, GIT_WORK_TREE, GIT_COMMON_DIR, and
GIT_INDEX_FILE, so a hook's Git environment cannot redirect nested queries.

The all-files mode follows the reference tool's intentional forutils/ asymmetry: tracked
forutils/ sources are included in rewrite, check, and diff targets, and they
are also included in the project case-resolution context. The Rust workflow
has repeatable `--exclude=<glob>` and `--extend-exclude=<glob>` options that remove matching
tracked sources from both sets. An excluded source named explicitly remains a formatting target;
exclusions are not force exclusions.

`--all` accepts an optional directory: `forformat --all ./src`. In that form the directory's Git
checkout is discovered, configuration is loaded relative to that directory, and only tracked
free-form sources beneath it are selected. Without the directory, the current checkout retains
the original behavior.

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
