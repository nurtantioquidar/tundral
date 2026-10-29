//! Parity with Graphify's clustered golden for `tests/fixtures/scala`.

mod common;
mod probe;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §22).
/// None: the fixture is reproduced exactly.
const ALLOWED: &[&str] = &[];

#[test]
fn scala_fixture_matches_graphify() {
    common::assert_fixture("scala", ALLOWED, common::Communities::Exact, true);
}

/// Type parameters of the enclosing classes and `def`s are never referenced; an undeclared one
/// still is (graphify-format.md §22).
#[test]
fn scala_type_parameters_are_not_referenced() {
    let l = probe::links(&[(
        "a/A.scala",
        "package a\n\ntrait Repo[K, V] {\n  def get(k: K): Option[V]\n}\n\nclass Box[+T](val item: T) {\n  def map[U](f: T => U): U = f(item)\n}\n\ndef stray(): T = ???\n",
    )]);
    assert!(
        l.contains("a_a_stray -references-> t [return_type]"),
        "{l:#?}"
    );
    for gone in [
        "a_a_repo_get -references-> k [parameter_type]",
        "a_a_repo_get -references-> v [generic_arg]",
        "a_a_box -references-> t [field]",
        "a_a_box_map -references-> u [return_type]",
    ] {
        assert!(!l.contains(gone), "{gone}: {l:#?}");
    }
}
