module m
 type :: t_Name
   integer :: n
 end type t_NAME
 type(t_name) :: v
contains
 subroutine SymPairs_t_init(self)
   type(t_name), intent(inout) :: self
 end subroutine sympairs_t_init
end module M
