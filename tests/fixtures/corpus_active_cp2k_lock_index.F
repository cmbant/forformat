subroutine p(arow, nthread, nblkrows_total, locks)
  integer :: arow, nthread, nblkrows_total
  integer :: locks(*)
!$ call omp_unset_lock(locks((arow - 1) * &
!$                          nthread*10/nblkrows_total + 1))
end subroutine p
