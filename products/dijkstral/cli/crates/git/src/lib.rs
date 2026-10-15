//! Git integration (`docs/graphify-git.md`): the `post-commit` and `post-checkout` hooks that
//! keep `dijkstral-out/` current (`hooks.rs`, run by `rebuild.rs`), the `graph.json` merge
//! driver (`merge.rs`) and its git config and `.gitattributes` wiring (`attributes.rs`), all
//! installed, removed and reported by `install.rs` in the repository `repo.rs` finds.

pub mod attributes;
pub mod hooks;
pub mod install;
pub mod merge;
pub mod rebuild;
pub mod repo;

/// The hooks `hook install` manages, in report order.
pub const HOOKS: [&str; 2] = ["post-commit", "post-checkout"];

/// Git's name for the merge driver (`merge.<name>.*`, `merge=<name>`).
pub const DRIVER: &str = "dijkstral";

/// The graph the merge driver is registered for, relative to the work tree's top.
pub const GRAPH_PATH: &str = "dijkstral-out/graph.json";
