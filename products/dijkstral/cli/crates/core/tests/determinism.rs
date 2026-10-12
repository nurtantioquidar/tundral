//! Clustered builds are byte-identical across runs and thread counts: graph.json, the analysis
//! sidecar and the report. The generated repository has more than 1,000 nodes, so the report's
//! betweenness takes the sampled path.

use std::fs;
use std::path::Path;

use dijkstral_core::{build, BuildOptions};

fn write_repo(root: &Path) {
    for m in 0..120 {
        let mut src = String::new();
        if m > 0 {
            src.push_str(&format!("from pkg.m{} import f0 as prev\n\n\n", m - 1));
        }
        for f in 0..8 {
            src.push_str(&format!("def f{f}(x):\n"));
            if f > 0 {
                src.push_str(&format!("    f{}(x)\n", f - 1));
            }
            if f == 0 && m > 0 {
                src.push_str("    prev(x)\n");
            }
            src.push_str("    return x\n\n\n");
        }
        src.push_str(&format!(
            "class K{m}:\n    def run(self):\n        return f7(1)\n"
        ));
        let path = root.join(format!("pkg/m{m}.py"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, src).unwrap();
    }
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
}

fn outputs(threads: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>, usize) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write_repo(&root);
    let report = build(&BuildOptions {
        root: root.clone(),
        out: None,
        threads,
        cluster: true,
    })
    .unwrap();
    let out = root.join("dijkstral-out");
    (
        fs::read(out.join("graph.json")).unwrap(),
        fs::read(out.join(".graphify_analysis.json")).unwrap(),
        fs::read(out.join("GRAPH_REPORT.md")).unwrap(),
        report.nodes,
    )
}

#[test]
fn clustered_outputs_do_not_depend_on_threads_or_runs() {
    let one = outputs(1);
    assert!(
        one.3 > 1000,
        "the repository must exercise sampled betweenness"
    );
    for threads in [1, 4] {
        let other = outputs(threads);
        assert!(
            one.0 == other.0,
            "graph.json differs with {threads} threads"
        );
        assert!(one.1 == other.1, "analysis differs with {threads} threads");
        assert!(one.2 == other.2, "report differs with {threads} threads");
    }
}

#[test]
fn no_cluster_writes_no_communities_and_no_sidecars() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write_repo(&root);
    for cluster in [true, false] {
        build(&BuildOptions {
            root: root.clone(),
            out: None,
            threads: 2,
            cluster,
        })
        .unwrap();
    }
    let out = root.join("dijkstral-out");
    let graph = fs::read_to_string(out.join("graph.json")).unwrap();
    assert!(!graph.contains("\"community\""));
    assert!(!out.join(".graphify_analysis.json").exists());
    assert!(!out.join("GRAPH_REPORT.md").exists());
}
