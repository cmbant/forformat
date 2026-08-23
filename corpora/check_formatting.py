#!/usr/bin/env python3
"""Run formatter/idempotency checks over selected Git repositories."""

from __future__ import annotations

import argparse
import difflib
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

# Several large Fortran projects use uppercase suffixes for free-form sources
# that are preprocessed by their build systems (notably CP2K and Q-E).  The
# comparison in `tracked_sources` lowercases suffixes, so `.f` covers `.F`.
SOURCE_SUFFIXES = {".f", ".f03", ".f08", ".f18", ".f23", ".f90", ".f95"}

FULL = ["--full", "--no-config"]
INDENT_ONLY = ["--indent-only", "--no-config"]
# Bound each formatter invocation so a regression cannot leave an overnight
# corpus run waiting forever.  A timeout is reported as an ordinary failure.
COMMAND_TIMEOUT_SECONDS = 120


PROFILES: dict[str, list[str]] = {
    "full-config": ["--full"],
    "full-plain": FULL,
    "no-wrap": [*FULL, "--no-wrap"],
    "line80": [*FULL, "--line-length=80"],
    "line100": [*FULL, "--line-length=100"],
    "line160": [*FULL, "--line-length=160"],
    "align-paren": [*FULL, "--align-paren"],
    "align-paren8": [*FULL, "--align-paren=8"],
    "indent2": [*FULL, "--indent=2"],
    "indent4": [*FULL, "--indent=4"],
    "indent8": [*FULL, "--indent=8"],
    "normalize-only": ["--normalize-only", "--no-config"],
    "canonicalize-only": ["--canonicalize-only", "--no-config"],
    "indent-only": INDENT_ONLY,
    # New full-mode style controls.  The explicit-on profile checks the
    # command-line wiring, while the off profiles exercise each independent
    # gate and the interactions that are easy to make non-idempotent.
    "style-explicit-on": [
        *FULL,
        "--keyword-case=lower",
        "--relational-symbols=1",
        "--array-brackets=1",
        "--compact-multiplicative=1",
        "--join-goto=1",
        "--split-compound-keywords=1",
        "--strip-empty-args=1",
        "--remove-redundant-parens=1",
        "--remove-terminal-return=1",
        "--program-unit-spacing=1",
        "--max-blank-lines=2",
        "--delimiter-spacing=1",
        "--comment-spacing=1",
        "--continuation-markers=1",
    ],
    "style-all-off": [
        *FULL,
        "--keyword-case=preserve",
        "--relational-symbols=0",
        "--array-brackets=0",
        "--compact-multiplicative=0",
        "--join-goto=0",
        "--split-compound-keywords=0",
        "--strip-empty-args=0",
        "--remove-redundant-parens=0",
        "--remove-terminal-return=0",
        "--program-unit-spacing=0",
        "--max-blank-lines=preserve",
        "--delimiter-spacing=0",
        "--comment-spacing=0",
        "--continuation-markers=0",
    ],
    "keyword-upper": [*FULL, "--keyword-case=upper"],
    "keyword-preserve": [*FULL, "--keyword-case=preserve"],
    "relational-off": [*FULL, "--relational-symbols=0"],
    "array-brackets-off": [*FULL, "--array-brackets=0"],
    "multiplicative-spaced": [*FULL, "--compact-multiplicative=0"],
    "goto-split": [*FULL, "--join-goto=0"],
    "compound-unsplit": [*FULL, "--split-compound-keywords=0"],
    "empty-args-kept": [*FULL, "--strip-empty-args=0"],
    "redundant-parens-kept": [*FULL, "--remove-redundant-parens=0"],
    "terminal-return-kept": [*FULL, "--remove-terminal-return=0"],
    "unit-spacing-off": [*FULL, "--program-unit-spacing=0"],
    "blank-preserve": [*FULL, "--max-blank-lines=preserve"],
    "blank-zero": [*FULL, "--max-blank-lines=0"],
    "blank-one": [*FULL, "--max-blank-lines=1"],
    "delimiter-spacing-off": [*FULL, "--delimiter-spacing=0"],
    "comment-spacing-off": [*FULL, "--comment-spacing=0"],
    "continuation-markers-off": [*FULL, "--continuation-markers=0"],
    # Combinations deliberately mix several new controls so that a later pass
    # cannot be tested only from the default starting representation.
    "style-lex-spacing-layout": [
        *FULL,
        "--keyword-case=preserve",
        "--relational-symbols=0",
        "--array-brackets=0",
        "--compact-multiplicative=0",
        "--delimiter-spacing=0",
        "--comment-spacing=0",
        "--continuation-markers=0",
        "--line-length=80",
        "--indent=8",
        "--align-paren=4",
    ],
    "style-structure-layout": [
        *FULL,
        "--keyword-case=upper",
        "--split-compound-keywords=0",
        "--join-goto=0",
        "--strip-empty-args=0",
        "--remove-redundant-parens=0",
        "--remove-terminal-return=0",
        "--program-unit-spacing=0",
        "--max-blank-lines=1",
        "--line-length=100",
        "--indent=4",
        "--ws-remred=0",
        "--align-comments=1",
    ],
    "style-default-layout": [
        *FULL,
        "--keyword-case=lower",
        "--relational-symbols=1",
        "--array-brackets=1",
        "--compact-multiplicative=1",
        "--join-goto=1",
        "--split-compound-keywords=1",
        "--strip-empty-args=1",
        "--remove-redundant-parens=1",
        "--remove-terminal-return=1",
        "--program-unit-spacing=1",
        "--max-blank-lines=2",
        "--delimiter-spacing=1",
        "--comment-spacing=1",
        "--continuation-markers=1",
        "--line-length=80",
        "--indent=8",
        "--indent-continuation=6",
        "--align-paren=8",
        "--align-comments=1",
    ],
    # Older layout controls are included because style switches interact with
    # the layout engine and these are common alternate starting points.
    "ws-remred-off": [*FULL, "--ws-remred=0"],
    "ws-remred-on": [*FULL, "--ws-remred=1"],
    "align-declarations-off": [*FULL, "--align-declarations=0"],
    "align-comments-on": [*FULL, "--align-comments=1"],
    "findent-layout-deep": [
        *FULL,
        "--indent=8",
        "--start-indent=2",
        "--indent-continuation=6",
        "--indent-ampersand",
        "--align-paren=4",
        "--ws-remred=1",
        "--align-declarations=1",
        "--align-comments=1",
        "--indent-contains=restart",
        "--openmp=1",
        "--max-indent=32",
    ],
    "findent-layout-legacy": [
        *FULL,
        "--indent=4",
        "--start-indent=2",
        "--indent-continuation=none",
        "--align-paren=0",
        "--ws-remred=0",
        "--align-declarations=0",
        "--align-comments=1",
        "--include-left=1",
        "--label-left=1",
        "--openmp=0",
    ],
}

