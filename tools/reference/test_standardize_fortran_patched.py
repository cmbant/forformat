"""Repository-authored checks for the patched reference module.

The CAMB test suite remains frozen in ``test_standardize_fortran.py``.  These
checks exercise only the governing-declaration corrections used for the Rust
port and are not additional CAMB oracle rows.
"""

import unittest
from pathlib import Path

from tools.reference.standardize_fortran_patched import (
    collect_declaration_cases,
    format_text,
)


class PatchedSpacingTests(unittest.TestCase):
    def test_type_bound_procedures_only_supply_component_case(self) -> None:
        sources = {
            Path("type.f90"): """\
module type_module
    type :: State
    contains
        procedure :: BuildValue
    end type State
end module type_module
""",
            Path("use.f90"): """\
call buildvalue()
state%buildvalue()
""",
        }
        cases = collect_declaration_cases(sources)[Path("use.f90")]
        self.assertNotIn("buildvalue", cases.symbol_cases)
        self.assertEqual(
            cases.type_procedure_cases,
            {("state", "buildvalue"): "BuildValue"},
        )
        self.assertEqual(
            format_text(
                sources[Path("use.f90")],
                wrap=False,
                symbol_cases=cases.symbol_cases,
                type_procedure_cases=cases.type_procedure_cases,
            ),
            """\
call buildvalue()
State%BuildValue()
""",
        )

    def test_governing_type_owner_beats_flat_binding_ambiguity(self) -> None:
        sources = {
            Path("types.f90"): """\
module types
    type :: ThermoData
    contains
        procedure :: values
    end type ThermoData
    type :: OtherData
    contains
        procedure :: Values
    end type OtherData
end module types
""",
            Path("use.f90"): """\
type(ThermoData) :: data
call data%Values()
""",
        }
        cases = collect_declaration_cases(sources)[Path("use.f90")]
        self.assertEqual(
            format_text(
                sources[Path("use.f90")],
                wrap=False,
                type_procedure_cases=cases.type_procedure_cases,
                variable_type_cases=cases.variable_type_cases,
                type_component_type_cases=cases.type_component_type_cases,
            ),
            """\
type(ThermoData) :: data
call data%values()
""",
        )

    def test_old_style_local_declaration_beats_project_case(self) -> None:
        sources = {
            Path("other.f90"): "module other\n    real :: PK\nend module other\n",
            Path("use.f90"): """\
subroutine load
    real(dl) kh, Pk
    read *, PK
end subroutine load
""",
        }
        cases = collect_declaration_cases(sources)[Path("use.f90")]
        self.assertEqual(
            format_text(
                sources[Path("use.f90")],
                wrap=False,
                symbol_cases=cases.symbol_cases,
                procedure_cases=cases.procedure_cases,
            ),
            """\
subroutine load
    real(dl) kh, Pk
    read *, Pk

end subroutine load
""",
        )

    def test_top_level_parameter_beats_project_case(self) -> None:
        sources = {
            Path("other.f90"): "module other\n    integer, parameter :: BJL_recurrence_MAX_L = 25\nend module other\n",
            Path("use.f90"): """\
integer, parameter :: BJL_RECURRENCE_MAX_L = 25
if (l > bjl_recurrence_max_l) then
end if
""",
        }
        cases = collect_declaration_cases(sources)[Path("use.f90")]
        self.assertEqual(
            format_text(
                sources[Path("use.f90")],
                wrap=False,
                symbol_cases=cases.symbol_cases,
            ),
            """\
integer, parameter :: BJL_RECURRENCE_MAX_L = 25
if (l > BJL_RECURRENCE_MAX_L) then
end if
""",
        )
