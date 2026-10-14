module shapes
  use geometry
  use utils
  implicit none

  type :: Circle
    type(Point) :: center
    real :: radius
  end type Circle

  abstract interface
    function area_fn(c) result(a)
      import :: Circle
      type(Circle), intent(in) :: c
      real :: a
    end function
  end interface

  interface
    subroutine external_draw(c, canvas)
      import :: Circle
      type(Circle), intent(in) :: c
      type(Canvas), intent(inout) :: canvas
    end subroutine external_draw
  end interface

contains

  function circle_area(c) result(a)
    type(Circle), intent(in) :: c
    real :: a
    a = PI * c%radius**2
  end function circle_area

  function make_circle(center, radius) result(c)
    type(Point), intent(in) :: center
    real, intent(in) :: radius
    type(Circle) :: c
    c = Circle(center, radius)
    call log_value(radius)
  end function make_circle

  subroutine describe(c, tag, opts)
    type(Circle), intent(in) :: c
    type(Label), intent(in) :: tag
    type(Options), intent(in) :: opts
    print *, circle_area(c)
    call external_draw(c, opts)
  end subroutine describe

end module shapes

submodule (shapes) shapes_render
contains
  module procedure render
    call describe_all()
  end procedure
end submodule
