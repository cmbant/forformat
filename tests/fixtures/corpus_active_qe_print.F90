    subroutine stringdataarray(string, array, num, iostat, sep, csv)
      character(len=*), intent(in) :: string
      character(len=*), dimension(:), intent(in) :: array
      integer, intent(in) :: num, iostat
      character, intent(in), optional :: sep
      logical, intent(in), optional :: csv
      character(len=len(array)) :: temp(size(array))
      integer :: n, i
      call rts(string, temp, separator=sep, csv=csv, num=n, iostat=i)
      if (any(temp/=array)) &
        print*, "Different array"
      if (i/=iostat) &
        print*, "Wrong iostat"
      if (n/=num) &
        print*, "Wrong num"
      if (((i<=0).and.(countrts(string," ",sep,csv)/=num)).or. &
          ((i>1).and.(countrts(string," ",sep,csv)/=0)))      &
        print*, "Countrts wrong", countrts(string," ",sep,csv)
    end subroutine stringdataarray
