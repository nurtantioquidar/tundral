pub mod fmt;

use self::fmt::render as render_shape;
use super::model::*;

pub mod inner {
    pub fn helper() -> u32 {
        41
    }

    pub mod deeper {
        pub fn helper() -> u32 {
            super::helper() + 1
        }
    }
}

/// Duplicate name: `render` also exists in util::fmt.
pub fn render(c: &Circle) -> String {
    render_shape(c)
}

pub fn apply<F: Fn(u32) -> u32>(f: F) -> u32 {
    f(inner::helper())
}

pub fn twice(x: u32) -> u32 {
    x * 2
}

pub fn reconfigure(_s: &Settings) {}

pub fn widen(x: f64) -> f64 {
    Meters(x).0
}

pub fn run_all() -> u32 {
    apply(twice) + inner::deeper::helper()
}
