! Free-standing procedures outside any module.
subroutine solve(n, x)
  integer, intent(in) :: n
  real, intent(inout) :: x(n)
  integer :: i
  do i = 1, n
    x(i) = step(x(i))
  end do
#ifdef DEBUG
  call trace(n)
#endif
  call missing_routine(n)
end subroutine solve

real function step(v)
  real, intent(in) :: v
  step = v * 0.5
end function step

subroutine trace(n)
  integer, intent(in) :: n
  print *, "n =", n
end subroutine

module legacy_types
  type :: Options
    integer :: max_iter = 100
  end type
  type :: Label
    integer :: id
  end type
end module legacy_types
