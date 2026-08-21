# Resolve Cargo's target directory the way Cargo itself resolves it.
#
# CARGO_TARGET_DIR is only one of the inputs. `build.target-dir` in
# $CARGO_HOME/config.toml is just as authoritative, and this devcontainer uses
# it to keep artifacts off the /workspaces mount: that is v9fs, whose mtimes
# come from the Windows host clock, and Cargo decides freshness by comparing
# mtimes. A bare "target" default under such a config would not merely fail to
# find the binary — it would find the one the previous layout left behind and
# check that instead, which is the stale-artifact bug in a new place.
#
# Sourced, not executed. Prints the directory; never fails.
cargo_target_dir() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        printf '%s\n' "$CARGO_TARGET_DIR"
        return 0
    fi
    cargo metadata --format-version 1 --no-deps 2>/dev/null |
        sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' |
        grep . || printf 'target\n'
}
