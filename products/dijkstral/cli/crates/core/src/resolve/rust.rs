//! Cross-file pass for Rust (§11). See `docs/graphify-format.md`.

use super::{Index, Resolution, View};

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let _ = (files, index);
    Resolution::default()
}
