//! Parity with Graphify's clustered golden for `tests/fixtures/dart` (docs/graphify-format.md
//! §27).
//!
//! One link field difference remains and is printed, not counted: a `variable_type`
//! reference location (which duplicate Graphify keeps, quirk DA2).

mod common;
mod probe;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §27).
/// None: the fixture's node and link sets are reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn dart_fixture_matches_graphify() {
    common::assert_fixture("dart", ALLOWED, common::Communities::Exact, true);
}

/// `typedef X = …` gives nothing when the file already defines `X`'s id, and `typedef` is never
/// a variable type (graphify-format.md §27.1).
#[test]
fn dart_typedefs() {
    let l = probe::links(&[(
        "lib/a.dart",
        "class Foo {}\n\ntypedef FOO = Other;\n\ntypedef Validator = bool Function(int x);\n\ntypedef Alias = Foo;\n",
    )]);
    for want in [
        "lib_a -defines-> lib_a_foo",
        "lib_a -defines-> lib_a_validator",
        "lib_a_alias -references-> lib_a_foo [typedef]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    for gone in [
        "lib_a_foo -references-> other [typedef]",
        "lib_a -references-> typedef [variable_type]",
    ] {
        assert!(!l.contains(gone), "{gone}: {l:#?}");
    }
}

/// `extension on T` is one node per file, `Extension on T` of the first (quirk DA3: its id holds
/// the file's absolute path); annotation nodes never bind to a definition (§27.3).
#[test]
fn dart_anonymous_extensions_and_annotations() {
    let files = [(
        "lib/a.dart",
        "int ffi = 1;\n\n@ffi.Packed(1)\nfinal class Packed {}\n\nextension on List<int> {\n  int get total => 0;\n}\n\nextension on Map<String, int> {\n  int get size2 => 0;\n}\n",
    )];
    // One build: the anonymous extension's id depends on where the probe was written.
    let g = probe::graph(&files);
    let l: Vec<String> = g["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{} -{}-> {}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap()
            )
        })
        .collect();
    assert!(
        l.contains(&"lib_a_packed -configures-> annotation_ffi".to_string()),
        "{l:#?}"
    );
    let ext: Vec<&serde_json::Value> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["label"].as_str().unwrap().starts_with("Extension on"))
        .collect();
    assert_eq!(ext.len(), 1, "{ext:#?}");
    assert_eq!(ext[0]["label"], "Extension on List");
    assert_eq!(ext[0]["source_location"], "L6");
    let id = ext[0]["id"].as_str().unwrap();
    assert!(
        id.starts_with("lib_a_") && id.ends_with("_lib_a_anonymous_extension"),
        "{id}"
    );
    for target in ["list", "map"] {
        assert!(l.contains(&format!("{id} -extends-> {target}")), "{l:#?}");
    }
}
