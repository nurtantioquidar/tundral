#include "geo/shape.hpp"

#include <cmath>
#include <sstream>

namespace geo {

Shape::Shape(Kind kind) : kind_(kind) {}

std::string Shape::name() const {
    std::ostringstream out;
    out << (kind_ == Kind::Circle ? "circle" : "rect");
    return out.str();
}

Circle::Circle(double r) : Shape(Kind::Circle), r_(r) {}

// WHY: M_PI is not standard C++, so spell pi out.
double Circle::area() const {
    return 3.141592653589793 * r_ * r_;
}

Rect::Rect(double w, double h) : Shape(Kind::Rect), w_(w), h_(h) {}

double Rect::area() const { return w_ * h_; }

Rect Rect::square(double side) {
    return Rect(side, side);
}

double total_area(const Registry<Shape> &reg) {
    double sum = 0;
    if (Shape *s = reg.first()) {
        sum += s->area();
    }
    return sum + detail::round_up(sum);
}

}  // namespace geo

int geo::detail::round_up(double v) {
    return static_cast<int>(std::ceil(v));
}

double geo::Point::dist(const Point &o) const {
    return std::sqrt((x - o.x) * (x - o.x) + (y - o.y) * (y - o.y));
}
