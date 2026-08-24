subroutine p(rho, e_0, e_rho, e_rho_rho, e_rho_rho_rho, npoints, order, scale, eps_rho)
  integer :: order, npoints, k
  real :: rho(*), e_0(*), e_rho(*), e_rho_rho(*), e_rho_rho_rho(*), scale, eps_rho
  real :: ed
  integer :: abs_order
!$OMP PARALLEL DO PRIVATE (k, ed) DEFAULT(NONE)&
!$OMP SHARED(npoints, rho, eps_rho, abs_order, scale, e_0, e_rho, &
!$OMP e_rho_rho, e_rho_rho_rho, order)
  do k = 1, npoints
    if (rho(k) > eps_rho .and. order >= 0) then
    end if
  end do
end subroutine p
