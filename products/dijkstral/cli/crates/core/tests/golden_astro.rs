//! Parity with Graphify's clustered golden for `tests/fixtures/astro`, plus probes of the Astro
//! rules (docs/graphify-format.md §40).

mod common;
mod probe;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn astro_fixture_matches_graphify() {
    common::assert_fixture("astro", ALLOWED, common::Communities::Exact, true);
}

#[test]
fn frontmatter_and_script_blocks_are_one_module() {
    let n = probe::nodes(&[(
        "P.astro",
        "---\nconst title = makeTitle()\nfunction makeTitle() { return 'x' }\n---\n<h1>{title}</h1>\n<script>\nfunction client() { return 1 }\n</script>\n<p>---</p>\n",
    )]);
    for want in [
        "p_title title L2",
        "p_maketitle makeTitle() L3",
        "p_client client() L7",
    ] {
        assert!(n.contains(want), "missing {want}: {n:#?}");
    }
}
