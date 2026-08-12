"""Patched reference used to validate the governing-declaration port.

The historical oracle remains ``standardize_fortran.py``.  This module loads
that file and changes only declaration-case resolution; the default differential
checks continue to load the frozen module.
"""

from __future__ import annotations

import dataclasses
import importlib.util
import re
import sys
from collections.abc import Collection, Iterable, Mapping
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "patched_frozen_standardize_fortran", HERE / "standardize_fortran.py"
)
assert _spec and _spec.loader
_frozen = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = _frozen
_spec.loader.exec_module(_frozen)

# Export the frozen public and test-facing helpers.  The selected internal
# hooks below are replaced in the frozen module's globals as well, so functions
# such as the frozen ``format_text`` and ``main`` use the patched helpers.
for _name in dir(_frozen):
    if not _name.startswith("__"):
        globals()[_name] = getattr(_frozen, _name)


def _validated_fortran_path(path: Path) -> Path:
    """Validate the suffix before consulting the filesystem.

    A caller that will read the file performs the existence check after this
    suffix check.  Keeping the two validations separate lets callers test
    extension acceptance independently of filesystem state.
    """
    resolved = path.resolve()
    if resolved.suffix.lower() not in FORTRAN_SOURCE_EXTENSIONS:
        extensions = ", ".join(sorted(FORTRAN_SOURCE_EXTENSIONS))
        raise ValueError(f"Expected a free-form Fortran source ({extensions}): {resolved}")
    return resolved


_frozen_declared_variable_names = _frozen._declared_variable_names


class OwnerProcedureCases(dict):
    """Owner-keyed bindings with the frozen flat map retained privately."""

    def __init__(self, owner_cases: Mapping[tuple[str, str], str], fallback: Mapping[str, str]):
        super().__init__(owner_cases)
        self.fallback = dict(fallback)


def _declared_variable_names(statement: str) -> list[str]:
    """Include objects declared with ``type(T) ::`` in their scope.

    The frozen helper excluded every ``type(...)`` declaration, which dropped
    ordinary typed locals such as ``type(EvolutionVars) :: EVOut``.  Derived
    type definitions are already excluded by ``_declared_type_name``.
    """
    if _frozen._declared_type_name(statement) or _frozen.PROCEDURE_WORD.search(statement):
        return _frozen_declared_variable_names(statement)
    if _frozen.TYPE_CLASS_CONTEXT.match(statement):
        separator = statement.find("::")
        if separator >= 0:
            entities = statement[separator + 2 :]
        else:
            old_style = _frozen.OLD_STYLE_DECLARATION.match(statement)
            if old_style is None:
                old_style = re.match(
                    r"^\s*(?:integer|real|double\s+precision|complex|logical|character)"
                    r"\s*\([^)]*\)\s*(?P<entities>.+)$",
                    statement,
                    re.IGNORECASE,
                )
            if old_style is None:
                return []
            entities = old_style.group("entities") if "entities" in old_style.groupdict() else old_style.group(1)
    else:
        separator = statement.find("::")
        if separator >= 0:
            entities = statement[separator + 2 :]
        else:
            old_style = _frozen.OLD_STYLE_DECLARATION.match(statement)
            if old_style is None:
                old_style = re.match(
                    r"^\s*(?:integer|real|double\s+precision|complex|logical|character)"
                    r"\s*\([^)]*\)\s*(?P<entities>.+)$",
                    statement,
                    re.IGNORECASE,
                )
            if old_style is None:
                return []
            entities = old_style.group("entities") if "entities" in old_style.groupdict() else old_style.group(1)
    names = []
    for entity in _frozen._split_top_level(entities):
        match = _frozen.DECLARATION_ENTITY.match(entity)
        if match:
            names.append(match.group(1))
    return names


def _is_declaration_entity(line: str, index: int, *, context: str | None = None) -> bool:
    """Whether a token is the entity head, not an array bound or initializer."""
    context = _frozen.code_context(line) if context is None else context
    declaration_start = context.rfind("::", 0, index)
    if declaration_start < 0:
        return False
    item_start = declaration_start + 2
    depth = 0
    for position in range(item_start, index):
        char = context[position]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth = max(0, depth - 1)
        elif char == "," and depth == 0:
            item_start = position + 1
    match = re.search(r"[A-Za-z_][A-Za-z0-9_]*", context[item_start:index + 1])
    return bool(match and item_start + match.start() <= index < item_start + match.end())


def is_contextual_identifier(line: str, index: int, *, context: str | None = None) -> bool:
    """Keep declaration entity heads protected, but resolve declaration uses."""
    context = _frozen.code_context(line) if context is None else context
    previous = index - 1
    while previous >= 0 and context[previous].isspace():
        previous -= 1
    if previous >= 0 and context[previous] == "%":
        return True
    return _is_declaration_entity(line, index, context=context)


def _top_level_cases(source: str) -> dict[str, str]:
    """Collect declarations in an implicit/top-level program unit."""
    procedures = _frozen.extract_procedure_cases(source)
    statements = _frozen._code_statements(source)
    program_scope = None
    if statements:
        header = _frozen._scope_header(statements[0].text)
        if header and header[0] == "program":
            program_scope = next(
                (procedure for procedure in procedures if procedure.start_line == statements[0].start_line),
                None,
            )
    if program_scope is not None:
        return dict(program_scope.local_cases)
    occurrences: dict[str, list[str]] = {}
    type_depth = 0
    for statement in statements:
        text = statement.text
        type_name = _frozen._declared_type_name(text)
        if type_name:
            type_depth += 1
            continue
        if type_depth:
            if _frozen.TYPE_DEFINITION_END.match(text):
                type_depth -= 1
            continue
        if _frozen.active_procedure_at(procedures, statement.start_line):
            continue
        for name in _declared_variable_names(text):
            occurrences.setdefault(name.lower(), []).append(name)
    return _frozen._resolve_case_occurrences(occurrences)


