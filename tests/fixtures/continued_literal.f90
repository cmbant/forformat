program p
implicit none
character(len=20) :: s
character :: t(3)
logical :: m(3)
integer :: i
s = 'abc def!ghi'
t = 'a'
m = .false.
if (s == 'abc &
&def!ghi') then
s = "x &
&y!z"
end if
! A continued character context steps over comment and blank lines: the
! separator is not part of the statement, so it neither closes the literal
! nor lets the `!` in `def!ghi` open a comment.
if (s == 'abc &
! explanatory comment
&def!ghi') then
s = 'p &

&q!r'
end if
if (s == 'abc &
! don't stop here
&def!ghi') then
s = "u &
! nor " here
&v!w"
end if
where (t == 'a')
m = .true.
end where
forall (i = 1:3, t(i) == 'a')
t(i) = 'b'
end forall
if (s == 'don''t') m(1) = .false.
print *, s, t, m
end program p
