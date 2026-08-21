#!/usr/bin/env bash
set -euo pipefail

binary=${1:-${CARGO_TARGET_DIR:-target}/debug/forformat}
case "$binary" in
    /*) ;;
    *) binary="$(pwd)/$binary" ;;
esac
test -x "$binary"

die() {
    echo "documentation check: $*" >&2
    exit 1
}

# Keep long options advertised by terminal help represented in the normal-usage reference.
# This is intentionally not an exhaustive parser or compatibility-alias inventory.
help_options=$($binary --help | grep -oE -- '--[a-z][a-z0-9-]*' | sort -u)
while IFS= read -r option; do
    test -z "$option" && continue
    grep -Fq -- "$option" docs/options.md || die "docs/options.md does not mention $option"
done <<<"$help_options"

# These normal long forms are summarized rather than individually expanded in --help.
for option in \
    --indent-associate --indent-block --indent-case --indent-contains --indent-do \
    --indent-entry --indent-enum --indent-forall --indent-if --indent-interface \
    --indent-module --indent-procedure --indent-select --indent-type --indent-where \
    --indent-critical --indent-changeteam; do
    grep -Fq -- "$option" docs/options.md || die "docs/options.md does not mention $option"
done

# Reject the stale fixed/free wording that previously appeared in user-facing docs.
if grep -Fqi 'automatic format detection are not supported' README.md README_PYPI.md docs/*.md; then
    die "stale automatic-format-detection wording"
fi

# Check local Markdown links without adding a documentation dependency.
# CI runners provide `python`; a bare Debian/devcontainer image only has `python3`.
python_bin=${PYTHON:-python}
command -v "$python_bin" > /dev/null 2>&1 || python_bin=python3
"$python_bin" - <<'PY'
from pathlib import Path
import re
import sys

files = [Path("README.md"), Path("README_PYPI.md"), *sorted(Path("docs").glob("*.md"))]
failed = []
for source in files:
    text = source.read_text(encoding="utf-8")
    for target in re.findall(r"\[[^\]]+\]\(([^)]+)\)", text):
        if target.startswith(("http://", "https://", "mailto:", "#")):
            continue
        path = target.split("#", 1)[0]
        if not path:
            continue
        candidate = (source.parent / path).resolve()
        if not candidate.exists():
            failed.append(f"{source}: missing local link target {target}")
if failed:
    print("\n".join(failed), file=sys.stderr)
    raise SystemExit(1)
PY

# Exercise the formatter commands shown in the quick-start/reference docs in an isolated Git repo.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/src" "$tmp/modules"
cd "$tmp"
git init -q
git config user.email docs@example.invalid
git config user.name docs

cat > src/module.f90 <<'F90'
MODULE Demo
CONTAINS
SUBROUTINE run()
INTEGER::x
IF(x.eq.1)THEN
PRINT *,x
ENDIF
END SUBROUTINE run
END MODULE Demo
F90
cat > modules/types.f90 <<'F90'
MODULE Types
INTEGER::SharedValue
END MODULE Types
F90
git add src/module.f90 modules/types.f90

# In-place path formatting.
"$binary" src/module.f90 >/dev/null
"$binary" modules/types.f90 >/dev/null
"$binary" --all-files --check >/dev/null

# stdin/stdout examples.
"$binary" --stdin < src/module.f90 > stdin.out
"$binary" --stdout src/module.f90 > stdout.out
cmp stdin.out stdout.out

# Common formatting examples.
"$binary" --indent=4 --indent-module=0 --indent-procedure=0 src/module.f90 >/dev/null
"$binary" --keyword-case=upper --line-length=100 src/module.f90 >/dev/null
grep -q '^MODULE Demo$' src/module.f90

# Editor/project-context and context-path examples.
"$binary" --stdin --project-context=src/module.f90 < src/module.f90 > project.out
"$binary" --stdout src/module.f90 --context-path=src --context-path=modules > context.out
cmp project.out context.out

# Selection/query examples.
"$binary" --all-files --show-files > files.out
grep -Fxq 'src/module.f90' files.out
grep -Fxq 'modules/types.f90' files.out
"$binary" --all-files --show-files --exclude='modules/' > excluded.out
grep -Fxq 'src/module.f90' excluded.out
! grep -q 'modules/types.f90' excluded.out
"$binary" --query-format src/module.f90 | grep -Fxq free

# Configuration example and underscore/hyphen key equivalence.
cat > .forformat.toml <<'TOML'
mode = "full"
indent = 4
indent_module = 0
indent-procedure = 0
line_length = 100
keyword_case = "lower"
context_paths = ["src", "modules"]
exclude = ["vendor/"]
extend_exclude = ["**/generated-*.f90"]
TOML
"$binary" --stdout src/module.f90 > configured.out
grep -q '^module Demo$' configured.out

echo "Documentation checks passed for $binary"
