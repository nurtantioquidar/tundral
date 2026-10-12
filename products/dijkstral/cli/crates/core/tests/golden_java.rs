//! Parity with Graphify's clustered golden for `tests/fixtures/java`.

mod common;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q11/J1: `import com.shop.model.*;` targets Graphify's class `Model` because the package's
    // last segment `model` equals that class's stub id; dijkstral keeps the external `model`.
    "model",
    "src_main_java_com_shop_service_userservice -imports-> model",
    "src_main_java_com_shop_service_userservice -imports-> src_main_java_com_shop_model_model_model",
    // Q11/J2: `import static com.shop.service.Formatter.slug;` links every class named
    // `Formatter`; dijkstral links only the one its FQN names (the main `Formatter`).
    "src_main_java_com_shop_service_userservice -imports-> src_test_java_com_shop_service_userservicetest_formatter",
];

#[test]
fn java_fixture_matches_graphify() {
    // Community ids move with the ALLOWED differences (NMI 0.92); Graphify orders two equal-score
    // surprises differently from G.edges() here (graphify-format.md §17.7).
    common::assert_fixture("java", ALLOWED, 0.42, false, true);
}