# These profiles cover option branches that are useful independently of the
# broad style/layout combinations above.  Keep them as named profiles rather
# than multiplying the whole matrix: the corpus is an I1/I2 stress test, not a
# parser-value Cartesian product.
PROFILES.update(
    {
        # Style passes also run in normalize-only mode, where layout and
        # wrapping cannot mask a non-idempotent normalization rule.
        "normalize-style-on": [
            "--normalize-only",
            "--no-config",
            *PROFILES["style-explicit-on"][2:],
        ],
        "normalize-style-off": [
            "--normalize-only",
            "--no-config",
            *PROFILES["style-all-off"][2:],
        ],
        # Indent-only must cover the layout engine without full-mode
        # normalization or wrapping in front of it.
        "indent-only-layout-edge": [
            *INDENT_ONLY,
            "--indent=8",
            "--start-indent=auto",
            "--max-indent=0",
            "--indent-continuation=0",
            "--indent-contains=4",
            "--indent-ampersand",
            "--align-paren=8",
            "--align-comments=1",
        ],
        "indent-only-constructs": [
            *INDENT_ONLY,
            "--indent=4",
            "--indent-associate=1",
            "--indent-block=2",
            "--indent-changeteam=3",
            "--indent-critical=4",
            "--indent-case=5",
            "--indent-do=6",
            "--indent-entry=1",
            "--indent-enum=2",
            "--indent-forall=3",
            "--indent-if=4",
            "--indent-interface=5",
            "--indent-module=6",
            "--indent-procedure=1",
            "--indent-select=2",
            "--indent-type=3",
            "--indent-where=4",
            "--indent-contains=5",
        ],
        # Exercise the wrapper against the less common layout boundaries and
        # a preserved authored indentation profile.
        "full-layout-edge": [
            *FULL,
            "--line-length=80",
            "--indent=8",
            "--start-indent=auto",
            "--max-indent=0",
            "--indent-continuation=0",
            "--indent-contains=4",
            "--indent-ampersand",
            "--align-paren=8",
            "--align-declarations=1",
            "--align-comments=1",
        ],
        "full-indent-none": [
            *FULL,
            "--indent=none",
            "--line-length=80",
            "--align-paren=8",
            "--align-comments=1",
        ],
        "full-left-options-off": [
            *FULL,
            "--line-length=80",
            "--include-left=0",
            "--label-left=0",
            "--openmp=0",
        ],
        # A style profile with wrapping disabled catches interactions that the
        # default wrapped profiles cannot reach.
        "style-all-off-no-wrap": [
            *FULL,
            "--no-wrap",
            *PROFILES["style-all-off"][2:],
        ],
        "style-lex-remred-layout": [
            *PROFILES["style-lex-spacing-layout"],
            "--ws-remred=1",
            "--align-declarations=1",
            "--align-comments=1",
        ],
        # These options change source bytes and also participate in case
        # resolution, so they deserve a real full-mode corpus pass.
        "full-refactor-macros": [
            *FULL,
            "--line-length=80",
            "--keyword-case=upper",
            "--refactor-end=upcase",
            "--uppercase-single-l",
            "--define=USE_MPI",
            "--define=REAL_KIND=8",
        ],
        # Verify command-line values override a discovered project config
        # (where one exists) while retaining the ordinary corpus invocation.
        "config-cli-overrides": [
            "--full",
            "--line-length=80",
            "--indent=8",
            "--indent-module=2",
            "--keyword-case=upper",
            "--wrap=1",
        ],
        # `invoke` selects explicit paths for this profile; all other profiles
        # intentionally retain project-wide context via `--all`.
        "full-isolated": [*FULL, "--isolated"],
    }
)


