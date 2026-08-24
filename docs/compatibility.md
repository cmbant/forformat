# Compatibility with findent

`forformat` accepts **free-form Fortran only**, over stdin/stdout or file paths. `--indent-only` is
a byte-for-byte compatibility contract against `findent -ifree`; `--full` (the default) adds lexical
normalization and wrapping on top of it. The reference build is findent 4.3.8~pre01.

A difference in `--indent-only` output is treated as a bug in this crate, except for the cases
listed under [Reviewed indentation differences](#reviewed-indentation-differences).

## What is supported

Global and per-construct indentation, start indent, continuation policies, labels, includes,
OpenMP free-form sentinels, CPP branch snapshots, `findentfix:` directives, maximum indentation,
`END` refactoring, whitespace reduction, and the `last-indent`/`last-usable` queries.

In `--indent-only`, only leading indentation is replaced and trailing spaces and tabs are trimmed.
Everything else — source spelling, body bytes, spaces inside strings and comments — is preserved. A
missing final line terminator is added, matching the preceding line (LF for a one-line unterminated
file). LF, CRLF, and mixed terminators are preserved as they are.

The parser is a shallow structural classifier, not a full Fortran semantic parser. Unknown or
incomplete statements are emitted conservatively rather than guessed at.

## Deliberate differences from findent

Three are unconditional:

- **Fixed-form is rejected, not formatted.** `-ifixed`, `--input-format=fixed`, `-ofixed`, and
  `--output-format=fixed` all fail with status 2. Automatic detection may still classify a source
  as fixed-form, in which case it is skipped unchanged rather than rewritten.
- **Unknown options fail** with status 2, so a misspelled option is visible instead of ignored.
- **`FINDENT_FLAGS` is not read.** Configuration comes only from the command line, a project config
  file, or the library API.

## Fixed/free detection

Automatic detection is enabled by default. Unlike findent, which stops at the first decisive line,
`forformat` accumulates positive fixed- and free-form evidence across the whole file. Strong fixed
evidence wins.

- **Suffix.** A modern suffix (`.f90`, `.F90`, `.f95`, and so on) is a strong free-form prior;
  `.f` and `.F` are decided purely on content. Anonymous stdin has no suffix, so it is free only
  when the bytes carry clear free-form evidence, and conservatively fixed when they are ambiguous.
- **Column six.** A nonblank, nonzero character there is fixed-form continuation evidence. `&`
  counts on its own. Markers that could equally begin a free-form statement indented five spaces —
  a letter, an underscore (a macro call such as `_ABORT(...)`), `!`, and label-like digit runs —
  count only when the preceding line is itself incomplete.
- **Preprocessor.** Literal `#if 0` and `#if 1` branches are resolved, and `#elif` chains track
  which branch was already taken. Evidence from a branch whose condition depends on a macro is held
  aside and used only if those branches agree on free form; conflicting alternatives stay fixed.

`--query-format` reports the verdict without formatting. `-ifree` or `--input-format=free` forces
free-form handling — the right answer when a source's form is genuinely undecidable from its bytes
and suffix.

## Reviewed indentation differences

These are the known `--indent-only` divergences. All three turn on whether a statement is a
procedure heading at all, which is where a lexer-driven recognizer and a structural classifier can
legitimately disagree.

**A FUNCTION or SUBROUTINE statement in a program-unit body with no `CONTAINS`.** findent opens a
frame for a nested definition only after `INTERFACE` or `CONTAINS`, so elsewhere it leaves the body
and its matching `END` at host depth. `forformat` opens the frame and indents the body. Inside a
`MODULE` body both open a frame.

```fortran
subroutine outer
   integer :: x
   subroutine inner    ! forformat indents the body below this;
   integer :: y        ! findent leaves it at host depth
   end subroutine inner
end subroutine outer
```

**A comma between a type specification and a heading's prefix attributes.** A `prefix` is a
blank-separated list, so `integer(4), pure elemental function f(x)` is not conforming. findent
accepts it and indents the body; `forformat` reads it as a malformed declaration and leaves the body
at host depth. The conforming blank-separated form opens a frame in both.

```fortran
integer(4), pure elemental function myfunc2(x)
integer, intent(in) :: x    ! findent indents these two lines;
myfunc2 = x                 ! forformat leaves them at host depth
end function
```

**A kind parameter named `function`.** Fortran reserves no words, so a named constant may be called
`function` and used as a kind selector. findent reads the inner occurrence as the heading keyword,
rejects the statement, and leaves the body and every later sibling procedure unindented.
`forformat` resolves the heading from the statement's structure and indents both. The divergence is
confined to that one spelling: a constant named `subroutine` is a heading to both tools, because
there the shadowed word and the heading keyword differ.

```fortran
module m
   integer, parameter :: function = 4
contains
   integer(kind=function) function f()
      f = 1                ! findent leaves this and `end function f` at host depth
   end function f
end module m
```

Separately, a preprocessor directive that interrupts a *continued statement* is a known oracle
difference: `forformat` keeps the following line a continuation and gives it the continuation
indent, while findent returns it to statement indent. A directive between statements does not
produce this.

```fortran
call work(arg1, &
#include "actual_args.inc"
arg2)
```

## Full-mode differences

`--full` adds normalization and wrapping, which are policy choices rather than indentation
compatibility claims:

- Multiline `(/ ... /)` array constructors are rewritten as complete, valid `[ ... ]` constructors.
  The reference can change only the opening delimiter on a later continuation.
- Comment bodies are changed only by the narrow, provably assignment-shaped comment rule. The
  reference also respaces some nested or non-Fortran comment expressions; `forformat` preserves
  them.
- A kind suffix follows its governing declaration, including exponent literals, and does so
  consistently on continuation lines where the reference can miss the declaration. Numeric kinds
  such as `_8` and undeclared names are inert.
- Conditional `!$` sentinels keep their authored boundary spacing while the Fortran-like body is
  normalized, including declaration-driven identifier casing.
- `--reduce-whitespace` (equivalent to findent-compatible `--ws_remred`) leaves the bytes of a valid literal intact. A legacy heuristic can treat the
  quote after `error stop` as code and collapse spaces inside that literal.

## Whitespace boundary

The default contract replaces leading indentation and trims trailing spaces and tabs, matching
findent's free-form emission. `--reduce-whitespace` is the explicit opt-in for broader redundant
whitespace reduction; statements bearing Hollerith constants bypass it, and malformed or ambiguous
continued string expressions are reduced conservatively.

`--reduce-whitespace` also interacts with the two column-alignment options, `--align-declarations`
(on by default) and `--align-comments` (off by default). Each owns the one gap it aligns — the space
before a declaration's `::`, and before a trailing comment. When the option is enabled,
`--reduce-whitespace` leaves that gap alone instead of collapsing it before the alignment pass sees
the authored spacing. findent has no equivalent options, so this precedence has no oracle to diverge
from.

## Outside the supported contract

Some inputs are not valid Fortran in isolation, or need preprocessing semantics the formatter does
not provide:

- FYPP template bodies whose template syntax is not valid Fortran on its own.
- Preprocessor configurations whose branches open and close Fortran constructs asymmetrically.
- Inputs using operators or syntax from another language.
- COCO (`??`) and FYPP (`#:`) directives beyond safe grouping and continuation. CPP is the supported
  preprocessor feature; fuller COCO/FYPP support is deferred.

These are not silently promoted to supported behaviour. A family becomes supported only with a
regression case and an explicit compatibility decision.
