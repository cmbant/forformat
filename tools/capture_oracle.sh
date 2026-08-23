#!/bin/sh
# Explicitly capture a findent 4.3.8~pre01 golden. Normal tests never invoke the oracle.
set -eu

oracle=${FINDENT_ORACLE:-/opt/findent/src/findent}
if [ "$#" -lt 2 ]; then
    echo "usage: $0 INPUT OUTPUT [findent-options...]" >&2
    exit 2
fi
input=$1
output=$2
shift 2

[ -x "$oracle" ] || { echo "oracle not found: $oracle" >&2; exit 2; }
env -u FINDENT_FLAGS LC_ALL=C "$oracle" -ifree "$@" < "$input" > "$output"
