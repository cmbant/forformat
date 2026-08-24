module m
contains
subroutine p(x)
  integer :: x
  if (x > 0) then
    data eight / 8.0D0/, four / 4.0D0/, half / 0.5D0/, &
         throv8 / 0.375D0/, &
         pi2 / 6.3661977236758134308D-1/, &
         p17 / 1.716D-1 / twopi / 6.2831853071795864769D+0/, &
         zero / 0.0D0/, twopi1 / 6.28125D0/, &
         twopi2 / 1.9353071795864769253D-03 / two56 / 256.0D+0/ &
         ,rtpi2 / 7.9788456080286535588D-1/
  endif
end subroutine p
end module m
