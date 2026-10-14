pub const Hue = enum(u8) {
    red,
    green,
    blue,
    _,

    pub fn isWarm(self: Hue) bool {
        return self == .red;
    }
};

pub fn tint(c: Hue) u8 {
    return @intFromEnum(c);
}
