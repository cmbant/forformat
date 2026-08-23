subroutine p
real, dimension(min0(jms, its):max0(jme, min(ite, ide - 1)), min0(jms, jts):max0(jme, min(jte, jde - 1)), &
               min0(ims, PARAM_FIRST_SCALAR):max0(ime, n_moist)) :: Tmpv402
real, dimension(its:min(ite, ide - 1), jts:min(jte, jde - 1), PARAM_FIRST_SCALAR:n_moist) :: Tmpv403
real, dimension(min0(1, its):max0(n_nba_mij, min(ite, ide - 1)), min0(jms, kts)  :max0(jme, kte - 1), &
               min0(kms, jts):max0(kme, min(jte, jde - 1)), &
               min0(ims, PARAM_FIRST_SCALAR)  :max0(ime, n_moist)) :: Tmpv500
real, dimension(min0(1, its):max0(n_nba_mij, min(ite, ide - 1)), min0(jms, kts)  :max0(jme, kte - 1), &
               min0(kms, jts):max0(kme, min(jte, jde - 1)), &
               min0(ims, PARAM_FIRST_SCALAR)  :max0(ime, n_moist)) :: Tmpv501
real, dimension(n_nba_mij, jms:jme, kms:kte, ims:ime) :: Tmpv502
end subroutine p
