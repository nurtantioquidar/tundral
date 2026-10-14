#include <iostream>
#include <memory>

#include "../include/geo/shape.hpp"
#include "kernels.h"

using namespace geo;

namespace {
int helper(int x) { return x + 1; }
}  // namespace

static void print_area(const Shape &s) {
    std::cout << s.name() << " " << s.area() << std::endl;
}

int main() {
    Registry<Shape> reg;
    reg.add(std::make_unique<Circle>(2.0));
    auto sq = Rect::square(3.0);
    print_area(sq);
    std::cout << total_area(reg) << helper(1) << std::endl;
    launch_scale(nullptr, 0, 2.0f);
    auto cb = [](int v) { return helper(v); };
    return cb(0) > 0 ? 0 : 1;
}
