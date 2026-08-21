#!/bin/sh
set -eu

. "$(dirname "$0")/target_dir.sh"

target_dir=$(cargo_target_dir)

# `cargo package` verifies by compiling the unpacked crate, and with no
# --target-dir it does that in the *workspace's* target directory. That build
# overwrites the workspace's own fingerprint for this package and repoints it at
# the unpacked copy's sources, which never change again: every later `cargo
# build` here then reports "Finished ... up to date" and serves a stale binary
# until `cargo clean -p forformat`. Give the packaging run its own directory so
# it cannot reach the artifacts the rest of the bar is checking.
packaging_dir="$target_dir/packaging"
package_dir="$packaging_dir/package"

# Everything version-specific comes from cargo, so a version bump is a one-line
# change in Cargo.toml. `cargo pkgid` prints `…#forformat@<version>`.
version=$(cargo pkgid | sed 's/.*[#@]//')
test -n "$version"
crate="forformat-$version"

cargo package --locked --allow-dirty --target-dir "$packaging_dir" >/dev/null
archive="$package_dir/$crate.crate"
test -f "$archive"
package_root="$package_dir/$crate"
test -f "$package_root/Cargo.toml"
package_target_dir="$package_root/target"
first_hash=$(sha256sum "$archive" | awk '{print $1}')

cargo package --locked --allow-dirty --target-dir "$packaging_dir" >/dev/null
test -f "$archive"
second_hash=$(sha256sum "$archive" | awk '{print $1}')
test "$first_hash" = "$second_hash"

# Cargo's package verification compiles the unpacked crate. Run its complete
# test/bench target set too, so the archive—not only the workspace—must remain
# self-contained and executable.
# Keep this verification build inside the unpacked crate. The workspace's
# CARGO_TARGET_DIR may contain a different binary from the outer checkout.
cargo test --target-dir "$package_target_dir" \
    --manifest-path "$package_root/Cargo.toml" --locked --all-targets >/dev/null
echo "package check: unpacked crate tests passed"

cargo build --target-dir "$package_target_dir" \
    --manifest-path "$package_root/Cargo.toml" --locked --release >/dev/null
./tools/check_cli_contract.sh "$package_target_dir/release/forformat"
echo "package check: unpacked release binary passed the CLI contract"

listing=$(tar -tzf "$archive")
for entry in Cargo.toml \
             LICENSE \
             LICENSE-THIRD-PARTY \
             NOTICE \
             src/main.rs \
             tests/fixtures/construct_options.f90 \
             tests/fixtures/align_legacy_full.f90 \
             tests/fixtures/legacy_free_matrix.f90 \
             tools/check_release.sh \
             tools/check_package.sh; do
    if ! printf '%s\n' "$listing" | grep -Fx "$crate/$entry" >/dev/null; then
        echo "package check: archive is missing $entry" >&2
        exit 1
    fi
done
if tar -tzf "$archive" | grep -E '/\.git($|/)' >/dev/null; then
    echo "package check: archive contains .git data" >&2
    exit 1
fi

bytes=$(wc -c < "$archive")
echo "package check: archive=$archive bytes=$bytes sha256=$first_hash"
