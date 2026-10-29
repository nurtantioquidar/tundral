//! Ingestion and chunking of `tests/fixtures/semantic/mixed` against what Graphify sent
//! (`docs/graphify-semantic.md` §2, §4, the cache keys of §9.1 in `mixed.cache_keys.json`).

use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_core::discover::discover_all;
use dijkstral_semantic::ingest::{self, cache_key, chunk, converted_name, IngestOptions};
use dijkstral_semantic::types::DocKind;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantic")
}

fn ingest_mixed(converted_dir: Option<&Path>) -> ingest::Ingested {
    let root = fixtures().join("mixed");
    let found = discover_all(&root, 2);
    ingest::ingest(
        &root,
        &found.documents,
        &IngestOptions {
            converted_prefix: "graphify-out/converted",
            converted_dir,
            workspace: None,
        },
    )
}

/// The text pypdf extracts from `papers/write_through.pdf` (Graphify's `pdf` extra).
const PYPDF_TEXT: &str = "Write-Through Caching for Job Queues\nA. Example, B. Sample - Monet Labs, 2026\nAbstract\nWe study a write-through cache placed in front of a durable job queue. Each\njob is pushed through JobQueue.push and slugified before storage. The cache\nreduces read latency by 41 percent on the QueueBench workload while keeping\nexactly-once delivery. We compare it with the lazy invalidation scheme of\nSmith et al. [1] and show that write-through avoids stale reads.\nReferences\n[1] J. Smith, L. Wong. Lazy Invalidation in Distributed Caches. 2019.";

#[test]
fn dispatch_order_kinds_and_estimates() {
    let out = ingest_mixed(None);
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let got: Vec<(&str, DocKind, usize)> = out
        .docs
        .iter()
        .map(|d| (d.dispatched(), d.kind, d.tokens))
        .collect();
    assert_eq!(
        got,
        [
            ("README.md", DocKind::Document, 142),
            ("docs/glossary.txt", DocKind::Document, 81),
            ("docs/guide.md", DocKind::Document, 92),
            (
                "graphify-out/converted/capacity_e7fd8f6a.md",
                DocKind::Document,
                76
            ),
            (
                "graphify-out/converted/runbook_b3207a85.md",
                DocKind::Document,
                91
            ),
            ("images/architecture.png", DocKind::RasterImage, 1600),
            ("papers/write_through.pdf", DocKind::Paper, 177),
        ]
    );
    let image = out.docs[5].image.as_ref().unwrap();
    assert_eq!(image.0, "image/png");
    assert_eq!(image.1.len(), 9_371);
    assert_eq!(out.docs[3].rel, "office/capacity.xlsx");
}

#[test]
fn pdf_text_matches_pypdf() {
    let out = ingest_mixed(None);
    let paper = out.docs.iter().find(|d| d.kind == DocKind::Paper).unwrap();
    assert_eq!(paper.text, PYPDF_TEXT);
}

#[test]
fn office_files_convert_like_graphify() {
    assert_eq!(
        converted_name("office/capacity.xlsx"),
        "capacity_e7fd8f6a.md"
    );
    assert_eq!(converted_name("office/runbook.docx"), "runbook_b3207a85.md");
    let dir = tempfile::tempdir().unwrap();
    let out = ingest_mixed(Some(dir.path()));
    let capacity = &out.docs[3];
    assert_eq!(
        capacity.text,
        "<!-- converted from capacity.xlsx -->\n\n## Sheet: Capacity\n| service | jobs_per_s |\n| --- | --- |\n| JobQueue | 120 |\n| Cache | 900 |"
    );
    assert_eq!(
        out.docs[4].text,
        "<!-- converted from runbook.docx -->\n\nQueue Runbook\nWhen the queue backs up, call run_job for each pending job and watch the cache hit rate.\nEscalation\nPage the on-call owner of JobQueue if latency exceeds 2 seconds."
    );
    let written = fs::read_to_string(dir.path().join("capacity_e7fd8f6a.md")).unwrap();
    assert_eq!(written, capacity.text);
}

/// Every dispatched file's §9.1 cache key equals Graphify's, which also proves the converted
/// office files are byte-identical and the front matter rule holds (`README.md`).
#[test]
fn cache_keys_match_graphify() {
    let keys: Value =
        serde_json::from_slice(&fs::read(fixtures().join("mixed.cache_keys.json")).unwrap())
            .unwrap();
    let root = fixtures().join("mixed");
    let out = ingest_mixed(None);
    for doc in &out.docs {
        let path = doc.dispatched();
        let bytes = match &doc.converted {
            Some(_) => doc.text.as_bytes().to_vec(),
            None => fs::read(root.join(&doc.rel)).unwrap(),
        };
        let want = &keys["entries"][path][path.to_lowercase()];
        assert_eq!(
            Value::String(cache_key(path, &bytes)),
            *want,
            "cache key of {path}"
        );
    }
}

#[test]
fn budget_250_chunks_like_graphify() {
    let out = ingest_mixed(None);
    let chunks = chunk(&out.docs, 250);
    let names: Vec<Vec<&str>> = chunks
        .iter()
        .map(|c| {
            c.parts
                .iter()
                .map(|p| {
                    assert!(p.piece.is_none());
                    let d = &out.docs[p.doc];
                    d.rel.rsplit('/').next().unwrap()
                })
                .collect()
        })
        .collect();
    assert_eq!(
        names,
        [
            vec!["README.md", "glossary.txt"],
            vec!["guide.md", "capacity.xlsx"],
            vec!["runbook.docx"],
            vec!["architecture.png"],
            vec!["write_through.pdf"],
        ]
    );
    // The default budget sends everything in one request, as the recorded extract did.
    assert_eq!(chunk(&out.docs, ingest::DEFAULT_TOKEN_BUDGET).len(), 1);
}
