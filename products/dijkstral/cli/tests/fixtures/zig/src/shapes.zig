const std = @import("std");
const hue = @import("hue.zig");

pub const Shape = union(enum) {
    rect: Rect,
    circle: f64,
    pub fn kind(self: Shape) []const u8 {
        _ = self;
        return "shape";
    }
};

pub const Rect = struct {
    w: f64,
    h: f64,
};

const Raw = union {
    int: i64,
    float: f64,
};

pub fn area(r: Rect) f64 {
    return r.w * r.h;
}

pub fn paint(c: hue.Hue) u8 {
    return tint(c);
}

fn add_(x: f64) f64 {
    return x;
}

fn add(x: f64) f64 {
    return add_(x);
}