SEQUENCES: dict[str, list[list[str]]] = {
    "line80-indent8": [PROFILES["line80"], PROFILES["indent8"]],
    "full-indentonly": [PROFILES["full-plain"], PROFILES["indent-only"]],
    "indentonly-full": [PROFILES["indent-only"], PROFILES["full-plain"]],
    "normalize-full": [PROFILES["normalize-only"], PROFILES["full-plain"]],
    "canonicalize-full": [
        PROFILES["canonicalize-only"],
        PROFILES["full-plain"],
    ],
    "full-canonicalize": [
        PROFILES["full-plain"],
        PROFILES["canonicalize-only"],
    ],
    "style-off-style-on": [PROFILES["style-all-off"], PROFILES["style-explicit-on"]],
    "style-mixed-layout": [
        PROFILES["style-lex-spacing-layout"],
        PROFILES["style-structure-layout"],
    ],
    "findent-layout-full": [PROFILES["findent-layout-deep"], PROFILES["full-plain"]],
    "full-findent-layout": [PROFILES["full-plain"], PROFILES["findent-layout-legacy"]],
    "upper-lower": [PROFILES["keyword-upper"], PROFILES["full-plain"]],
    "line80-line160": [PROFILES["line80"], PROFILES["line160"]],
    "blank-preserve-blank-zero": [PROFILES["blank-preserve"], PROFILES["blank-zero"]],
    "normalize-style-off-on": [
        PROFILES["normalize-style-off"],
        PROFILES["normalize-style-on"],
    ],
    "full-plain-layout-edge": [PROFILES["full-plain"], PROFILES["full-layout-edge"]],
    "style-off-no-wrap": [PROFILES["style-all-off"], PROFILES["style-all-off-no-wrap"]],
}


