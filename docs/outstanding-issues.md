# Compatibility inputs outside the supported contract

This page records the input families referenced by [compatibility.md](compatibility.md) that remain
outside the supported free-form compatibility contract. It exists primarily to keep that contract
explicit; checked-in fixtures and the corpus audit are the source of truth for individual cases and
counts.

Current families include:

- FYPP template bodies whose template syntax is not valid Fortran in isolation.
- Preprocessor configurations whose conditional branches open and close Fortran constructs
  asymmetrically.
- Inputs using non-Fortran operators or syntax from another language.
- COCO (`??`) and FYPP (`#:`) directives beyond the safe grouping/continuation behaviour documented
  in [compatibility.md](compatibility.md).

These cases are not silently promoted to supported behaviour. A family should leave this page only
when it has a checked-in fixture, an explicit compatibility decision, and the corresponding
regression coverage.
