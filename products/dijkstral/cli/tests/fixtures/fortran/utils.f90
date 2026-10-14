module utils
  implicit none

  type :: Label
    character(len=32) :: text
  end type Label

contains

  subroutine log_value(v)
    real, intent(in) :: v
    print *, "value:", v
  end subroutine log_value

  recursive function factorial(n) result(f)
    integer, intent(in) :: n
    integer :: f
    if (n <= 1) then
      f = 1
    else
      f = n * factorial(n - 1)
    end if
  end function factorial

end module utils
