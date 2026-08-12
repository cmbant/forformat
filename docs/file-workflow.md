# File and project workflow

The command-line workflow discovers the repository with git rev-parse and
enumerates tracked free-form sources with git ls-files. Both commands pass
through one helper that removes GIT_DIR, GIT_WORK_TREE, GIT_COMMON_DIR, and
GIT_INDEX_FILE, so a hook's Git environment cannot redirect nested queries.

The all-files mode follows the reference tool's intentional forutils/ asymmetry: tracked
forutils/ sources are included in rewrite, check, and diff targets, and they
are also included in the project case-resolution context. The Rust workflow
has no separate pre-commit exclusion mode; keeping the all-files mode faithful
to the reference is the least surprising choice, while explicit positional paths let
a hook select only its staged fortran/ files.

All project source bytes are loaded into memory before analysis. One
ProjectContext is then built and shared by every target in the invocation.
In-place replacement writes a same-directory temporary, copies the original
mode bits, fsyncs, and renames over the resolved symlink target.
