use crate::model::{Circle, Shape};
use memchr::memchr;

pub fn render(c: &Circle) -> String {
    let text = format!("{}", c.area());
    match memchr(b'.', text.as_bytes()) {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

pub fn helper() -> u32 {
    7
}

pub fn configure(_s: &Settings) {}
