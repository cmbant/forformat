program benchmark
do i = 1, 100
if (mod(i, 2) == 0) then
x = "quoted ! text; still one statement" &
 & + i
else
x = i
end if
end do
end program benchmark
