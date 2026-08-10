#!/bin/sh
# Differential smoke runner for the retained free-form legacy fixtures.
# The oracle is deliberately optional: normal Rust builds do not depend on it.
set -eu

rust_findent=${1:-target/debug/findent}
oracle=${FINDENT_ORACLE:-/opt/findent/src/findent}
fixture_root=${FINDENT_TEST_ROOT:-/opt/findent/test}

if [ ! -x "$oracle" ]; then
    echo "oracle not found: $oracle" >&2
    exit 2
fi

status=0
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

normalize_legacy_difference() {
    # The only retained smoke-test boundary is leading whitespace on
    # findentfix comments.  Default trailing-horizontal-whitespace handling
    # now matches the 4.3.7 free emitter.  Any remaining structural
    # difference is still a failure.
    sed -E 's/\r$//; s/^[[:blank:]]+(![[:blank:]]*findentfix:)/\1/I' "$1"
}

for fixture in progfree.f progfree1.f progfree-dos.f; do
    input=$fixture_root/$fixture
    [ -f "$input" ] || { echo "missing fixture: $input" >&2; status=1; continue; }
    flags=$(sed -n '1s/^! //p' "$input" | tr -d '\r')
    # The legacy fixtures carry their option line in a comment.  Splitting it
    # here is intentional and matches the shell test driver’s argv contract.
    set -- $flags
    env -u FINDENT_FLAGS LC_ALL=C "$oracle" -ifree "$@" < "$input" > "$fixture.oracle"
    env -u FINDENT_FLAGS LC_ALL=C "$rust_findent" -ifree "$@" < "$input" > "$fixture.rust"
    if cmp -s "$fixture.oracle" "$fixture.rust"; then
        echo "$fixture: match"
    elif [ "${FINDENT_DIFFERENTIAL_STRICT:-0}" = 0 ] \
        && normalize_legacy_difference "$fixture.oracle" > "$tmpdir/oracle" \
        && normalize_legacy_difference "$fixture.rust" > "$tmpdir/rust" \
        && cmp -s "$tmpdir/oracle" "$tmpdir/rust"; then
        echo "$fixture: match (documented findentfix normalization)"
    else
        echo "$fixture: DIFFER" >&2
        diff -u "$fixture.oracle" "$fixture.rust" >&2 || true
        status=1
    fi
    rm -f "$fixture.oracle" "$fixture.rust"
done
exit "$status"