@dataclass
class ProfileResult:
    repo: str
    name: str
    args: list[str]
    first_rc: int
    second_rc: int
    check_rc: int
    changed_count: int
    unstable: list[str]


@dataclass
class SequenceResult:
    repo: str
    name: str
    stages: list[list[str]]
    return_codes: list[int]
    unstable: list[str]


@dataclass
class OracleMismatch:
    repo: str
    path: str
    source: bytes
    expected: bytes
    actual: bytes
    error: str | None = None


def run(
    command: list[str], *, cwd: Path | None = None, data: bytes | None = None
) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(
            command,
            cwd=cwd,
            input=data,
            capture_output=True,
            check=False,
            timeout=COMMAND_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.output or b""
        stderr = error.stderr or b""
        if isinstance(stdout, str):
            stdout = stdout.encode()
        if isinstance(stderr, str):
            stderr = stderr.encode()
        stderr += (
            f"\nforformat corpus command timed out after {COMMAND_TIMEOUT_SECONDS} seconds"
        ).encode()
        return subprocess.CompletedProcess(command, 124, stdout, stderr)


def git(repo: Path, *args: str) -> subprocess.CompletedProcess[bytes]:
    return run(["git", "-C", str(repo), *args])


def restore(repo: Path) -> None:
    result = git(repo, "restore", "--source=HEAD", "--worktree", "--", ".")
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace").strip())
    result = git(
        repo,
        "submodule",
        "foreach",
        "--recursive",
        "git restore --source=HEAD --worktree -- .",
    )
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace").strip())


def clean_status(repo: Path) -> str:
    result = git(
        repo,
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--ignore-submodules=none",
    )
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace").strip())
    return result.stdout.decode(errors="replace")


def tracked_sources(repo: Path, binary: Path) -> list[Path]:
    result = git(repo, "ls-files", "--recurse-submodules", "-z", "--")
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace").strip())
    paths = []
    for raw in result.stdout.split(b"\0"):
        if not raw:
            continue
        path = Path(os.fsdecode(raw))
        if path.suffix.lower() in SOURCE_SUFFIXES:
            paths.append(path)
    paths = sorted(paths)
    if not paths:
        return paths
    query = run(
        [str(binary), "--query-format", *(path.as_posix() for path in paths)],
        cwd=repo,
    )
    if query.returncode:
        raise RuntimeError(
            f"format detector failed in {repo}: rc={query.returncode}; "
            f"{query.stderr.decode(errors='replace').strip()}"
        )
    forms = query.stdout.splitlines()
    if len(forms) != len(paths) or any(
        form not in {b"free", b"fixed"} for form in forms
    ):
        raise RuntimeError(
            f"format detector returned {len(forms)} results for {len(paths)} sources in {repo}"
        )
    return [path for path, form in zip(paths, forms) if form == b"free"]


def read_sources(repo: Path, sources: list[Path]) -> dict[str, bytes]:
    return {relative.as_posix(): (repo / relative).read_bytes() for relative in sources}


def changed_paths(before: dict[str, bytes], after: dict[str, bytes]) -> list[str]:
    return sorted(path for path in after if before.get(path) != after[path])


def invoke(
    binary: Path,
    repo: Path,
    sources: list[Path],
    args: list[str],
    log: Path,
    *,
    check: bool = False,
) -> int:
    command = [str(binary), *args]
    if check:
        command.append("--check")
    if "--isolated" in args:
        command.extend(path.as_posix() for path in sources)
    else:
        command.append("--all")
    result = run(command, cwd=repo)
    log.parent.mkdir(parents=True, exist_ok=True)
    log.with_name(f"{log.name}.stdout").write_bytes(result.stdout)
    log.with_name(f"{log.name}.stderr").write_bytes(result.stderr)
    return result.returncode


def profile(
    binary: Path,
    repo: Path,
    sources: list[Path],
    name: str,
    args: list[str],
    report: Path,
) -> tuple[ProfileResult, dict[str, tuple[bytes, bytes]]]:
    restore(repo)
    before = read_sources(repo, sources)
    first_rc = invoke(binary, repo, sources, args, report / f"{name}.first")
    after_first = read_sources(repo, sources)
    second_rc = invoke(binary, repo, sources, args, report / f"{name}.second")
    after_second = read_sources(repo, sources)
    check_rc = invoke(binary, repo, sources, args, report / f"{name}.check", check=True)
    unstable = changed_paths(after_first, after_second)
    unstable_states = {
        path: (after_first[path], after_second[path]) for path in unstable
    }
    return (
        ProfileResult(
            repo.name,
            name,
            args,
            first_rc,
            second_rc,
            check_rc,
            len(changed_paths(before, after_first)),
            unstable,
        ),
        unstable_states,
    )


def sequence(
    binary: Path,
    repo: Path,
    sources: list[Path],
    name: str,
    stages: list[list[str]],
    report: Path,
) -> tuple[SequenceResult, dict[str, tuple[bytes, bytes]]]:
    restore(repo)
    snapshots: list[dict[str, bytes]] = []
    return_codes: list[int] = []
    for stage_number, args in enumerate(stages, 1):
        return_codes.append(
            invoke(binary, repo, sources, args, report / f"{name}.stage{stage_number}")
        )
        snapshots.append(read_sources(repo, sources))
    # The stages deliberately create a non-default starting point.  Only the
    # final stage is checked for I1 from that point; replaying the whole
    # sequence would test whether two different style configurations commute,
    # which is neither required nor useful.
    final_args = stages[-1]
    return_codes.append(
        invoke(binary, repo, sources, final_args, report / f"{name}.probe1")
    )
    snapshots.append(read_sources(repo, sources))
    return_codes.append(
        invoke(binary, repo, sources, final_args, report / f"{name}.probe2")
    )
    snapshots.append(read_sources(repo, sources))
    return_codes.append(
        invoke(binary, repo, sources, final_args, report / f"{name}.check", check=True)
    )
    unstable = changed_paths(snapshots[-2], snapshots[-1])
    unstable_states = {
        path: (snapshots[-2][path], snapshots[-1][path]) for path in unstable
    }
    return SequenceResult(
        repo.name, name, stages, return_codes, unstable
    ), unstable_states


def oracle(
    binary: Path, findent: str, repo: Path, sources: list[Path]
) -> tuple[int, list[OracleMismatch]]:
    mismatches: list[OracleMismatch] = []
    errors = 0
    for relative in sources:
        source = (repo / relative).read_bytes()
        reference = run([findent, "-ifree"], data=source)
        rust = run(
            [
                str(binary),
                "--indent-only",
                "--no-config",
                "--isolated",
                "--stdout",
                str(relative),
            ],
            cwd=repo,
        )
        if reference.returncode or rust.returncode:
            errors += 1
            mismatches.append(
                OracleMismatch(
                    repo.name,
                    relative.as_posix(),
                    source,
                    reference.stdout,
                    rust.stdout,
                    f"findent rc={reference.returncode}, forformat rc={rust.returncode}; "
                    f"stderr: {rust.stderr.decode(errors='replace').strip()}",
                )
            )
        elif reference.stdout != rust.stdout:
            mismatches.append(
                OracleMismatch(
                    repo.name,
                    relative.as_posix(),
                    source,
                    reference.stdout,
                    rust.stdout,
                )
            )
    return errors, mismatches


def discover(root: Path, requested: list[str]) -> list[Path]:
    candidates = sorted(
        path for path in root.iterdir() if path.is_dir() and (path / ".git").exists()
    )
    if not requested:
        return candidates
    selected = []
    for item in requested:
        path = Path(item)
        if not path.is_absolute():
            path = root / path if (root / path).exists() else Path.cwd() / path
        path = path.resolve()
        if not path.is_dir() or not (path / ".git").exists():
            raise SystemExit(f"not a Git repository: {item}")
        selected.append(path)
    return selected


def default_binary(root: Path) -> Path:
    configured = os.environ.get("FORFORMAT")
    candidates = [Path(configured)] if configured else []
    candidates += [
        root.parent / "target/release/forformat",
        root.parent / "target/debug/forformat",
    ]
    found = shutil.which("forformat")
    if found:
        candidates.append(Path(found))
    for candidate in candidates:
        if candidate and candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate.resolve()
    raise SystemExit("forformat binary not found; set FORFORMAT or pass --binary")


def short_diff(
    left: bytes, right: bytes, left_name: str, right_name: str, limit: int = 80
) -> str:
    left_lines = left.decode(errors="replace").splitlines()
    right_lines = right.decode(errors="replace").splitlines()
    diff = list(
        difflib.unified_diff(
            left_lines, right_lines, fromfile=left_name, tofile=right_name, lineterm=""
        )
    )
    if len(diff) <= limit:
        return "\n".join(diff)
    return "\n".join([*diff[:limit], f"... ({len(diff) - limit} diff lines omitted)"])


def shell_args(args: list[str]) -> str:
    return " ".join(args)


def write_markdown(
    destination: Path,
    binary: Path,
    profiles: list[ProfileResult],
    sequences: list[SequenceResult],
    oracle_errors: dict[str, int],
    mismatches: list[OracleMismatch],
    profile_states: dict[tuple[str, str], dict[str, tuple[bytes, bytes]]],
    sequence_states: dict[tuple[str, str], dict[str, tuple[bytes, bytes]]],
) -> None:
    failures = [
        result
        for result in profiles
        if result.first_rc or result.second_rc or result.check_rc or result.unstable
    ]
    failures += [
        result for result in sequences if any(result.return_codes) or result.unstable
    ]
    destination.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Corpus formatting test failures",
        "",
        "Generated by `corpora/check_formatting.py`.",
        "",
        f"Binary: `{binary}`",
        "",
        (
            "The harness restores each corpus checkout from its current `HEAD` before every profile "
            "or sequence. A profile is a failure if formatting errors, `--check` errors, or changes "
            "between the first and second application. A sequence first applies its stages once to "
            "create a starting point, then checks the final stage twice from that point."
        ),
        "",
        "## Summary",
        "",
        f"- Profile failures: **{len(failures)}**",
        (
            f"- `findent -ifree` mismatches/errors: **{len(mismatches)}** "
            f"({sum(oracle_errors.values())} command errors)"
        ),
        "",
    ]
    if not failures and not mismatches:
        lines += ["No failures found.", ""]
    else:
        lines += [
            "## Profile and sequence failures",
            "",
            "| Repository | Test | Return codes | Unstable files |",
            "| --- | --- | --- | --- |",
        ]
        for result in failures:
            if isinstance(result, ProfileResult):
                codes = (
                    f"{result.first_rc}, {result.second_rc}, check {result.check_rc}"
                )
                unstable = ", ".join(f"`{path}`" for path in result.unstable) or "—"
            else:
                codes = ", ".join(map(str, result.return_codes))
                unstable = ", ".join(f"`{path}`" for path in result.unstable) or "—"
            lines.append(
                f"| `{result.repo}` | `{result.name}` | `{codes}` | {unstable} |"
            )
        lines.append("")

        for result in failures:
            lines += [f"### `{result.repo}` / `{result.name}`", ""]
            if isinstance(result, ProfileResult):
                lines += [
                    (
                        f"Command: `forformat {shell_args(result.args)} "
                        f"{'<explicit paths>' if '--isolated' in result.args else '--all'}`"
                    ),
                    "",
                    (
                        f"Return codes: first `{result.first_rc}`, second `{result.second_rc}`, "
                        f"check `{result.check_rc}`; files changed on first pass: `{result.changed_count}`."
                    ),
                    "",
                ]
                states = profile_states[(result.repo, result.name)]
                for path in result.unstable[:8]:
                    lines += [
                        f"#### `{path}`",
                        "",
                        "The second application changed the first output:",
                        "",
                        "```diff",
                        short_diff(
                            states[path][0],
                            states[path][1],
                            "first pass",
                            "second pass",
                        ),
                        "```",
                        "",
                        "Reproduce with:",
                        "",
                        f"```sh\nforformat {shell_args(result.args)} --stdout {path}\n```",
                        "",
                    ]
                if not result.unstable and (
                    result.first_rc or result.second_rc or result.check_rc
                ):
                    lines += [
                        (
                            "See the corresponding `.stderr` log beside the harness report directory "
                            "for the command diagnostic."
                        ),
                        "",
                    ]
            else:
                lines += [
                    "Stages (applied in order):",
                    "",
                    *[
                        f"{number}. `forformat {shell_args(args)} --all`"
                        for number, args in enumerate(result.stages, 1)
                    ],
                    "",
                    (
                        f"Return codes (stages, probe 1, probe 2, check): "
                        f"`{', '.join(map(str, result.return_codes))}`."
                    ),
                    "",
                ]
                states = sequence_states[(result.repo, result.name)]
                for path in result.unstable[:8]:
                    lines += [
                        f"#### `{path}`",
                        "",
                        (
                            "The final stage changed its first result when applied a second time from "
                            "the sequence-created starting point:"
                        ),
                        "",
                        "```diff",
                        short_diff(
                            states[path][0],
                            states[path][1],
                            "final stage probe 1",
                            "final stage probe 2",
                        ),
                        "```",
                        "",
                    ]

        lines += ["## `--indent-only` versus findent", ""]
        if not mismatches:
            lines += ["No mismatches or command errors.", ""]
        else:
            lines += [
                (
                    "Each example compares `findent -ifree` with `forformat --indent-only --no-config "
                    "--isolated --stdout <path>`. The diff is intentionally capped to keep examples "
                    "short; the path names the complete tracked input."
                ),
                "",
            ]
            for mismatch in mismatches:
                lines += [f"### `{mismatch.repo}` / `{mismatch.path}`", ""]
                if mismatch.error:
                    lines += [f"Command error: `{mismatch.error}`", ""]
                else:
                    lines += [
                        "```diff",
                        short_diff(
                            mismatch.expected, mismatch.actual, "findent", "forformat"
                        ),
                        "```",
                        "",
                    ]
                lines += [
                    "Reproduce with:",
                    "",
                    (
                        f"```sh\nfindent -ifree < {mismatch.path}\n"
                        f"forformat --indent-only --no-config --isolated --stdout {mismatch.path}\n```"
                    ),
                    "",
                ]
    destination.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        type=Path,
        help="forformat executable (default: target/release or target/debug)",
    )
    parser.add_argument(
        "--findent",
        default=shutil.which("findent"),
        help="findent executable for oracle comparison",
    )
    parser.add_argument(
        "--repo",
        action="append",
        default=[],
        help="repository name/path; repeatable (default: repositories under corpora/)",
    )
    parser.add_argument(
        "--profile",
        action="append",
        choices=sorted(PROFILES),
        help="profile to run; repeatable",
    )
    parser.add_argument("--skip-sequences", action="store_true")
    parser.add_argument("--skip-oracle", action="store_true")
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="run despite local changes; tracked changes are restored",
    )
    parser.add_argument("--report-dir", type=Path, help="directory for command logs")
    parser.add_argument(
        "--markdown-report",
        type=Path,
        help="failure report path (default: docs/corpora-formatting-failures.md)",
    )
    options = parser.parse_args()

    root = Path(__file__).resolve().parent
    binary = (options.binary or default_binary(root)).resolve()
    repositories = discover(root, options.repo)
    profiles = options.profile or list(PROFILES)
    report_root = (
        options.report_dir or Path(tempfile.mkdtemp(prefix="forformat-corpora-"))
    ).resolve()
    markdown_report = (
        options.markdown_report or root.parent / "docs/corpora-formatting-failures.md"
    ).resolve()
    report_root.mkdir(parents=True, exist_ok=True)
    failed = False
    profile_results: list[ProfileResult] = []
    sequence_results: list[SequenceResult] = []
    profile_states: dict[tuple[str, str], dict[str, tuple[bytes, bytes]]] = {}
    sequence_states: dict[tuple[str, str], dict[str, tuple[bytes, bytes]]] = {}
    oracle_errors: dict[str, int] = {}
    oracle_mismatches: list[OracleMismatch] = []

    print(f"binary: {binary}")
    print(f"logs: {report_root}")
    print(f"markdown: {markdown_report}")
    for repo in repositories:
        if not options.allow_dirty:
            status = clean_status(repo)
            if status:
                raise SystemExit(
                    f"dirty repository (use --allow-dirty to override): {repo}\n{status}"
                )
        sources = tracked_sources(repo, binary)
        repo_report = report_root / repo.name
        repo_report.mkdir(parents=True, exist_ok=True)
        print(f"\n{repo.name}: {len(sources)} free-form sources")
        try:
            for name in profiles:
                result, states = profile(
                    binary, repo, sources, name, PROFILES[name], repo_report
                )
                profile_results.append(result)
                profile_states[(repo.name, name)] = states
                failed |= bool(
                    result.first_rc
                    or result.second_rc
                    or result.check_rc
                    or result.unstable
                )
                print(
                    f"  {name:28} rc={result.first_rc},{result.second_rc} check={result.check_rc} "
                    f"stable={'yes' if not result.unstable else 'NO'} changed={result.changed_count}"
                )
            if not options.skip_sequences:
                for name, stages in SEQUENCES.items():
                    result, states = sequence(
                        binary, repo, sources, name, stages, repo_report
                    )
                    sequence_results.append(result)
                    sequence_states[(repo.name, name)] = states
                    failed |= bool(any(result.return_codes) or result.unstable)
                    print(
                        f"  sequence {name:20} rc={','.join(map(str, result.return_codes))} "
                        f"stable={'yes' if not result.unstable else 'NO'}"
                    )
            if not options.skip_oracle:
                if options.findent:
                    errors, mismatches = oracle(binary, options.findent, repo, sources)
                    oracle_errors[repo.name] = errors
                    oracle_mismatches.extend(mismatches)
                    failed |= bool(errors or mismatches)
                    print(
                        f"  findent oracle             errors={errors} mismatches={len(mismatches)}"
                    )
                    if mismatches:
                        print(f"    first mismatch: {mismatches[0].path}")
                else:
                    print("  findent oracle             skipped (findent not found)")
        finally:
            restore(repo)

    write_markdown(
        markdown_report,
        binary,
        profile_results,
        sequence_results,
        oracle_errors,
        oracle_mismatches,
        profile_states,
        sequence_states,
    )
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
