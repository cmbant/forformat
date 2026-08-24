subroutine p(on_root, eval_ahc, eval_shc, adapt, unit, scan)
  logical :: on_root, eval_ahc, eval_shc, adapt, scan
  character(*) :: unit
  if (on_root) then
    if (eval_ahc .and. adapt .ne. 1) then
      if (unit == 'ang2') then
        write(*,*) 'ang'
      elseif (unit == 'bohr2') then
        write(*,*) 'bohr'
      endif
    elseif (eval_shc) then
      if (adapt .ne. 1) then
        if (.not. scan) then
          if (unit == 'ang2') then
            write(*,*) 'ang'
          elseif (unit == 'bohr2') then
            write(*,*) 'bohr'
          endif
        endif
      endif
    endif
  endif
end subroutine p
