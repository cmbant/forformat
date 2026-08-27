"""In-memory Python interface to the bundled :command:`forformat` binary."""

from __future__ import annotations

import os
import subprocess
import warnings
from collections.abc import Sequence
from typing import Optional, Union, overload

from forformat_runner.launcher import bundled_binary

__all__ = ["ForformatError", "ForformatWarning", "format_source"]

Source = Union[str, bytes]
Pathish = Union[str, os.PathLike]


class ForformatError(RuntimeError):
    """Raised when the native formatter rejects an in-memory request."""

    def __init__(self, returncode: int, stderr: str) -> None:
        self.returncode = returncode
        self.stderr = stderr
        message = stderr.strip() or f"forformat exited with status {returncode}"
        super().__init__(message)


class ForformatWarning(UserWarning):
    """A non-fatal diagnostic emitted by the native formatter."""


def _uses_explicit_config(options: Sequence[str]) -> bool:
    for option in options:
        name = option.split("=", 1)[0].replace("_", "-").lower()
        if name in ("--config", "--no-config"):
            return True
    return False


@overload
def format_source(
    source: str,
    *,
    options: Sequence[str] = (),
    filename: Optional[Pathish] = None,
    repo_context_path: Optional[Pathish] = None,
) -> str: ...


@overload
def format_source(
    source: bytes,
    *,
    options: Sequence[str] = (),
    filename: Optional[Pathish] = None,
    repo_context_path: Optional[Pathish] = None,
) -> bytes: ...


def format_source(
    source: Source,
    *,
    options: Sequence[str] = (),
    filename: Optional[Pathish] = None,
    repo_context_path: Optional[Pathish] = None,
) -> Source:
    """Format one string or byte buffer and return the same input type.

    ``options`` accepts formatter and explicit configuration options from the
    command line. Project configuration discovery is disabled unless a
    ``--config`` option is supplied. ``filename`` says which source file the
    in-memory buffer represents; it supplies filename-aware source detection,
    relative INCLUDE resolution, diagnostics, and the default Git project for
    analysis. ``repo_context_path`` may override that project with an existing
    directory in another Git checkout. It does not change ``filename`` or
    configuration selection.
    """

    if not isinstance(source, (str, bytes)):
        raise TypeError("source must be str or bytes")
    if isinstance(options, str):
        raise TypeError("options must be a sequence of complete option strings")
    arguments = list(options)
    if not all(isinstance(option, str) for option in arguments):
        raise TypeError("options must contain only strings")

    command = [os.fspath(bundled_binary()), "--stdin"]
    if filename is not None:
        command.extend(("--stdin-filename", os.fspath(filename)))
    if repo_context_path is not None:
        command.extend(("--project-context", os.fspath(repo_context_path)))
    if not _uses_explicit_config(arguments):
        command.append("--no-config")
    command.extend(arguments)

    is_text = isinstance(source, str)
    encoded = source.encode("utf-8") if is_text else source
    completed = subprocess.run(command, input=encoded, capture_output=True, check=False)
    stderr = completed.stderr.decode("utf-8", errors="replace")
    if completed.returncode != 0:
        raise ForformatError(completed.returncode, stderr)
    if stderr:
        warnings.warn(stderr.rstrip(), ForformatWarning, stacklevel=2)
    if is_text:
        return completed.stdout.decode("utf-8")
    return completed.stdout
