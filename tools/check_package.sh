#!/bin/sh
set -eu

target_dir=${CARGO_TARGET_DIR:-target}
package_dir="$target_dir/package"

cargo package --locked --allow-dirty >/dev/null
archive=$(find "$package_dir" -maxdepth 1 -type f -name 'findent-*.crate' | sort | tail -n 1)
test -n "$archive"
package_root="$package_dir/findent-0.1.0"
test -f "$package_root/Cargo.toml"
first_hash=$(sha256sum "$archive" | awk '{print $1}')

cargo package --locked --allow-dirty >/dev/null
archive_again=$(find "$package_dir" -maxdepth 1 -type f -name 'findent-*.crate' | sort | tail -n 1)
test "$archive_again" = "$archive"
second_hash=$(sha256sum "$archive_again" | awk '{print $1}')
test "$first_hash" = "$second_hash"

# Cargo's package verification compiles the unpacked crate. Run its complete
# test/bench target set too, so the archive—not only the workspace—must remain
# self-contained and executable.
cargo test --manifest-path "$package_root/Cargo.toml" --locked --all-targets >/dev/null
echo "package check: unpacked crate tests passed"

cargo build --manifest-path "$package_root/Cargo.toml" --locked --release >/dev/null
./tools/check_cli_contract.sh "$package_root/target/release/findent"
echo "package check: unpacked release binary passed the CLI contract"

tar -tzf "$archive" | grep -F 'findent-0.1.0/Cargo.toml' >/dev/null
tar -tzf "$archive" | grep -F 'findent-0.1.0/src/main.rs' >/dev/null
tar -tzf "$archive" | grep -F 'findent-0.1.0/tests/fixtures/construct_options.f90' >/dev/null
tar -tzf "$archive" | grep -F 'findent-0.1.0/tests/fixtures/align_legacy_full.f90' >/dev/null
tar -tzf "$archive" | grep -F 'findent-0.1.0/tests/fixtures/legacy_free_matrix.f90' >/dev/null
tar -tzf "$archive" | grep -F 'findent-0.1.0/tools/check_release.sh' >/dev/null
tar -tzf "$archive" | grep -F 'findent-0.1.0/tools/check_package.sh' >/dev/null
if tar -tzf "$archive" | grep -E '/\.git($|/)' >/dev/null; then
    echo "package check: archive contains .git data" >&2
    exit 1
fi

bytes=$(wc -c < "$archive")
echo "package check: archive=$archive bytes=$bytes sha256=$first_hash"
