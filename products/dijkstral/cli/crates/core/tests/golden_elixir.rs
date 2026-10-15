//! Parity with Graphify's clustered golden for `tests/fixtures/elixir`, plus focused checks of the
//! Elixir rules (docs/graphify-format.md §29) on small probe repositories.

mod common;

use std::collections::BTreeSet;
use std::fs;

use dijkstral_core::{build, BuildOptions};
use serde_json::Value;

/// Documented, deliberate differences (quirks not reproduced; see docs/graphify-format.md §29).
/// Each entry is a line as printed by the parity summary.
const ALLOWED: &[&str] = &[];

#[test]
fn elixir_fixture_matches_graphify() {
    common::assert_fixture("elixir", ALLOWED, common::Communities::Exact, true);
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
fn bare_calls_bind_last_in_file_then_through_imported_files() {
    // Probe e2/e3 of §29: a bare call binds the file's last function of that name, whatever
    // module defines it; across files only the files of `import`/`use` modules are searched.
    let l = probe_links(&[
        (
            "lib/x.ex",
            "defmodule X do\n  import Y\n  def f(a) do\n    g(a)\n    X.g(a)\n    only_in_y(a)\n    y_and_w(a)\n    nowhere(a)\n  end\n  def g(a), do: a\nend\n\ndefmodule Z do\n  def g(a), do: a\nend\n",
        ),
        (
            "lib/y.ex",
            "defmodule Y do\n  def only_in_y(a), do: a\n  def y_and_w(a), do: a\nend\n\ndefmodule W do\n  def y_and_w(a), do: a\nend\n",
        ),
        ("lib/r.ex", "defmodule R do\n  def nowhere(a), do: a\nend\n"),
    ]);
    for want in [
        "lib_x_x_f -calls-> lib_x_z_g [call]",
        "lib_x_x_f -calls-> lib_x_x_g [call]",
        "lib_x_x_f -calls-> lib_y_y_only_in_y [call]",
        "lib_x -imports-> lib_y_y [import]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // Two candidates in the imported file, and a function outside any imported file, bind
    // nothing.
    assert!(
        !l.iter()
            .any(|x| x.contains("-calls->") && (x.contains("y_and_w") || x.contains("nowhere"))),
        "{l:#?}"
    );
}

#[test]
fn nested_modules_alias_and_stay_file_local() {
    let l = probe_links(&[
        (
            "lib/m.ex",
            "defmodule M do\n  def go(x) do\n    Inner.deep(x)\n    Shop.B.run(x)\n  end\n  defmodule Inner do\n    def deep(x), do: x\n  end\n  defmodule Shop.Other do\n    def o(x), do: x\n  end\nend\n",
        ),
        (
            "lib/n.ex",
            "defmodule Shop.B do\n  def run(x), do: x\nend\n\ndefmodule N do\n  def n(x), do: Inner.deep(x)\nend\n",
        ),
    ]);
    // `Inner` is `M.Inner` inside M; `defmodule Shop.Other` makes `Shop` mean `M.Shop`, so
    // `Shop.B.run` binds nothing; a nested module is not reachable by name from another file.
    assert!(
        l.contains("lib_m_m_go -calls-> lib_m_inner_deep [call]"),
        "{l:#?}"
    );
    assert!(l.contains("lib_m -contains-> lib_m_inner"), "{l:#?}");
    assert!(
        !l.iter().any(|x| x.contains("-calls-> lib_n_shop_b_run")),
        "{l:#?}"
    );
    assert!(
        !l.iter().any(|x| x.starts_with("lib_n_n_n -calls->")),
        "{l:#?}"
    );
}

#[test]
fn imports_resolve_to_modules_files_or_externals() {
    let l = probe_links(&[
        (
            "lib/q.ex",
            "defmodule Outer do\n  defmodule R do\n  end\nend\n\ndefmodule Twice do\nend\n",
        ),
        ("lib/w/v.ex", "defmodule Vmod do\nend\n"),
        ("lib/t.ex", "defmodule Twice do\nend\n"),
        (
            "lib/imp.ex",
            "defmodule Imp do\n  alias Q.R\n  alias W.V\n  alias Vmod\n  use Twice\n  import :timer\n  require Logger\nend\n",
        ),
        ("config/config.exs", "import Config\n"),
    ]);
    for want in [
        "lib_imp -imports-> lib_q_r [import]",
        "lib_imp -imports-> lib_w_v [import]",
        "lib_imp -imports-> lib_w_v_vmod [import]",
        "lib_imp -imports-> twice [import]",
        "lib_imp -imports-> logger [import]",
    ] {
        assert!(l.contains(want), "missing {want}: {l:#?}");
    }
    // An import that lands on the importing file itself is dropped; atoms import nothing.
    assert!(
        !l.iter().any(|x| x.starts_with("config_config -imports->")),
        "{l:#?}"
    );
    assert!(!l.iter().any(|x| x.contains("timer")), "{l:#?}");
}
