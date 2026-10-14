#include <cassert>

#include "geo/shape.hpp"

using geo::Circle;

void test_circle() {
    Circle c(1.0);
    assert(c.area() > 3.0);
}

int main() {
    test_circle();
    return 0;
}
