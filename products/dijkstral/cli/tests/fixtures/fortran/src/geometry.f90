module Geometry
  use constants, only: PI
  use, intrinsic :: iso_fortran_env, only: real64
  implicit none

  type :: Point
    real :: x
    real :: y
  contains
    procedure :: norm => point_norm
    procedure, pass :: shift, scale_point
    generic :: move => shift
  end type Point

  type, extends(Point) :: Point3
    real :: z
  end type

  interface Distance
    module procedure distance_points, distance_coords
  end interface Distance

  interface operator(+)
    module procedure add_points
  end interface

contains

  function point_norm(p) result(r)
    class(Point), intent(in) :: p
    real :: r
    r = sqrt(p%x**2 + p%y**2)
  end function point_norm

  subroutine shift(p, dx, dy)
    class(Point), intent(inout) :: p
    real, intent(in) :: dx, dy
    p%x = p%x + dx
    p%y = p%y + dy
  end subroutine shift

  function distance_points(a, b) result(d)
    type(Point), intent(in) :: a, b
    real :: d
    d = distance_coords(a%x, a%y, b%x, b%y)
  end function

  function distance_coords(x1, y1, x2, y2) result(d)
    real, intent(in) :: x1, y1, x2, y2
    real :: d
    d = sqrt((x2 - x1)**2 + (y2 - y1)**2)
  end function

  function add_points(a, b) result(c)
    type(Point), intent(in) :: a, b
    type(Point) :: c
    c = Point(a%x + b%x, a%y + b%y)
  end function

  function origin()
    type(Point) :: origin
    origin = Point(0.0, 0.0)
  end function

  ! WHY: the circumference helper lives inside its caller.
  subroutine report_circle(radius)
    real, intent(in) :: radius
    print *, circumference()
    call log_value(radius)
  contains
    real function circumference()
      circumference = 2.0 * PI * radius
      call report_circle(0.0)
    end function
  end subroutine report_circle

end module Geometry
