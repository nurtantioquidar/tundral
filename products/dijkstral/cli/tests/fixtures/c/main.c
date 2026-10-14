/*
 * Demo driver for the shape library.
 */
#include <stdio.h>
#include "include/shape.h"
#include "missing.h"

typedef int (*Callback)(int);

static int twice(int v) { return v * 2; }

static int apply(Callback cb, int v) {
    return cb(v);
}

int main(int argc, char **argv) {
    Point origin = {0.0, 0.0};
    Shape *s = shape_new(SHAPE_CIRCLE, origin);
    printf("%f\n", shape_area(s));
    printf("%d\n", apply(twice, argc));
    util_report();
    shape_free(s);
    return 0;
}
