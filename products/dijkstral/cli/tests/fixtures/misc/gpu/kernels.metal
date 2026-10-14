#include <metal_stdlib>
using namespace metal;

struct Particle {
    float x;
    float y;
};

float speed(Particle p) {
    return p.x + p.y;
}

kernel void step(device Particle* ps [[buffer(0)]], uint id [[thread_position_in_grid]]) {
    ps[id].x += speed(ps[id]);
}
