//! Parity with Graphify's clustered golden for `tests/fixtures/vue`, plus probes of the
//! single-file-component rules (docs/graphify-format.md §38).

mod common;
mod probe;

use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[
    // Q8: `createApp(App).use(store).mount(selector)` inside `mount()` binds `.mount()` to the
    // last same-file function named `mount`, a self-loop. Dijkstral binds member calls by
    // receiver type only.
    "src_main_mount -calls-> src_main_mount",
];

#[test]
fn vue_fixture_matches_graphify() {
    let (built, agreement) = common::assert_golden("vue", 1.0, 1.0, ALLOWED);
    assert!(
        agreement.exact() == 1.0,
        "vue: community ids differ on {:.3} of shared nodes",
        1.0 - agreement.exact()
    );
    let order = |g: &Value| -> Vec<String> {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    };
    let (ours, golden) = (order(&built.ours), order(&built.golden));
    let ours: Vec<&String> = ours.iter().filter(|i| golden.contains(i)).collect();
    let golden: Vec<&String> = golden.iter().filter(|i| ours.contains(i)).collect();
    assert_eq!(ours, golden, "vue: node array order");
    // Not exact: the allowed Q8 self-loop changes `mount()`'s degree, and the bridge question
    // below.
    common::check_report_goldens("vue", &built, false);
}

/// `dijkstral cluster-only` on Graphify's own graph matches its communities, order, god nodes
/// and surprises, but asks one more bridge question (`formatTitle()`, whose only neighbour in
/// another community is its own file node); Graphify appears not to count same-file neighbours
/// when it lists the communities a bridge connects (graphify-format.md §38.6).
#[test]
#[ignore = "bridge questions count same-file neighbours: graphify-format.md §38.6"]
fn vue_cluster_only_matches_graphify() {
    common::assert_cluster_only("vue", false);
}

#[test]
fn script_blocks_keep_outer_lines_and_comments_are_not_rationale() {
    let l = probe::nodes(&[(
        "C.vue",
        "<template>\n  <!-- <script>function inComment() {}</script> -->\n</template>\n<SCRIPT lang=\"ts\">\n// NOTE: ignored\nexport function a() { return 1 }\n</SCRIPT>\n<script setup>\nfunction b() { return a() }\n</script>\n",
    )]);
    for want in ["c_incomment inComment() L2", "c_a a() L6", "c_b b() L9"] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    assert!(!l.iter().any(|n| n.contains("rationale")), "{l:#?}");
}

#[test]
fn grammar_follows_lang_and_default_exports_use_typescript() {
    let l = probe::links(&[
        // `lang="tsx"`: the JSX parses, but a default export whose body holds JSX is not
        // importable (Graphify finds defaults with the TypeScript grammar).
        (
            "X.vue",
            "<script lang=\"tsx\">\nexport default function X() { return <div/> }\nexport const AFTER = 1\n</script>\n",
        ),
        // A default declared after the JSX is found (the TypeScript parse nests it in `J`).
        (
            "Y.vue",
            "<script lang=\"tsx\">\nexport function J() { return <div/> }\nexport const AFTER = 1\nexport default function Y() { return 1 }\n</script>\n",
        ),
        // No `lang`: TypeScript, so JSX swallows what follows it.
        (
            "Z.vue",
            "<script>\nexport function J() { return <i/> }\nexport const LOST = 1\n</script>\n",
        ),
        (
            "use.ts",
            "import X from './X.vue'\nimport Y from './Y.vue'\nimport { AFTER } from './X.vue'\n",
        ),
    ]);
    for want in [
        "x -contains-> x_after",
        "y -contains-> y_y",
        "use -imports-> y_y [import]",
        "use -imports-> x_after [import]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    assert!(!l.contains("use -imports-> x_x [import]"), "{l:#?}");
    assert!(!l.iter().any(|x| x.contains("z_lost")), "{l:#?}");
}

#[test]
fn self_imports_and_cross_kind_inferred_calls_are_dropped() {
    let l = probe::links(&[
        (
            "K.vue",
            "<script>\nimport { k } from './K.vue'\nexport class K2 { go() { return 3 } }\nexport function k() { return 1 }\n</script>\n",
        ),
        (
            "use.ts",
            "import { K2 } from './K.vue'\nexport function u() { const x = new K2(); x.go() }\n",
        ),
    ]);
    assert!(l.contains("k -imports-> k_k [import]"), "{l:#?}");
    assert!(!l.contains("k -imports_from-> k [import]"), "{l:#?}");
    assert!(l.contains("use_u -calls-> k_k2 [call]"), "{l:#?}");
    assert!(!l.contains("use_u -calls-> k_k2_go [call]"), "{l:#?}");
}
