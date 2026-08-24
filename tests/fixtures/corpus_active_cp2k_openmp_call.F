subroutine p(hash, locks)
  integer :: hash
  integer :: locks(*)
!$ call &
!$   omp_set_lock(locks(hash))
!$ call &
!$   omp_unset_lock(locks(hash))
end subroutine p
