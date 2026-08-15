module parent
interface
module subroutine binding()
end subroutine binding
end interface
end module parent

submodule (parent) child
contains
module procedure binding
integer :: value
value=1
end procedure binding
end submodule child

module equivalent
contains
subroutine binding
integer :: value
value=1
end subroutine binding
end module equivalent
