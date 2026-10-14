const std = @import("std");
const mem = @import("std").mem;
const util = @import("util.zig");
const shapes = @import("src/shapes.zig");
const Hue = @import("src/hue.zig").Hue;
const missing = @import("missing.zig");
const self_ref = @import("main.zig");

// NOTE: rationale comments are not extracted for Zig.
/// Doc comments neither.
pub fn main() !void {
    const total = add(1, 2);
    _ = scale(total);
    var tally = Tally.init();
    tally.bump();
    tally.bump();
    _ = shapes.area(.{ .w = 2, .h = 3 });
    std.debug.print("{}\n", .{total});
    report();
    _ = paint(Hue.red);
    try load();
}

fn report() void {
    report();
    helper();
}

fn helper() void {}

fn load() !void {
    return error.Missing;
}

pub const Tally = struct {
    n: u32,
    mode: enum { idle, busy },

    pub fn init() Tally {
        return .{ .n = 0, .mode = .idle };
    }

    pub fn bump(self: *Tally) void {
        self.n += 1;
        self.reset();
        helper();
    }

    fn reset(self: *Tally) void {
        self.n = 0;
    }

    const Snapshot = struct {
        value: u32,
        fn take() Snapshot {
            return .{ .value = 0 };
        }
    };
};

const LoadError = error{ Missing, Corrupt };

var phase: enum { start, stop } = .start;

const Handle = opaque {
    fn close() void {}
};
