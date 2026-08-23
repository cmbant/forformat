module m
contains
subroutine p
   REAL,DIMENSION(min0(1,its):max0(n_nba_mij,min(ite, ide-1)),min0(jms,kts) &
   :max0(jme,kte-1),min0(kms,jts):max0(kme,min(jte, jde-1)),min0(ims,PARAM_FIRST_SCALAR) &
   :max0(ime,n_moist)) :: Tmpv500
   REAL  A(3), B(4)
end subroutine p
end module m
