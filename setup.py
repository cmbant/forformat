"""Build the Python wheel around a separately compiled forformat binary."""

from __future__ import annotations

import os
import re
import shutil
import stat
from pathlib import Path

from setuptools import setup
from setuptools.command.build_py import build_py
from wheel.bdist_wheel import bdist_wheel


ROOT = Path(__file__).resolve().parent


def cargo_version() -> str:
    cargo_toml = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"\s*$', cargo_toml, re.MULTILINE)
    if match is None:
        raise RuntimeError("could not read package version from Cargo.toml")
    return match.group(1)


def binary_path() -> Path:
    configured = os.environ.get("FORFORMAT_BINARY")
    candidates = [Path(configured)] if configured else []
    candidates.extend(
        ROOT / path
        for path in (
            "target/release/forformat",
            "target/release/forformat.exe",
        )
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    searched = ", ".join(str(candidate) for candidate in candidates)
    raise RuntimeError(
        "forformat wheel builds require a prebuilt release binary; "
        f"searched {searched}. Run `cargo build --locked --release` first or set FORFORMAT_BINARY."
    )


class build_py_with_binary(build_py):
    """Put the platform's already-built executable into the wheel."""

    def run(self) -> None:
        super().run()
        source = binary_path()
        destination_dir = Path(self.build_lib) / "forformat_runner" / "bin"
        destination_dir.mkdir(parents=True, exist_ok=True)
        destination = destination_dir / source.name
        shutil.copy2(source, destination)
        if os.name != "nt":
            destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


class bdist_wheel_with_binary(bdist_wheel):
    """Mark the wheel platform-specific because it contains native code."""

    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False

    def get_tag(self) -> tuple[str, str, str]:
        _, _, platform = super().get_tag()
        return "py3", "none", platform


setup(
    version=cargo_version(),
    cmdclass={"build_py": build_py_with_binary, "bdist_wheel": bdist_wheel_with_binary},
)
