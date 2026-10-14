const std = @import("std");
const util = @import("../util.zig");

test "add works" {
    const Fixture = struct {
        fn make() i32 {
            return add(2, 3);
        }
    };
    _ = Fixture.make();
    run(struct {
        fn callback() void {
            helper();
        }
    });
}

fn run(x: anytype) void {
    _ = x;
}

comptime {
    const Probe = enum { on, off };
    _ = Probe;
}
