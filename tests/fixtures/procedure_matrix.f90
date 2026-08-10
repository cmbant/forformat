subroutine p_sub1
continue
contains
subroutine mysub
continue
end
end subroutine p_sub1

real function myfunc()
continue
contains
subroutine asub(x)
continue
end subroutine asub
end function myfunc

program p_interface
interface inter
subroutine s(x)
real x
end
end interface inter
end program

program p
end
integer(4), pure elemental function myfunc2(x)
integer, intent(in) :: x
myfunc2 = x
end function
pure function pfunc(x) result(y)
real*8, intent(in) :: x
real*8 y
y=x
end
elemental subroutine mysub2(i)
integer, intent(inout) :: i
i = 2*i
continue
end subroutine mysub2
pure subroutine psub(x,y)
real, intent(inout) :: x
real, intent(in) :: y
x = x*y
continue
end
