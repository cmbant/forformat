# CosmoMC formatter comparison

Date: 2026-08-13  
Corpus: recursive clean-`HEAD` `cosmomc` checkout, including `camb` and `camb/forutils`
Sources: 102 free-form Fortran files (`cosmomc` 83f00ac4, `camb` c9e9bb0a, `camb/forutils` daf3e401)

## Method

The two outputs were generated from the same clean checkout:

1. `findent 4.3.7` with the profile in `tools/reference/findent_fortran.py`, followed by
   `tools/reference/standardize_fortran.py --all` over the recursive checkout. The Python
   run used `git ls-files --recurse-submodules` and therefore supplied all 102 Fortran files
   as one declaration-resolution project.
2. Rust `forformat --full` with the same indentation profile.

The list below intentionally omits matching behavior. It records the differences that remain,
classified by which formatter needs correction, with a neutral category for formatting policy
choices that are not correctness bugs, and closes with what has since been fixed.

## Python/reference is wrong

### Inherited type-bound members

Rust canonicalizes these members using the declaration graph:

```fortran
call F%write(...)  ->  call F%Write(...)
call F%close()     ->  call F%Close()
```

Python resolves the local `F` as `TTextFile`, but does not include the inherited generic
`Write` binding or follow the parent-type chain from `TTextFile` to `TFileStream%Close`.
Fortran is case-insensitive, so this is a Python canonicalization gap rather than a semantic
program difference.

`camb/fortran/SeparableBispectrum.f90` contains the same declaration-backed
`file_alpha%close()` → `file_alpha%Close()` case, alongside neutral layout differences below.

## Rust is wrong

### Implicit variables in `PowellConstrainedMinimize.f90`

Rust changes implicit local variables such as:

```fortran
DO I=1,N             ->  do i = 1, N
XOPT(I)=XPT(KOPT,I)  ->  XOPT(i) = XPT(KOPT, i)
```

Under Fortran's implicit rules, `I` through `N` are implicit integer variables local to each
procedure. Rust's declaration analysis does not model those implicit entities and can borrow
the spelling `i` from an unrelated project declaration. The safer behavior is to preserve the
local authored spelling until implicit typing is represented explicitly.

## Neither is wrong: formatting policy

These remaining differences affect presentation, not Fortran meaning:

- continuation reflow and the location of a continuation break, for example the long `J_l`
  assignment in `camb/fortran/SeparableBispectrum.f90`:

  ```fortran
  J_l = a2*ajl(...) - ((a2+1) &
      *ajlpr(...)) * fac
  ```

  The competing output moves the break to a different safe operator boundary.
- indentation of comments, such as the disabled block in
  `camb/fortran/SeparableBispectrum.f90`:

  ```fortran
  !           if (bispectrum_type == fnl_bispectrum_ix) then
  !              fish_contribs_sig = 0
  ```

  Only the comment indentation changes.
- spacing of operators inside commented-out code, such as
  `! CForLensing(i)%C=0` versus `! CForLensing(i)%C = 0`.
- a named argument whose value opens with `.not.`, in `source/SampleCollector.f90`:

  ```fortran
  append=.not. new_chains   ! Rust
  append= .not. new_chains  ! Python
  ```

  Python pads the left of `.not.` without padding the right of the `=` it follows. Rust keeps
  the argument compact on both sides, as it already did for `m=-3`.

There is no general correctness winner for these cases; choosing one requires an explicit
project layout or comment-preservation policy. In `SeparableBispectrum.f90`, the moved
continuation break and comment indentation belong to this category.

## Resolved: corpus fixed-point instability

The Rust output used to need a second pass over this corpus, which changed
`source/EstCovmat.f90` (continuation wrapping) and `source/SampleCollector.f90` (spacing around
`append=`). Both are fixed; `forformat --check` is now clean immediately after the first pass,
and the result is a fixed point for every profile tried, including `--align-paren`, `--indent=8`
and `--line-length=80`.

Four causes, all of the same shape — something the pipeline does *after* the wrapper has
measured the text pushed a line past the budget, and the next run rewrapped it:

- the wrapper measured the authored physical lines, not the ones the layout engine was about to
  emit, so normalization widening (` // `) or a deeper indent went unnoticed (`EstCovmat.f90`);
- OpenMP directives were measured at their authored indent (`camb/fortran/results.f90`, visible
  at `--indent=4` without the CosmoMC profile's `--indent_module=0`);
- step 17's `::` padding was not charged to the wrapper's budget (`source/szcounts.f90`, visible
  at `--indent=8`);
- a detached trailing comment was forced back to the statement indent after layout, which
  disagreed with the engine above a dedented `else if` (`camb/fortran/MathUtils.f90`, visible at
  `--line-length=80`).

Two latent defects surfaced during the same investigation and are also fixed:

- a statement the wrapper declined (`NoSafeBreak`) kept only its first physical line, deleting
  the continuations of a multi-line statement and leaving a dangling `&`;
- `/)` on a *continuation* line of a `FORMAT` statement was rewritten to `]` as if it closed an
  array constructor, because the `format` keyword sits on the previous line
  (`camb/forutils/Interpolation.f90` under any profile that wraps that statement).

## Indentation agrees with findent

Over the same 102 files, `forformat --indent-only` with the profile above is byte-identical to
`findent 4.3.7` for every file, and `findent` is a fixed point of the `--full` output — I2
(`indent_only(full(x)) == full(x)`) holds on the corpus, not just on the fixtures.
