"""Delegate the console command to the native binary shipped in this wheel."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def bundled_binary() -> Path:
    binary_dir = Path(__file__).resolve().parent / "bin"
    for name in ("forformat.exe", "forformat"):
        candidate = binary_dir / name
        if candidate.is_file():
            return candidate
    raise RuntimeError("the installed forformat wheel has no bundled native binary")


def main() -> int:
    command = [os.fspath(bundled_binary()), *sys.argv[1:]]
    completed = subprocess.run(command, check=False)
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
