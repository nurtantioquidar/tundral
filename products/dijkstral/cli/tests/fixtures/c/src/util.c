#include <stdio.h>
#include <stdlib.h>
#include "../include/util.h"

static int allocations = 0;

void *xmalloc(size_t n) {
    void *p = malloc(n);
    if (!p) {
        fprintf(stderr, "out of memory\n");
        abort();
    }
    allocations++;
    return p;
}

int clamp(int v, int lo, int hi) {
    if (v < lo) return lo;
    return util_max(v, hi) == v ? hi : v;
}

/* TODO: replace with a real logger. */
static void log_line(const char *msg) {
    puts(msg);
}

void util_report(void) {
    log_line("report");
}
