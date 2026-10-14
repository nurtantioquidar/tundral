#pragma once

#include <stdlib.h>

// WHY: a single allocator lets tests count allocations.
void *xmalloc(size_t n);
int clamp(int v, int lo, int hi);

static inline int util_max(int a, int b) {
    return a > b ? a : b;
}
