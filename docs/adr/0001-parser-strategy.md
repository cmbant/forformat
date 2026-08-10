# ADR 0001: Handwritten scanner and classifier

## Decision

Use a handwritten byte scanner and ordered statement recognizers. Do not add a parser generator or
`tree-sitter-fortran` dependency.

## Rationale

The formatter needs statement boundaries and a small amount of structural information, not an AST,
expression parsing, name resolution, or semantic diagnostics. A generated parser would add a C build
step and complicate static musl and Windows builds, while tree-sitter would parse more of Fortran
than the formatter needs and would increase binary size and startup cost. Editor buffers also contain
truncated statements, so total recognizers with an `Unknown` fallback are easier to constrain.

The source is scanned as bytes to preserve arbitrary non-UTF-8 comments and strings. Scanner facts
are shared by assembly, classification, and transformations.

