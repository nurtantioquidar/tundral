//! JSON manifests, `Cargo.toml`/`pyproject.toml`, `go.mod` and shell scripts (§12, §14).

use serde::{Deserialize, Serialize};

use super::Ctx;
use crate::lang::Lang;

/// Facts the cross-file pass (`crate::resolve::data`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DataFacts {}

pub fn extract(cx: &mut Ctx) {
    match cx.lang {
        // TOML package files and non-manifest JSON produce no file node (§14.1, §14.2).
        Lang::Json | Lang::Toml => {}
        _ => cx.add_file_node(vec![]),
    }
}
