program benchmark_preprocessor
#if OUTER
if (x) then
#if INNER
do i = 1, 100
continue
end do
#else
select case (i)
case (1)
continue
end select
#endif
end if
#else
do i = 1, 100
#if INNER
continue
#else
if (y) then
continue
end if
#endif
end do
#endif
end program benchmark_preprocessor
