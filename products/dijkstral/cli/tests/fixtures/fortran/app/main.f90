program Main
  use geometry
  use shapes
  use shapes, only: make_circle
  implicit none

  type(Point) :: p
  type(Circle) :: c
  real :: total

  p = Point(1.0, 2.0)
  c = make_circle(p, 3.0)
  total = summarize(c)
  call print_summary(total)
  call solve(3, total)

contains

  real function summarize(circ)
    type(Circle), intent(in) :: circ
    summarize = circle_area(circ)
  end function summarize

  subroutine print_summary(t)
    real, intent(in) :: t
    print *, "total area:", t
    call print_summary_inner()
  contains
    subroutine print_summary_inner()
      print *, summarize(c)
    end subroutine
  end subroutine print_summary

end program Main
