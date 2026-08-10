program p
outer: do 100 i=1,2
#if OUTER
inner: do 100 j=1,2
#else
inner: do 100 k=1,2
#endif
continue
100 continue
x = 1
end program
