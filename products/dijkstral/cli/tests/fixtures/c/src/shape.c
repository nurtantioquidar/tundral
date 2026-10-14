#include <math.h>
#include <string.h>
#include "shape.h"
#include "util.h"

int shape_count = 0;

static double circle_area(const Shape *s) {
    return M_PI * SQUARE(s->extra.d);
}

static double rect_area(const struct Shape *s) {
    return s->extra.d * s->origin.x;
}

static AreaFn pick_area(ShapeKind kind) {
    switch (kind) {
    case SHAPE_CIRCLE:
        return circle_area;
    default:
        return &rect_area;
    }
}

Shape *shape_new(ShapeKind kind, Point origin) {
    Shape *s = xmalloc(sizeof(Shape));
    memset(s, 0, sizeof *s);
    s->kind = kind;
    s->origin = origin;
    s->area_fn = pick_area(kind);
    shape_count++;
    return s;
}

void shape_free(Shape *s) {
    shape_count = clamp(shape_count - 1, 0, MAX_SHAPES);
    free(s);
}

// HACK: dispatch through the stored pointer.
double shape_area(const Shape *s) {
    return s->area_fn(s);
}

struct Registry {
    struct Shape *items[MAX_SHAPES];
    size_t len;
};

static struct Registry registry;

void registry_add(struct Registry *r, Shape *s) {
    r->items[r->len++] = s;
}
