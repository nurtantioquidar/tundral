//! Parity with Graphify's clustered golden for `tests/fixtures/rust`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Cargo.toml `[package]` node (§11.6): emitted by the TOML extractor, owned elsewhere.
    "pkg_shapes",
    // Q14: generic type parameters (`T` of `Wrapper<T>`/`Pair<T>`, `F` of `apply<F>`) are not
    // type references, so neither their stubs nor the edges to them are emitted.
    "t",
    "f",
    "src_model_pair -references-> t",
    "src_model_pair_t_first -references-> t",
    "src_model_wrapper -references-> t",
    "src_model_wrapper_t_new -references-> t",
    "src_util_mod_apply -references-> f",
    // Q6: the clustered build's file-stem remap is not reproduced, so `use std::fmt` stays on the
    // external `fmt` instead of the unrelated `src/util/fmt.rs`, and `use super::model::*` stays on
    // the external `model`.
    "fmt",
    "model",
    "src_model -imports_from-> fmt",
    "src_model -imports_from-> src_util_fmt",
    "src_util_mod -imports_from-> model",
    "src_util_mod -imports_from-> src_model",
];

#[test]
fn rust_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (NMI 0.88, graphify-format.md §17.7).
    common::assert_fixture("rust", ALLOWED, 0.25, false, false);
}
