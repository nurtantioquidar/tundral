//! Parity with Graphify's clustered golden for `tests/fixtures/mixed`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // `app/main.py` belongs to the Python extractor; its symbols are allowed until it lands, so
    // this test measures the JSON, TOML and shell extractors only.
    "app_main_helper",
    "app_main_main",
    "app_main -contains-> app_main_helper",
    "app_main -contains-> app_main_main",
    "app_main_main -calls-> app_main_helper",
];

#[test]
fn mixed_fixture_matches_graphify() {
    common::assert_fixture("mixed", ALLOWED, common::Communities::Exact, true, false);
}
