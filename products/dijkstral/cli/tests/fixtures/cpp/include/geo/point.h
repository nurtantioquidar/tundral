#ifndef GEO_POINT_H
#define GEO_POINT_H

#include <cmath>

#define GEO_EPS 1e-9
#define GEO_ABS(x) ((x) < 0 ? -(x) : (x))

namespace geo {

struct Point {
    double x = 0;
    double y = 0;
    double dist(const Point &o) const;
};

struct Named {
    const char *label;
};

using Points = std::vector<Point>;
typedef Point Vec2;

union Bits {
    float f;
    unsigned u;
};

inline bool near(double a, double b) { return GEO_ABS(a - b) < GEO_EPS; }

}  // namespace geo

#endif
