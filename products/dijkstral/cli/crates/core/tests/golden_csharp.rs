//! Parity with Graphify's clustered golden for `tests/fixtures/csharp`.

mod common;
mod probe;

use serde_json::Value;

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

fn node<'g>(g: &'g Value, id: &str) -> &'g Value {
    g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node {id}"))
}

/// Overloads after the first take `_<sha1(signature)[:6]>` unless they repeat the previous
/// signature; every method records `arity`, `min_arity`, `variadic` and `signature` (§18.1).
#[test]
fn overloads_get_signature_ids_and_metadata() {
    let g = probe::graph(&[(
        "A.cs",
        "namespace N {\n\
         public class K {\n\
           public void Do() { }\n\
           public void Do(int a, string b = \"x\", params object[] rest) { }\n\
           public void Do(List<Dictionary<string, int>> m) { }\n\
           public T Gen<T, U>(T a, ref readonly int r) => a;\n\
           public void Twin(int a) { }\n\
           public void Twin(int b) { }\n\
         }\n\
         public static class E { public static void Shout(this string s, int n = 1) { } }\n\
         public delegate void Changed(K k);\n\
         public partial class P { }\n\
         }\n",
    )]);
    let meta = |id: &str| node(&g, id)["metadata"].to_string();
    assert_eq!(
        meta("a_n_k_do"),
        r#"{"arity":0,"signature":"","namespace":"N","scope_chain":["s0:0"]}"#
    );
    assert_eq!(
        meta("a_n_k_do_4f6389"),
        r#"{"arity":3,"min_arity":1,"variadic":true,"signature":"int,string,params object[]","namespace":"N","scope_chain":["s0:0"]}"#
    );
    // The hash is of the unescaped text.
    assert_eq!(
        node(&g, "a_n_k_do_520dc3")["metadata"]["signature"],
        "List&lt;Dictionary&lt;string,int&gt;&gt;"
    );
    assert_eq!(
        node(&g, "a_n_k_gen")["metadata"]["signature"],
        "`2:T,ref readonly int"
    );
    assert_eq!(
        meta("a_n_e_shout"),
        r#"{"arity":2,"min_arity":0,"signature":"this string,int","namespace":"N","scope_chain":["s0:0"]}"#
    );
    assert!(g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["id"] != "a_n_k_twin_46f8ab"));
    assert_eq!(node(&g, "a_n_changed")["_callable_class"], true);
    assert_eq!(node(&g, "a_n_p")["metadata"]["type_path"], "P");
}

/// Calls pick the only overload that takes their argument count, else the first; a `var`
/// initialized by a call to a method without overloads has its declared return type, and
/// `foreach (var x in field)`, `field[i]`, `a.f` and `new T()` type receivers too.
#[test]
fn calls_pick_overloads_and_type_receivers() {
    let l = probe::scored_links(&[
        (
            "Item.cs",
            "namespace F {\n\
             public class Item {\n\
               public Item Next;\n\
               public static Item Of(string s) => null;\n\
               public void A() { } public void B() { } public void C() { } public void D() { }\n\
               public void E() { } public void G() { }\n\
               public void Q(int a, int b) { } public void Q(int a) { }\n\
             }\n\
             public interface IRun { void Run(); }\n\
             public class R1 : IRun { public void Run() { } }\n\
             public class R2 : IRun { public void Run() { } }\n\
             }\n",
        ),
        (
            "Use.cs",
            "using System.Collections.Generic;\n\
             namespace F {\n\
             public class Use {\n\
               Item[] _items;\n\
               List<Item> _list;\n\
               public void M1(Item p) { p.Q(1); p.Q(); }\n\
               public void M2() { var x = Item.Of(\"a\"); x.A(); }\n\
               public void M3() { foreach (var x in _list) { x.B(); } }\n\
               public void M4() { _items[0].C(); }\n\
               public void M5(Item p) { p.Next.D(); }\n\
               public void M6() { new Item().E(); }\n\
               public void M7(Item p) { p.Next.Next.G(); }\n\
             }\n\
             }\n",
        ),
    ]);
    for want in [
        "use_f_use_m1 -calls-> item_f_item_q_46f8ab 0.85 L6",
        "use_f_use_m2 -calls-> item_f_item_a 0.85 L7",
        "use_f_use_m3 -calls-> item_f_item_b 0.85 L8",
        "use_f_use_m4 -calls-> item_f_item_c 0.85 L9",
        "use_f_use_m5 -calls-> item_f_item_d 0.85 L10",
        "use_f_use_m6 -calls-> item_f_item_e 0.85 L11",
        "item_f_irun_run -dispatches_to-> item_f_r1_run 0.75 L10",
        "item_f_irun_run -dispatches_to-> item_f_r2_run 0.75 L11",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // `p.Q()` fits no overload: the first. Two field steps type nothing.
    assert!(
        l.contains("use_f_use_m1 -calls-> item_f_item_q 0.85 L6"),
        "{l:#?}"
    );
    assert!(
        !l.iter().any(|x| x.contains("-calls-> item_f_item_g ")),
        "{l:#?}"
    );
}
