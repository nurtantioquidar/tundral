//! Parity with Graphify's clustered golden for `tests/fixtures/ruby`, plus focused checks of the
//! Ruby rules (docs/graphify-format.md §20) on small probe repositories.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §15).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn ruby_fixture_matches_graphify() {
    common::assert_fixture("ruby", ALLOWED, common::Communities::Exact, false, false);
}

/// Build a probe repository and return its links as `source -relation-> target [context]`.
fn probe_links(files: &[(&str, &str)]) -> BTreeSet<String> {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("probe");
    for (path, text) in files {
        let p = root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads: 1,
        cluster: false,
    })
    .unwrap();
    let g: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    g["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let ctx = e["context"]
                .as_str()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            format!(
                "{} -{}-> {}{}",
                e["source"].as_str().unwrap(),
                e["relation"].as_str().unwrap(),
                e["target"].as_str().unwrap(),
                ctx
            )
        })
        .collect()
}

#[test]
fn calls_bind_by_receiver_kind() {
    let l = probe_links(&[(
        "a.rb",
        "class A\n  def m; end\n  def go\n    x.m\n    self.m\n    helper.n\n  end\n  def helper; end\nend\nclass B\n  def m; end\n  def n; end\n  def outer\n    def inner; end\n  end\nend\n",
    )]);
    for want in [
        // `x.m` binds to the file's last `m`, `self.m` to the owner's.
        "a_a_go -calls-> a_b_m [call]",
        "a_a_go -calls-> a_a_m [call]",
        "a_a_go -calls-> a_b_n [call]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // A receiver name is not a call; a `def` inside a method is not a node.
    assert!(!l.contains("a_a_go -calls-> a_a_helper [call]"), "{l:#?}");
    assert!(!l.iter().any(|x| x.contains("inner")), "{l:#?}");
}

#[test]
fn constants_resolve_by_suffix_and_mixins_lexically() {
    let l = probe_links(&[
        (
            "lib/s/base.rb",
            "module S\n  module Helpers\n    def h; end\n  end\n  class Base\n    include Helpers\n    def go\n      Helpers.unknown\n      Request.new\n      Response.new\n    end\n  end\n  class Request; end\n  class Response; end\nend\n",
        ),
        (
            "lib/x/other.rb",
            "module X\n  module Helpers; end\n  class Request; end\nend\n",
        ),
    ]);
    for want in [
        "lib_s_base_s_base -mixes_in-> lib_s_base_s_helpers [mixin]",
        "lib_s_base_s_base_go -calls-> lib_s_base_s_response [call]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // `Request` and `Helpers` are ambiguous by suffix, so calls on them bind nothing.
    for absent in [
        "lib_s_base_s_base_go -calls-> lib_s_base_s_request [call]",
        "lib_s_base_s_base_go -calls-> lib_s_base_s_helpers [call]",
    ] {
        assert!(!l.contains(absent), "unexpected {absent}: {l:#?}");
    }
}
