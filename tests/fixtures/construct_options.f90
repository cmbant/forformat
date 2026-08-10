program options
associate (a => b)
continue
end associate
block
continue
end block
do i = 1, 2
continue
end do
if (x) then
continue
end if
enum, bind(c)
enumerator :: red = 1
end enum
forall (i = 1:2)
continue
end forall
interface
subroutine member(a)
continue
end subroutine member
end interface
contains
subroutine work
continue
end subroutine work
end program options

module options_module
type :: t
integer :: value
end type t
end module options_module

program select_options
select case (i)
case (1)
continue
end select
where (x > 0)
continue
end where
critical
continue
end critical
end program select_options
