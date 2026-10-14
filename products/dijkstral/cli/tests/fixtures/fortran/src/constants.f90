! NOTE: physical constants shared by every module.
module constants
  implicit none
  real, parameter :: PI = 3.14159
  real, parameter :: TAU = 2.0 * PI
contains
  pure function deg2rad(deg) result(rad)
    real, intent(in) :: deg
    real :: rad
    rad = deg * PI / 180.0
  end function deg2rad
end module constants