def _owner_type_procedure_cases(sources: Mapping[Path, str]) -> dict[tuple[str, str], str]:
    """Resolve type-bound bindings by their owning derived type."""
    occurrences: dict[tuple[str, str], list[str]] = {}
    for source in sources.values():
        type_stack: list[str] = []
        for statement in _frozen._code_statements(source):
            text = statement.text
            type_name = _frozen._declared_type_name(text)
            if type_name:
                type_stack.append(type_name)
                continue
            if not type_stack:
                continue
            if _frozen.TYPE_DEFINITION_END.match(text):
                type_stack.pop()
                continue
            if _frozen.DECLARATION_PROCEDURE_START.match(text):
                for name in _declared_variable_names(text):
                    occurrences.setdefault((type_stack[-1].lower(), name.lower()), []).append(name)
    return {
        key: spellings[0]
        for key, spellings in occurrences.items()
        if len(set(spellings)) == 1
    }


_frozen._declared_variable_names = _declared_variable_names
_frozen.is_contextual_identifier = is_contextual_identifier
_original_collect = _frozen.collect_declaration_cases
_original_format_text = _frozen.format_text


def collect_declaration_cases(
    sources: Mapping[Path, str],
    target_paths: Collection[Path] | None = None,
) -> dict[Path, FileDeclarationCases]:
    cases = _original_collect(sources, target_paths=target_paths)
    owner_cases = _owner_type_procedure_cases(sources)
    updated: dict[Path, FileDeclarationCases] = {}
    for path, case in cases.items():
        local_cases = _top_level_cases(sources[path])
        symbols = dict(case.symbol_cases)
        symbols.update(local_cases)
        type_cases = OwnerProcedureCases(owner_cases, case.type_procedure_cases)
        updated[path] = dataclasses.replace(
            case,
            symbol_cases=symbols,
            type_procedure_cases=type_cases,
        )
    return updated


def _apply_owner_cases(
    source: str,
    owner_cases: Mapping[tuple[str, str], str],
    procedure_cases: Iterable[ProcedureDeclarationCases],
    variable_type_cases: Mapping[str, str],
    type_component_type_cases: Mapping[tuple[str, str], str],
) -> str:
    """Apply only owner-resolved member cases before ordinary formatting."""
    if not owner_cases:
        return source
    procedures = tuple(procedure_cases) or _frozen.extract_procedure_cases(source)
    active = _frozen._active_procedures_by_line(procedures, len(source.splitlines(keepends=True)))
    variable_types = {**variable_type_cases, **_frozen.extract_variable_types(source)}
    output: list[str] = []
    for line_number, line in enumerate(source.splitlines(keepends=True)):
        context = _frozen.code_context(line)
        replacements: list[tuple[int, int, str]] = []
        for match in re.finditer(r"[A-Za-z_][A-Za-z0-9_]*", context):
            if match.start() == 0 or context[match.start() - 1] != "%":
                continue
            owner = _frozen.member_owner_type(
                context,
                match.start(),
                local_types=active[line_number].local_types if active[line_number] else {},
                variable_types=variable_types,
                type_component_types=type_component_type_cases,
            )
            spelling = owner_cases.get((owner.lower(), match.group().lower())) if owner else None
            if spelling:
                replacements.append((match.start(), match.end(), spelling))
        for start, end, spelling in reversed(replacements):
            line = line[:start] + spelling + line[end:]
        output.append(line)
    return "".join(output)


def format_text(
    original: str,
    wrap: bool = True,
    module_cases: Mapping[str, str] | None = None,
    symbol_cases: Mapping[str, str] | None = None,
    procedure_cases: Iterable[ProcedureDeclarationCases] = (),
    scope_cases: Iterable[NamedScopeCase] = (),
    type_procedure_cases: Mapping[str, str] | None = None,
    type_component_cases: Mapping[tuple[str, str], str] | None = None,
    variable_type_cases: Mapping[str, str] | None = None,
    type_component_type_cases: Mapping[tuple[str, str], str] | None = None,
    uppercase_single_l: bool = False,
    macro_cases: Mapping[str, str] | Collection[str] = (),
) -> str:
    owner_cases = {
        key: value
        for key, value in (type_procedure_cases or {}).items()
        if isinstance(key, tuple) and len(key) == 2
    }
    fallback_type_cases = getattr(type_procedure_cases, "fallback", {})
    fallback_type_cases = {
        key: value
        for key, value in (type_procedure_cases or {}).items()
        if isinstance(key, str)
    } or fallback_type_cases
    source = _apply_owner_cases(
        original,
        owner_cases,
        procedure_cases,
        variable_type_cases or {},
        type_component_type_cases or {},
    )
    # The flat map is deliberately not passed through: an unresolved member
    # must not inherit a same-named binding from another derived type.
    formatted = _original_format_text(
        source,
        wrap=wrap,
        module_cases=module_cases,
        symbol_cases=symbol_cases,
        procedure_cases=procedure_cases,
        scope_cases=scope_cases,
        type_procedure_cases=fallback_type_cases,
        type_component_cases=type_component_cases,
        variable_type_cases=variable_type_cases,
        type_component_type_cases=type_component_type_cases,
        uppercase_single_l=uppercase_single_l,
        macro_cases=macro_cases,
    )
    return _apply_owner_cases(
        formatted,
        owner_cases,
        procedure_cases,
        variable_type_cases or {},
        type_component_type_cases or {},
    )


_frozen.collect_declaration_cases = collect_declaration_cases
_frozen.format_text = format_text
_frozen.lowercase_file.__globals__["format_text"] = format_text
