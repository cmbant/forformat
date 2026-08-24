module m
contains
subroutine p(iatom, jatom, natom, nlock, hash)
  integer :: iatom, jatom, natom, nlock, hash
  if (iatom > 0) then
    do j = 1, natom
      if (j > 0) then
        do k = 1, nlock
          if (k > 0) then
!$           hash = mod((iatom - &
!$                       1) * natom + jatom, &
!$                       nlock) + 1
          endif
        enddo
      endif
    enddo
  endif
end subroutine p
end module m
