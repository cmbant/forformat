program matrix
integer :: i, j, n
if (i .eq. 1) then
x = 1
else if (i .eq. 2) then
x = 2
else
x = 3
end if
select case (i)
case (1)
x = 4
case default
x = 5
end select
do i = 1, n
do j = 1, n
continue
end do
end do
associate (z => x)
z = z + 1
end associate
block
integer :: local
local = 0
end block
where (x > 0)
y = x
elsewhere
y = 0
end where
forall (i = 1:n)
y(i) = i
end forall
critical
continue
end critical
change team (team)
continue
end team
enum, bind(c)
enumerator :: red = 1
end enum
type :: t
integer :: value
end type t
interface
subroutine member(a)
integer :: a
end subroutine member
end interface
contains
subroutine work(a)
integer :: a
if (a > 0) then
continue
end if
end subroutine work
function value(a)
integer :: a
value = a
end function value
end program matrix
