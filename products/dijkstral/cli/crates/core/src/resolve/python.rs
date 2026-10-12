//! Cross-file pass for Python (§9). See `docs/graphify-format.md`.

use super::{Index, Resolution, View};

pub fn resolve(files: &[View], index: &Index) -> Resolution {
    let _ = (files, index);
    Resolution::default()
}
