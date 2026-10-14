//! Parity with Graphify's clustered golden for `tests/fixtures/svelte`, plus probes of the
//! Svelte rules (docs/graphify-format.md §39).

mod common;
mod probe;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // SV2 (the Q9 family): every import of a Svelte or Astro file whose specifier does not
    // resolve to a local file also gets a phantom code node named after its last segment, with an
    // `imports_from` link without context or line, beside the `ref_lib` external. Dijkstral keeps
    // the external only.
    "badge_svelte",
    "cart_svelte",
    "src_routes_page_svelte_src_routes_page -imports_from-> badge_svelte",
    "src_routes_page_svelte_src_routes_page -imports_from-> cart_svelte",
];

#[test]
fn svelte_fixture_matches_graphify() {
    // The two phantom nodes move community ids; the partition is otherwise the same.
    common::assert_fixture(
        "svelte",
        ALLOWED,
        common::Communities::MinNmi(0.95),
        false,
        false,
    );
}

#[test]
fn props_without_a_value_have_no_node_and_no_types_cross_files() {
    let l = probe::links(&[
        (
            "lib.ts",
            "export interface T { a: number }\nexport class Base {}\nexport default class D {}\n",
        ),
        (
            "S.svelte",
            "<script lang=\"ts\">\nimport type { T } from './lib'\nimport D, { Base } from './lib'\nexport let item: T\nexport let size = 1\ninterface P { t: T }\nclass K extends Base {}\n</script>\n<p>{item.a}</p>\n",
        ),
    ]);
    for want in [
        "s -contains-> s_size",
        "s -imports-> lib_t [import]",
        "s -imports-> lib_base [import]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    for absent in [
        "s -contains-> s_item",
        // No default-import link, no heritage, no type references.
        "s -imports-> lib_d [import]",
        "s_k -inherits-> lib_base [type]",
        "s_p -references-> lib_t [field]",
    ] {
        assert!(!l.contains(absent), "unexpected {absent}: {l:#?}");
    }
}
