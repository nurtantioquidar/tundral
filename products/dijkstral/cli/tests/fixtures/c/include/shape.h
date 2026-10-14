#ifndef SHAPE_H
#define SHAPE_H

#include <stddef.h>
#include "util.h"

#define MAX_SHAPES 16
#define SQUARE(x) ((x) * (x))

/* NOTE: every shape is allocated by shape_new and freed by shape_free. */
typedef enum ShapeKind {
    SHAPE_CIRCLE,
    SHAPE_RECT = 4,
    SHAPE_POLY
} ShapeKind;

typedef struct Point {
    double x;
    double y;
} Point;

union Value {
    int i;
    double d;
};

struct Shape {
    ShapeKind kind;
    Point origin;
    union Value extra;
    struct Shape *next;
    double (*area_fn)(const struct Shape *);
};

typedef struct Shape Shape;
typedef double (*AreaFn)(const Shape *);

Shape *shape_new(ShapeKind kind, Point origin);
void shape_free(Shape *s);
double shape_area(const Shape *s);
extern int shape_count;

#endif
