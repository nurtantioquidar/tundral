#include <assert.h>
#include "../include/shape.h"

static void test_area(void) {
    Point p = {1.0, 2.0};
    Shape *s = shape_new(SHAPE_RECT, p);
    assert(shape_area(s) >= 0.0);
    shape_free(s);
}

int main(void) {
    test_area();
    return 0;
}
