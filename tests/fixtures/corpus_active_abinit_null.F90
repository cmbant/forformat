module m
  use iso_c_binding, only: c_double, c_double_complex
  type, public :: xgBlock_t
    integer, private :: space
    real(kind=c_double), ABI_CONTIGUOUS pointer, private :: vecR(:, :) => null()
    complex(kind=c_double_complex), ABI_CONTIGUOUS pointer, private :: vecC(:, :) => null()
  end type xgBlock_t
  type, public :: xg_t
    integer, private :: space
    real(kind=c_double), ABI_CONTIGUOUS pointer, private :: vecR(:, :) => null()
    complex(kind=c_double_complex), ABI_CONTIGUOUS pointer, private :: vecC(:, :) => null()
  end type xg_t
end module m
