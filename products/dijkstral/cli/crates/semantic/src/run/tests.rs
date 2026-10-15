//! Runner tests with a scripted fake backend and the recorded Codex transcript: repair, retries,
//! bisection, concurrency, the cache, the shrink guard and the hand-off round trip.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::backend::handoff::{self, Handoff};
use super::backend::replay::Replay;
use super::cache::{self, sha256_hex};
use super::*;
use crate::types::{Backend, Chunk, DocKind, Part, Request, Response, SourceDoc};

type Script = dyn Fn(&[String], usize) -> Result<Response> + Send + Sync;
type Delay = dyn Fn(&[String]) -> Duration + Send + Sync;

/// A backend answering through `script(files of the request, call number from 1)`.
struct Fake {
    model: String,
    script: Box<Script>,
    calls: Mutex<Vec<Vec<String>>>,
    delay: Option<Box<Delay>>,
}

impl Fake {
    fn new(script: impl Fn(&[String], usize) -> Result<Response> + Send + Sync + 'static) -> Fake {
        Fake {
            model: "fake-1".into(),
            script: Box::new(script),
            calls: Mutex::new(Vec::new()),
            delay: None,
        }
    }

    /// Answers with `nodes(file)` concept nodes for every file of the request.
    fn nodes(n: impl Fn(&str) -> usize + Send + Sync + 'static) -> Fake {
        Fake::new(move |files, _| Ok(ok(&reply_for(files, &n))))
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl Backend for Fake {
    fn name(&self) -> &str {
        "fake"
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn complete(&self, req: &Request) -> Result<Response> {
        let files = request_files(req);
        if let Some(d) = &self.delay {
            std::thread::sleep(d(&files));
        }
        let n = {
            let mut c = self.calls.lock().unwrap();
            c.push(files.clone());
            c.len()
        };
        (self.script)(&files, n)
    }
}

fn request_files(req: &Request) -> Vec<String> {
    let first = req.user_text.lines().next().unwrap();
    let list = first
        .split_once("one of: ")
        .unwrap()
        .1
        .trim_end_matches('.');
    list.split(", ").map(String::from).collect()
}

fn stem(rel: &str) -> String {
    rel.rsplit_once('.')
        .map_or(rel, |(s, _)| s)
        .replace(['/', '.', '-'], "_")
        .to_lowercase()
}

fn reply_for(files: &[String], n: &dyn Fn(&str) -> usize) -> Value {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for f in files {
        let s = stem(f);
        for i in 0..n(f) {
            nodes.push(json!({"id": format!("{s}_c{i}"), "label": format!("C{i}"),
                              "file_type": "concept", "source_file": f}));
            if i > 0 {
                edges.push(
                    json!({"source": format!("{s}_c0"), "target": format!("{s}_c{i}"),
                                  "relation": "references", "confidence": "EXTRACTED",
                                  "source_file": f}),
                );
            }
        }
    }
    json!({"nodes": nodes, "edges": edges, "hyperedges": []})
}

fn ok(v: &Value) -> Response {
    Response {
        text: v.to_string(),
        input_tokens: Some(100),
        output_tokens: Some(10),
        model: "fake-1".into(),
        truncated: false,
    }
}

fn doc(rel: &str, text: &str) -> SourceDoc {
    SourceDoc {
        rel: rel.into(),
        kind: DocKind::Document,
        text: text.into(),
        image: None,
        sha256: sha256_hex(text.as_bytes()),
        tokens: 10,
    }
}

fn chunks(groups: &[&[usize]]) -> Vec<Chunk> {
    groups
        .iter()
        .enumerate()
        .map(|(index, g)| Chunk {
            index,
            parts: g.iter().map(|&doc| Part { doc, piece: None }).collect(),
        })
        .collect()
}

fn opts(out: &Path) -> ExtractOptions {
    ExtractOptions {
        retry_delays: vec![Duration::ZERO, Duration::ZERO],
        ..ExtractOptions::new(out)
    }
}

fn states(o: &ExtractOutcome) -> Vec<FileState> {
    o.files.iter().map(|(_, s)| *s).collect()
}

fn corpus() -> Vec<SourceDoc> {
    vec![
        doc("README.md", "---\ntitle: Jobs\n---\n# Jobs\nRun them.\n"),
        doc("docs/guide.md", "# Guide\n"),
        doc("docs/glossary.txt", "Job: work\n"),
    ]
}

#[test]
fn rerun_with_nothing_changed_makes_no_call() {
    let out = tempfile::tempdir().unwrap();
    let mut docs = corpus();
    let ch = chunks(&[&[0, 1], &[2]]);
    let fake = Fake::nodes(|_| 2);
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(fake.calls().len(), 2);
    assert_eq!(states(&o), vec![FileState::Complete; 3]);
    assert_eq!(o.extracted.len(), 2);
    assert!(!o.extracted[0].cached);
    assert_eq!(o.extracted[0].files, vec!["README.md", "docs/guide.md"]);
    assert_eq!(o.usage.requests, 2);
    assert_eq!(o.usage.input_tokens, 200);
    let usage: Value =
        serde_json::from_slice(&std::fs::read(out.path().join("semantic/usage.json")).unwrap())
            .unwrap();
    assert_eq!(usage["extract"]["requests"], 2);

    // Unchanged: everything from the cache, one reply per file, in dispatch order.
    let fake2 = Fake::nodes(|_| 2);
    let o = extract(&ch, &docs, &fake2, &opts(out.path())).unwrap();
    assert!(fake2.calls().is_empty());
    assert_eq!(states(&o), vec![FileState::Cached; 3]);
    assert_eq!(o.summary(), "semantic cache: 3 hit / 0 miss");
    assert_eq!(o.usage.requests, 0);
    let files: Vec<&str> = o.extracted.iter().map(|e| e.files[0].as_str()).collect();
    assert_eq!(
        files,
        vec!["README.md", "docs/guide.md", "docs/glossary.txt"]
    );
    assert!(o
        .extracted
        .iter()
        .all(|e| e.cached && e.reply.nodes.len() == 2));
    assert_eq!(o.extracted[0].reply.edges.len(), 1);

    // Front matter only: still a hit.
    docs[0].text = "---\ntitle: Jobs v2\n---\n# Jobs\nRun them.\n".into();
    let fake3 = Fake::nodes(|_| 2);
    extract(&ch, &docs, &fake3, &opts(out.path())).unwrap();
    assert!(fake3.calls().is_empty());

    // A changed body re-sends only that file; the other files of its chunk are not sent.
    docs[1].text = "# Guide\nNew section.\n".into();
    let fake4 = Fake::nodes(|_| 2);
    let o = extract(&ch, &docs, &fake4, &opts(out.path())).unwrap();
    assert_eq!(fake4.calls(), vec![vec!["docs/guide.md".to_string()]]);
    assert_eq!(
        states(&o),
        vec![FileState::Cached, FileState::Complete, FileState::Cached]
    );
    assert_eq!(
        cache_misses(&docs, &fake4, &opts(out.path())),
        Vec::<usize>::new()
    );

    // --force asks again; another model or mode has its own cache.
    let fake5 = Fake::nodes(|_| 2);
    let forced = ExtractOptions {
        force: true,
        ..opts(out.path())
    };
    extract(&ch, &docs, &fake5, &forced).unwrap();
    assert_eq!(fake5.calls().len(), 2);
    let mut other = Fake::nodes(|_| 2);
    other.model = "fake-2".into();
    assert_eq!(
        cache_misses(&docs, &other, &opts(out.path())),
        vec![0, 1, 2]
    );
    extract(&ch, &docs, &other, &opts(out.path())).unwrap();
    assert_eq!(other.calls().len(), 2);
    let deep = ExtractOptions {
        mode: Mode::Deep,
        ..opts(out.path())
    };
    let fake6 = Fake::nodes(|_| 2);
    extract(&ch, &docs, &fake6, &deep).unwrap();
    assert_eq!(fake6.calls().len(), 2);
}

#[test]
fn repairs_fenced_and_prose_replies() {
    let out = tempfile::tempdir().unwrap();
    let docs = corpus();
    let fake = Fake::new(|files, n| {
        let body = reply_for(files, &|_| 1).to_string();
        Ok(Response {
            text: if n == 1 {
                format!("```json\n{body}\n```")
            } else {
                format!("Sure! Here it is:\n{body}\nLet me know.")
            },
            ..Response::default()
        })
    });
    let o = extract(&chunks(&[&[0], &[1, 2]]), &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(states(&o), vec![FileState::Complete; 3]);
    assert_eq!(fake.calls().len(), 2);
}

#[test]
fn hollow_and_invalid_replies_are_retried_then_given_up() {
    let out = tempfile::tempdir().unwrap();
    let docs = corpus();
    // Hollow, invalid, then good: three attempts, complete.
    let fake = Fake::new(|files, n| match n {
        1 => Ok(ok(&json!({"nodes": [], "edges": []}))),
        2 => Ok(Response {
            text: "{'nodes': []}".into(),
            ..Response::default()
        }),
        _ => Ok(ok(&reply_for(files, &|_| 1))),
    });
    let o = extract(&chunks(&[&[0]]), &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(fake.calls().len(), 3);
    assert_eq!(states(&o), vec![FileState::Complete]);
    assert!(o
        .warnings
        .iter()
        .any(|w| w.contains("returned no nodes/edges")));
    assert!(o
        .warnings
        .iter()
        .any(|w| w.starts_with("LLM returned invalid JSON (first 200 chars: {'nodes': []})")));
    let outcomes: Vec<&str> = o.usage.records.iter().map(|r| r.outcome.as_str()).collect();
    assert_eq!(outcomes, vec!["hollow", "invalid", "ok"]);

    // Always hollow: given up after 3 attempts, partial entry, asked again next run.
    let fake = Fake::new(|_, _| Ok(ok(&json!({"nodes": []}))));
    let o = extract(&chunks(&[&[1]]), &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(fake.calls().len(), 3);
    assert_eq!(states(&o), vec![FileState::Partial]);
    assert!(o
        .warnings
        .iter()
        .any(|w| w == "chunk of 1 still hollow after 3 attempt(s)"));
    assert!(o.extracted.is_empty());
    let fake = Fake::nodes(|_| 1);
    let o = extract(&chunks(&[&[1]]), &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(fake.calls().len(), 1);
    assert_eq!(states(&o), vec![FileState::Complete]);

    // A failing backend: not cached, asked again next run.
    let fake = Fake::new(|_, _| bail!("HTTP 500"));
    let o = extract(&chunks(&[&[2]]), &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(fake.calls().len(), 3);
    assert_eq!(states(&o), vec![FileState::Failed]);
    assert_eq!(cache_misses(&docs[2..], &fake, &opts(out.path())), vec![0]);
}

#[test]
fn truncation_bisects_to_single_files() {
    let out = tempfile::tempdir().unwrap();
    let docs: Vec<SourceDoc> = (0..4)
        .map(|i| doc(&format!("d{i}.md"), &format!("# {i}")))
        .collect();
    let fake = Fake::new(|files, _| {
        let mut r = ok(&reply_for(files, &|_| 2));
        r.truncated = files.len() > 1;
        Ok(r)
    });
    let o = extract(&chunks(&[&[0, 1, 2, 3]]), &docs, &fake, &opts(out.path())).unwrap();
    let sizes: Vec<usize> = fake.calls().iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![4, 2, 1, 1, 2, 1, 1]);
    assert_eq!(states(&o), vec![FileState::Complete; 4]);
    assert_eq!(o.extracted.len(), 4);
    assert!(o.warnings[0]
        .starts_with("chunk of 4 truncated at depth 0, splitting into halves of 2 and 2"));

    // A truncated single file keeps what parsed, uncached.
    let fake = Fake::new(|_, _| {
        Ok(Response {
            text: r#"{"nodes":[{"id":"x_a","label":"A","source_file":"x.md"},{"id":"x_b","lab"#
                .into(),
            truncated: true,
            ..Response::default()
        })
    });
    let docs = vec![doc("x.md", "# X")];
    let o = extract(&chunks(&[&[0]]), &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(states(&o), vec![FileState::Partial]);
    assert!(o.extracted[0].partial);
    assert_eq!(o.extracted[0].reply.nodes.len(), 1);
    assert_eq!(cache_misses(&docs, &fake, &opts(out.path())), vec![0]);
}

#[test]
fn concurrency_one_is_strictly_ordered_and_results_never_reorder() {
    let docs: Vec<SourceDoc> = (0..6)
        .map(|i| doc(&format!("d{i}.md"), &format!("# {i}")))
        .collect();
    let ch = chunks(&[&[0], &[1], &[2], &[3], &[4], &[5]]);
    let expect: Vec<String> = (0..6).map(|i| format!("d{i}.md")).collect();
    for concurrency in [1, 4] {
        let out = tempfile::tempdir().unwrap();
        let mut fake = Fake::nodes(|_| 1);
        // Early chunks answer slowest, so with concurrency later replies arrive first.
        fake.delay = Some(Box::new(|files| {
            let i: u64 = files[0][1..2].parse().unwrap();
            Duration::from_millis(60 - 10 * i)
        }));
        let o = extract(
            &ch,
            &docs,
            &fake,
            &ExtractOptions {
                max_concurrency: concurrency,
                ..opts(out.path())
            },
        )
        .unwrap();
        let order: Vec<String> = fake.calls().into_iter().map(|c| c[0].clone()).collect();
        if concurrency == 1 {
            assert_eq!(order, expect);
        } else {
            assert_ne!(order, expect);
        }
        let got: Vec<String> = o.extracted.iter().map(|e| e.files[0].clone()).collect();
        assert_eq!(got, expect);
    }
}

#[test]
fn omitted_files_are_retried_once_then_cached_empty() {
    let out = tempfile::tempdir().unwrap();
    let docs = corpus();
    let ch = chunks(&[&[0, 1]]);
    let only_readme = || {
        Fake::new(|files, _| {
            let keep: Vec<String> = files
                .iter()
                .filter(|f| *f == "README.md")
                .cloned()
                .collect();
            if keep.is_empty() {
                return Ok(ok(
                    &json!({"nodes": [{"id": "x", "source_file": "elsewhere.md"}]}),
                ));
            }
            Ok(ok(&reply_for(&keep, &|_| 1)))
        })
    };
    let fake = only_readme();
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(states(&o), vec![FileState::Complete, FileState::Omitted]);
    assert!(o.warnings.iter().any(|w| w
        == "WARNING: 1/2 dispatched file(s) produced no nodes (docs/guide.md); a re-run will retry them"));
    let fake = only_readme();
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(fake.calls(), vec![vec!["docs/guide.md".to_string()]]);
    assert_eq!(states(&o), vec![FileState::Cached, FileState::Omitted]);
    let fake = only_readme();
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert!(fake.calls().is_empty());
    assert_eq!(states(&o), vec![FileState::Cached, FileState::Cached]);
}

#[test]
fn shrink_guard_refuses_unless_allowed() {
    let out = tempfile::tempdir().unwrap();
    let mut docs = vec![doc("README.md", "# A\n")];
    let ch = chunks(&[&[0]]);
    extract(&ch, &docs, &Fake::nodes(|_| 5), &opts(out.path())).unwrap();
    docs[0].text = "# A, shorter\n".into();
    let fake = Fake::nodes(|_| 1);
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert_eq!(
        o.shrunk,
        vec![Shrink {
            rel: "README.md".into(),
            before: 5,
            after: 1
        }]
    );
    assert!(o
        .shrink_error()
        .unwrap()
        .starts_with("unverified semantic shrink detected for 'README.md' (5 -> 1 nodes)"));
    // The reply is cached (no second model call), but the guard holds until --allow-partial.
    let fake = Fake::nodes(|_| 1);
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert!(fake.calls().is_empty());
    assert_eq!(o.shrunk.len(), 1);
    let allow = ExtractOptions {
        allow_partial: true,
        ..opts(out.path())
    };
    let o = extract(&ch, &docs, &fake, &allow).unwrap();
    assert!(o.shrunk.is_empty() && o.shrink_error().is_none());
    let o = extract(&ch, &docs, &fake, &opts(out.path())).unwrap();
    assert!(o.shrunk.is_empty());
}

#[test]
fn handoff_round_trip() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("README.md"), "# Jobs\n").unwrap();
    std::fs::write(root.path().join("diagram.png"), [137, 80, 78, 71]).unwrap();
    let out = root.path().join("dijkstral-out");
    let mut img = doc("diagram.png", "");
    img.kind = DocKind::RasterImage;
    img.image = Some(("image/png".into(), vec![137, 80, 78, 71]));
    let docs = vec![doc("README.md", "# Jobs\n"), img, doc("notes.txt", "n\n")];
    let ch = chunks(&[&[0, 1], &[2]]);
    let o = ExtractOptions {
        root: Some(root.path().to_path_buf()),
        ..opts(&out)
    };
    cache::mark_needs_update(&out).unwrap();

    let first = extract(&ch, &docs, &Handoff, &o).unwrap();
    assert_eq!(states(&first), vec![FileState::Pending; 3]);
    assert_eq!(first.pending.len(), 2);
    assert_eq!(first.usage.requests, 0);
    assert!(cache::needs_update_path(&out).exists());
    let pending = handoff::list_pending(&out).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].files.len(), 2);
    assert!(pending[0].system.contains("\"hyperedges\""));
    assert!(pending[0].user_text.contains("=== FILE: README.md"));
    assert_eq!(
        PathBuf::from(&pending[0].images[0].path),
        std::path::absolute(root.path().join("diagram.png")).unwrap()
    );

    // The host assistant answers request 1 (fenced) and writes junk for request 2.
    let replies = handoff::replies_dir(&out);
    std::fs::create_dir_all(&replies).unwrap();
    let files: Vec<String> = pending[0].files.iter().map(|f| f.rel.clone()).collect();
    std::fs::write(
        replies.join("1.json"),
        format!("```json\n{}\n```", reply_for(&files, &|_| 2)),
    )
    .unwrap();
    std::fs::write(replies.join("2.json"), "I could not read it").unwrap();
    let report = import(&out, &replies).unwrap();
    assert_eq!(report.imported, vec![1]);
    assert_eq!(report.files, 2);
    assert_eq!(report.rejected.len(), 1);
    assert!(report
        .summary()
        .starts_with("imported 1 reply (2 file(s)); 0 missing; 1 rejected"));
    // A reply about another request's files is rejected.
    std::fs::write(
        replies.join("2.json"),
        reply_for(&["README.md".into()], &|_| 1).to_string(),
    )
    .unwrap();
    let report = import(&out, &replies).unwrap();
    assert!(report.rejected[0]
        .1
        .starts_with("no node has a source_file of this request"));
    std::fs::write(
        replies.join("2.json"),
        reply_for(&["notes.txt".into()], &|_| 1).to_string(),
    )
    .unwrap();
    let report = import(&out, &replies).unwrap();
    assert_eq!(report.imported, vec![2]);
    assert!(handoff::list_pending(&out).unwrap().is_empty());

    // Now everything is cached: no request, nothing pending, replies ready to merge.
    let done = extract(&ch, &docs, &Handoff, &o).unwrap();
    assert_eq!(states(&done), vec![FileState::Cached; 3]);
    assert!(done.pending.is_empty());
    assert_eq!(done.extracted.len(), 3);
    assert_eq!(done.extracted[1].reply.nodes.len(), 2);
    assert!(!cache::needs_update_path(&out).exists());
    assert_eq!(pending_changes(root.path(), &out), None);
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/semantic")
}

/// The mixed fixture's text documents and its PNG, as `ingest` frames them.
fn mixed_docs() -> Vec<SourceDoc> {
    let root = fixture().join("mixed");
    let mut docs: Vec<SourceDoc> = [
        "README.md",
        "config/queue.yaml",
        "docs/glossary.txt",
        "docs/guide.md",
    ]
    .iter()
    .map(|rel| doc(rel, &std::fs::read_to_string(root.join(rel)).unwrap()))
    .collect();
    let png = std::fs::read(root.join("images/architecture.png")).unwrap();
    let mut img = doc("images/architecture.png", "");
    img.kind = DocKind::RasterImage;
    img.sha256 = sha256_hex(&png);
    img.image = Some(("image/png".into(), png));
    docs.push(img);
    docs
}

/// One real `codex exec` on the mixed fixture (four documents and the PNG in one request, no
/// retry), recorded for `dijkstral_transcript_replays_by_fingerprint`:
///
/// ```text
/// DIJKSTRAL_CODEX_SMOKE=/tmp/smoke cargo test -p dijkstral-semantic codex_smoke -- --ignored --nocapture
/// ```
#[test]
#[ignore = "calls the real Codex CLI"]
fn codex_smoke() {
    let Some(dir) = std::env::var_os("DIJKSTRAL_CODEX_SMOKE") else {
        return;
    };
    let settings = BackendSettings {
        kind: Some(BackendKind::Codex),
        ..BackendSettings::default()
    };
    let codex = select(&settings, &Env::system()).unwrap();
    let rec = backend::record::Recorder::new(codex, PathBuf::from(dir)).unwrap();
    let out = tempfile::tempdir().unwrap();
    let docs = mixed_docs();
    let o = ExtractOptions {
        retry_delays: Vec::new(),
        max_concurrency: 1,
        root: Some(fixture().join("mixed")),
        ..ExtractOptions::new(out.path())
    };
    let r = extract(&chunks(&[&[0, 1, 2, 3, 4]]), &docs, &rec, &o).unwrap();
    eprintln!("{:?}\n{}\n{}", r.files, r.summary(), r.usage.summary());
    for w in &r.warnings {
        eprintln!("{w}");
    }
    let count = |f: fn(&Extracted) -> usize| r.extracted.iter().map(f).sum::<usize>();
    eprintln!(
        "nodes {}, edges {}, hyperedges {}",
        count(|e| e.reply.nodes.len()),
        count(|e| e.reply.edges.len()),
        count(|e| e.reply.hyperedges.len())
    );
}

#[test]
fn codex_transcript_replays_into_the_cache() {
    let out = tempfile::tempdir().unwrap();
    let docs = mixed_docs();
    let ch = chunks(&[&[0, 1, 2, 3, 4]]);
    let replay = Replay::open(&fixture().join("mixed.transcript")).unwrap();
    let o = ExtractOptions {
        root: Some(fixture().join("mixed")),
        ..opts(out.path())
    };
    let first = extract(&ch, &docs, &replay, &o).unwrap();
    assert_eq!(states(&first), vec![FileState::Complete; 5]);
    assert_eq!(first.usage.requests, 1);
    assert_eq!(first.usage.input_tokens, 16843);
    assert_eq!(first.usage.output_tokens, 3275);
    let raw = &first.extracted[0].raw;
    let readme_nodes = raw["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["source_file"] == "README.md")
        .count();
    assert!(readme_nodes > 0);

    // The rerun is served from the cache, attributed per file.
    let replay = Replay::open(&fixture().join("mixed.transcript")).unwrap();
    let again = extract(&ch, &docs, &replay, &o).unwrap();
    assert_eq!(states(&again), vec![FileState::Cached; 5]);
    assert_eq!(again.usage.requests, 0);
    assert_eq!(again.extracted[0].reply.nodes.len(), readme_nodes);
    let fresh: usize = first.extracted[0]
        .reply
        .nodes
        .iter()
        .filter(|n| {
            docs.iter()
                .any(|d| Some(d.rel.as_str()) == n.source_file.as_deref())
        })
        .count();
    let cached: usize = again.extracted.iter().map(|e| e.reply.nodes.len()).sum();
    assert_eq!(fresh, cached);
}

#[test]
fn dijkstral_transcript_replays_by_fingerprint() {
    let dir = fixture().join("mixed.dijkstral.transcript");
    let out = tempfile::tempdir().unwrap();
    let docs = mixed_docs();
    let replay = Replay::open(&dir).unwrap();
    let req =
        prompt::extraction_request(&docs, &chunks(&[&[0, 1, 2, 3, 4]])[0].parts, Mode::Normal);
    let recorded: Value =
        serde_json::from_slice(&std::fs::read(dir.join("req-0001.json")).unwrap()).unwrap();
    assert_eq!(
        recorded["fingerprint"],
        backend::replay::fingerprint(&req),
        "the recorded request no longer matches today's prompt (PROMPT_VERSION {PROMPT_VERSION}); \
         re-record the transcript"
    );
    let o = extract(
        &chunks(&[&[0, 1, 2, 3, 4]]),
        &docs,
        &replay,
        &opts(out.path()),
    )
    .unwrap();
    assert_eq!(states(&o), vec![FileState::Complete; 5]);
}

/// The mixed fixture's communities as `label` would describe them.
fn mixed_communities() -> Vec<Community> {
    let c = |id: &str, labels: &[&str]| Community {
        id: id.into(),
        labels: labels.iter().map(|l| l.to_string()).collect(),
    };
    vec![
        c(
            "0",
            &[
                "Write-through cache",
                "Keep write-through caching",
                "Queue configuration",
                "Write-through",
            ],
        ),
        c(
            "1",
            &[
                "JobQueue",
                "run_job()",
                "JobQueue.push",
                "Job",
                "service.py",
            ],
        ),
        c(
            "2",
            &[
                "Job Service",
                "Operator Guide",
                "Glossary",
                "Architecture diagram",
            ],
        ),
        c("3", &["slugify()", "Slug", "util.py"]),
    ]
}

/// One real community-naming request through Codex, recorded like `codex_smoke`:
///
/// ```text
/// DIJKSTRAL_CODEX_SMOKE=/tmp/smoke cargo test -p dijkstral-semantic codex_label_smoke -- --ignored --nocapture
/// ```
#[test]
#[ignore = "calls the real Codex CLI"]
fn codex_label_smoke() {
    let Some(dir) = std::env::var_os("DIJKSTRAL_CODEX_SMOKE") else {
        return;
    };
    let settings = BackendSettings {
        kind: Some(BackendKind::Codex),
        ..BackendSettings::default()
    };
    let codex = select(&settings, &Env::system()).unwrap();
    let rec = backend::record::Recorder::new(codex, PathBuf::from(dir)).unwrap();
    let out = name_communities(&mixed_communities(), &rec, &LabelOptions::default());
    eprintln!(
        "{:?}\n{}\n{}",
        out.names,
        out.summary(),
        out.usage.summary()
    );
}

#[test]
fn community_names_replay_from_codex() {
    let dir = fixture().join("mixed.dijkstral.transcript");
    let batch: Vec<(String, Vec<String>)> = mixed_communities()
        .into_iter()
        .map(|c| {
            let labels = c
                .labels
                .iter()
                .map(|l| l.trim_end_matches("()").to_string())
                .collect();
            (c.id, labels)
        })
        .collect();
    let recorded: Value =
        serde_json::from_slice(&std::fs::read(dir.join("req-0002.json")).unwrap()).unwrap();
    assert_eq!(
        recorded["fingerprint"],
        backend::replay::fingerprint(&prompt::label_request(&batch))
    );
    let replay = Replay::open(&dir).unwrap();
    let out = name_communities(&mixed_communities(), &replay, &LabelOptions::default());
    assert_eq!(out.names.len(), 4, "{:?}", out.names);
    assert!(out.unnamed.is_empty());
    assert_eq!(out.usage.requests, 1);
}
