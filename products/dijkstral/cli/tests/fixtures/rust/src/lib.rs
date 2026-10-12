//! Crate root: shapes and their rendering.

pub mod model;
pub mod util;

pub use crate::model::{Shape, Circle as Round};
pub use util::fmt::render;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// Maximum number of shapes in a scene.
pub const MAX_SHAPES: usize = 64;
pub static GREETING: &str = "hello";
pub type Registry = HashMap<String, Box<dyn Shape>>;

#[macro_export]
macro_rules! square {
    ($x:expr) => {
        $x * $x
    };
}

pub fn build(names: &[&str]) -> Registry {
    let mut reg = Registry::new();
    let seen: HashSet<&str> = names.iter().copied().collect();
    for n in seen {
        reg.insert(n.to_string(), Box::new(model::Circle::new(1.0)));
    }
    reg
}

pub fn label(name: &str) -> Cow<'_, str> {
    // NOTE: borrowed when already lowercase.
    if name.chars().all(|c| c.is_lowercase()) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.to_lowercase())
    }
}

pub fn total_area(shapes: &[Box<dyn Shape>]) -> f64 {
    shapes.iter().map(|s| s.area()).sum()
}
