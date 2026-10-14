#include "../include/geo/point.h"

extern "C" {
int legacy_add(int a, int b) { return a + b; }
}

struct Counter {
    int n = 0;
    void bump() { n = legacy_add(n, 1); }
    int get() const;
};

int Counter::get() const { return n; }

class Base {
public:
    virtual void run() {}
};

class Derived : public Base {
public:
    void run() override { counter.bump(); }
    Counter counter;
};

template <typename T>
T clamp_to(T v, T lo, T hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

int use_point() {
    geo::Point p;
    p.x = 1;
    Derived d;
    d.run();
    return clamp_to(legacy_add(1, 2), 0, 10) + static_cast<int>(p.dist(p));
}
