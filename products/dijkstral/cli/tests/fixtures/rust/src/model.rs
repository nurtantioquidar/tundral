//! Shape model.
use serde::Serialize;
use std::fmt;

/// Anything with an area.
pub trait Shape: fmt::Debug {
    fn area(&self) -> f64;

    /// Default method on the trait.
    fn describe(&self) -> String {
        format!("area={}", self.area())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Circle {
    pub radius: f64,
}

#[derive(Debug)]
pub struct Meters(pub f64);

/// A named shape; `Sized` is a std trait and yields no edge.
pub trait Named: Shape + Sized {
    fn name(&self) -> String;
}

#[cfg(unix)]
pub struct Platform;
#[cfg(not(unix))]
pub struct Platform;

#[derive(Debug, Default)]
pub struct Unit;

#[derive(Debug)]
pub enum Kind {
    Round(Circle),
    Square { side: f64 },
    Empty,
}

impl Circle {
    pub const UNIT: f64 = 1.0;

    pub fn new(radius: f64) -> Self {
        Circle { radius }
    }

    pub fn diameter(&self) -> Meters {
        Meters(self.radius * 2.0)
    }
}

impl Circle {
    // WHY: a second inherent impl block for the same type.
    pub fn scale(&mut self, by: f64) {
        self.radius *= by;
    }
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * crate::square!(self.radius)
    }
}

impl fmt::Display for Circle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "circle({})", self.radius)
    }
}

pub struct Wrapper<T: Shape> {
    pub inner: T,
}

impl<T: Shape + Clone> Wrapper<T> {
    pub fn new(inner: T) -> Self {
        Wrapper { inner }
    }

    pub fn area(&self) -> f64 {
        self.inner.area()
    }
}

pub struct Pair<T>(pub T, pub T);

impl<T> Pair<T> {
    pub fn first(&self) -> &T {
        &self.0
    }
}

pub fn round_kind() -> Kind {
    Kind::Round(Circle::new(Circle::UNIT))
}

pub fn classify(kind: &Kind) -> &'static str {
    fn pick(round: bool) -> &'static str {
        if round { "round" } else { "other" }
    }
    match kind {
        Kind::Round(_) => pick(true),
        _ => pick(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Circle {
        Circle::new(2.0)
    }

    #[test]
    fn area_is_positive() {
        assert!(fixture().area() > 0.0);
    }
}
