program paren
   x = y + &
      fun(a, &
      abcd ,&
      z)
   x = y +  &
              z + &
      a; call sub (a, &
!comment
      b, &
      c)
   x = y +  a; call sub (a, &
      b, &
      c)

   print *,'abc'; call sub(a,&
      b,&
      c)
#if 0
   write(10,'20) x; call sub(a,&
      b,&
      c)
   '
   write(10           '   20) x; call sub(a,&
      b,&
      c)
   write(10"20) x; call sub(a,&
      b,&
      c)
   "
#endif
contains
   subroutine sub(a,b,c)
   end
   function fun(a,b,c)
      fun=a
   end
end program
subroutine gnikit
 x = p + fun (a,&
  b,&
  c)
100 format(4ha(cd, &
       4hx(bc,&
       i5)
10 call sub1(a,&
  bcd,&
   fun(3.0,&
   4.0,&
      5.0),&
  [6, &
  7, &
  8] &
   )
 call sub1(a,&
  bcd,&
   fun(3.0,&
! comment
   4.0,&
   ! comment
      5.0),&
  [6, &
  7, &
  8] &
   )
contains
 subroutine sub1(a,b,c,d)
    integer d(3)
   end
   function fun(a,b,c)
      fun=a
   end
  end
