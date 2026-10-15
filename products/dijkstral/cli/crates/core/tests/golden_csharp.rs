//! Parity with Graphify's clustered golden for `tests/fixtures/csharp`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §18).
/// Each entry is a line as printed by the parity summary. None: node and link sets match.
/// Three link *field* differences remain (printed, not asserted): quirk CS1, Graphify merges the
/// partial `User` after folding links, so `User -method-> .CompareTo()` / `.Of()` absorb the
/// reversed reference's or call's `context` and `metadata`.
const ALLOWED: &[&str] = &[];

#[test]
fn csharp_fixture_matches_graphify() {
    common::assert_fixture("csharp", ALLOWED, common::Communities::Exact, true);
}
