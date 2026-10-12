//! Java (§13) extraction. See `docs/graphify-format.md`.

use serde::{Deserialize, Serialize};
use tree_sitter::Tree;

use super::Ctx;

/// Facts the cross-file pass (`crate::resolve::java`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JavaFacts {}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(vec![]);
    let _ = tree;
}
