subroutine p(i, j, ktau)
  if (wrf_at_debug_level(lsmruc_dbg_lvl)) then
    if (ktau == 1 .and. &
        (i == 358 .and. j == 260)) print * &
      , 'before soilvegin - ' // &
      'z0,znt(195,254)', z0(i, j), znt(i, j)
  endif
end subroutine p
