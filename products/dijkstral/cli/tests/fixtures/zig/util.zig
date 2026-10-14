pub fn add(a: i32, b: i32) i32 {
    return a + b;
}

pub fn scale(x: i32) i32 {
    return add(x, x);
}

pub fn List(comptime T: type) type {
    return struct {
        items: Buffer(T),
        fn len() usize {
            return 0;
        }
    };
}

fn Buffer(comptime T: type) type {
    return []T;
}

const Sentinel = struct {
    marker: u8,
};

fn sentinel() u8 {
    return 0;
}

pub fn check() u8 {
    return sentinel();
}
