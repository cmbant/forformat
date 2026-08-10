module matrix_mod
implicit none
type :: base_type
integer :: value
contains
procedure :: binding
end type base_type
type, extends(base_type) :: child_type
integer :: extra
end type child_type
abstract interface
subroutine callback(x)
integer :: x
end subroutine callback
end interface
contains
subroutine binding(self)
class(base_type) :: self
continue
end subroutine binding
end module matrix_mod

submodule (matrix_mod) matrix_impl
contains
module procedure binding
continue
end procedure binding
end submodule matrix_impl

program matrix
use matrix_mod
implicit none
integer :: i, j, n
real :: a(10), b(10)
type(child_type) :: value
associate (x => value%value)
x = 1
end associate
block
integer :: local
local = 0
end block
do concurrent (i = 1:n) local(j)
a(i) = i
end do
where (a /= 0)
b = 1 / a
elsewhere
b = 0
end where
forall (i = 1:n)
b(i) = i
end forall
select type (value)
type is (child_type)
continue
class default
continue
end select
select rank (a)
rank (0)
continue
rank default
continue
end select
if (i > 0) then
continue
else if (i < 0) then
continue
else
continue
end if
outer: do 100 i = 1, n
inner: do 100 j = 1, n
continue
100 continue
critical
continue
end critical
change team (team)
continue
end team
enum, bind(c)
enumerator :: red = 1
end enum
include 'matrix.inc'
end program matrix

structure /legacy/
union
map
integer :: field
end map
end union
end structure
