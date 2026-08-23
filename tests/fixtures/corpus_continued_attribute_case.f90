module corpus_continued_attribute_case
   TYPE(T), POINTER, &
      DIMENSION(:) :: P
   REAL, DIMENSION(3), &
      PARAMETER :: R = (/1.0, 2.0, 3.0/)
contains
   SUBROUTINE p(a, b)
      REAL, OPTIONAL, DIMENSION(ims:ime, jms:jme), &
           INTENT(IN) :: a
      REAL, ALLOCATABLE, TARGET, &
           SAVE :: b(:)
   END SUBROUTINE p
end module corpus_continued_attribute_case
