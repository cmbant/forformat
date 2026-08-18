"""Build the Python wheel around a separately compiled forformat binary."""

from __future__ import annotations

import os
import re
import shutil
import stat
import subprocess
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


def _release_directory() -> Path:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target_dir.is_absolute():
        target_dir = ROOT / target_dir
    target = os.environ.get("CARGO_BUILD_TARGET")
    if target:
        target_dir /= target
    return target_dir / "release"


def _build_release_binary() -> Path:
    cargo = os.environ.get("CARGO") or "cargo"
    command = [cargo, "build", "--locked", "--release"]
    target = os.environ.get("CARGO_BUILD_TARGET")
    if target:
        command.extend(("--target", target))
    try:
        subprocess.run(command, cwd=ROOT, check=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        raise RuntimeError(
            "could not build the forformat release binary with Cargo. "
            "Install Rust and Cargo, or set FORFORMAT_BINARY to a prebuilt executable."
        ) from exc
    return _release_directory()


def binary_path() -> Path:
    configured = os.environ.get("FORFORMAT_BINARY")
    if configured:
        candidate = Path(configured)
        if candidate.is_file():
            return candidate
        raise RuntimeError(
            f"FORFORMAT_BINARY does not point to a file: {candidate}. "
            "Set it to a prebuilt executable or unset it to build with Cargo."
        )

    release_directory = _release_directory()
    candidates = [release_directory / name for name in ("forformat", "forformat.exe")]
    for candidate in candidates:
        if candidate.is_file():
            return candidate

    release_directory = _build_release_binary()
    candidates = [release_directory / name for name in ("forformat", "forformat.exe")]
    for candidate in candidates:
        if candidate.is_file():
            return candidate

    searched = ", ".join(str(candidate) for candidate in candidates)
    raise RuntimeError(
        "Cargo completed but did not produce a forformat release binary; "
        f"searched {searched}. Install Rust and Cargo, or set FORFORMAT_BINARY to a prebuilt executable."
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
            destination.chmod(
                destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
            )


class bdist_wheel_with_binary(bdist_wheel):
    """Mark the wheel platform-specific because it contains native code."""

    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False

    def get_tag(self) -> tuple[str, str, str]:
        _, _, platform = super().get_tag()
        override = os.environ.get("FORFORMAT_WHEEL_PLATFORM_TAG")
        return "py3", "none", override or platform


setup(
    version=cargo_version(),
    cmdclass={"build_py": build_py_with_binary, "bdist_wheel": bdist_wheel_with_binary},
)
