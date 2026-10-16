//! `save-result` and `reflect` against Graphify's goldens (`tests/fixtures/memory/`,
//! `docs/graphify-memory.md` §2–§4, §7), and the memory directory as corpus content (§3).
//!
//! Saved files must match Graphify's byte for byte apart from the time stamp, the random token
//! and `contributor`. Lessons are compared as data: Graphify's `LESSONS.md` and dijkstral's own
//! wording are both parsed into the same lists (sections, items in order, counts, leans, dates).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn cases() -> Value {
    serde_json::from_slice(&fs::read(fixtures().join("memory/cases.json")).unwrap()).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// The day the goldens were captured (2026-10-16): their 2030 and 2031 results stay in the future.
const CAPTURED: &str = "1792108800";

fn run_in(dir: &Path, args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dijkstral"))
        .args(args)
        .env("DIJKSTRAL_NOW", CAPTURED)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            md_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// The front-matter lines whose values differ by design: the time stamp and the contributor.
fn mask(text: &str) -> String {
    text.split('\n')
        .map(|l| {
            if l.starts_with("date: ") {
                "date: <masked>"
            } else if l.starts_with("contributor: ") {
                "contributor: <masked>"
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cases whose file differs on purpose, with the reason.
const SAVE_DIFFS: &[(&str, &str)] = &[(
    "bogus_type",
    "dijkstral escapes --type like every other value (quirk M1)",
)];

#[test]
fn save_result_matches_graphify() {
    let cases = cases();
    let mut checked = 0;
    for c in cases["save_result"].as_array().unwrap() {
        let name = c["case"].as_str().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("answer.txt"),
            "answer from file\n  second line  \n\n",
        )
        .unwrap();
        let out = run_in(tmp.path(), &strings(&c["argv"]));
        let code = out.status.code().unwrap();
        assert_eq!(
            code as i64,
            c["exit"].as_i64().unwrap(),
            "{name}: exit code"
        );
        if code != 0 {
            assert!(!out.stderr.is_empty(), "{name}: error message");
            continue;
        }
        let mut files = Vec::new();
        md_files(tmp.path(), &mut files);
        assert_eq!(files.len(), 1, "{name}: one file written");
        let rel = files[0]
            .strip_prefix(tmp.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let want = c["saved_path"].as_str().unwrap().replacen(
            "graphify-out/memory/",
            "dijkstral-out/memory/",
            1,
        );
        let (want_dir, want_name) = want.rsplit_once('/').unwrap();
        let (got_dir, got_name) = rel.rsplit_once('/').unwrap();
        assert_eq!(got_dir, want_dir, "{name}: directory");
        // query_YYYYMMDD_HHMMSS_<8 hex>_<slug>.md: the 31-byte prefix varies.
        let shape = |n: &str| {
            let b = n.as_bytes();
            n.starts_with("query_")
                && b[6..14].iter().all(u8::is_ascii_digit)
                && b[14] == b'_'
                && b[15..21].iter().all(u8::is_ascii_digit)
                && b[21] == b'_'
                && n[22..30]
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                && b[30] == b'_'
        };
        assert!(shape(got_name), "{name}: file name {got_name}");
        assert_eq!(&got_name[31..], &want_name[31..], "{name}: slug");
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(stdout, format!("Saved {rel}\n"), "{name}: stdout");
        let got = fs::read_to_string(&files[0]).unwrap();
        assert!(got.contains("contributor: \"dijkstral\"\n"), "{name}");
        let golden =
            fs::read_to_string(fixtures().join("memory").join(c["file"].as_str().unwrap()))
                .unwrap();
        if let Some((_, why)) = SAVE_DIFFS.iter().find(|(n, _)| *n == name) {
            let strip = |t: &str| {
                mask(t)
                    .split('\n')
                    .filter(|l| !l.starts_with("type: "))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            assert_eq!(strip(&got), strip(&golden), "{name} ({why})");
            assert!(got.contains("type: \"Weird \\\"type\\\"\""), "{name}");
        } else {
            assert_eq!(mask(&got), mask(&golden), "{name}");
        }
        checked += 1;
    }
    assert_eq!(checked, 21);
}

/// One block of lessons: its kind and its items, normalised to comparable strings.
type Blocks = Vec<(&'static str, Vec<String>)>;

#[derive(Debug, PartialEq)]
struct Lessons {
    summary: (u64, u64, u64, u64),
    all: Blocks,
    topics: Vec<(String, Blocks)>,
}

/// Inline code: the content of the first backtick span of `s` and what follows it. Graphify's
/// spans always use one backtick (`single`); dijkstral's fence grows past backticks in the text.
fn take_code(s: &str, single: bool) -> (String, &str) {
    let fence = if single {
        1
    } else {
        s.len() - s.trim_start_matches('`').len()
    };
    let rest = &s[fence..];
    let close = "`".repeat(fence);
    // Graphify's spans are one backtick around raw text that may itself hold backticks: the span
    // ends at the first closing fence followed by what can follow a name.
    let candidates: Vec<usize> = rest.match_indices(close.as_str()).map(|(i, _)| i).collect();
    let end = candidates
        .iter()
        .copied()
        .find(|&i| {
            let after = &rest[i + fence..];
            after.is_empty()
                || after.starts_with(" (")
                || after.starts_with(" — ")
                || after.starts_with(", ")
        })
        .unwrap_or(*candidates.last().unwrap());
    let mut code = rest[..end].to_string();
    // Padding spaces (dijkstral pads spans that start or end with a backtick, and empty ones).
    let padded = code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ');
    if padded && (fence > 1 || code.trim().is_empty()) {
        code = code[1..code.len() - 1].to_string();
    }
    (code, &rest[end + fence..])
}

fn codes(mut s: &str, single: bool) -> Vec<String> {
    let mut out = Vec::new();
    while s.starts_with('`') {
        let (c, rest) = take_code(s, single);
        out.push(c);
        s = rest.strip_prefix(", ").unwrap_or(rest);
    }
    out
}

/// Graphify's item text, with line breaks inside an item joined by spaces (dijkstral flattens
/// them so one item stays on one line).
fn graphify_items(lines: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for l in lines {
        if let Some(rest) = l.strip_prefix("- ") {
            items.push(rest.to_string());
        } else if let Some(last) = items.last_mut() {
            last.push(' ');
            last.push_str(l);
        }
    }
    items
        .into_iter()
        .map(|i| i.replace(['\r', '\u{2028}', '\u{2029}'], " "))
        .collect()
}

fn graphify_block(header: &str, lines: &[&str]) -> (&'static str, Vec<String>) {
    let kind = if header.starts_with("**Preferred sources**") {
        "preferred"
    } else if header.starts_with("**Tentative**") {
        "tentative"
    } else if header.starts_with("**Contested**") {
        "contested"
    } else if header.starts_with("**Known dead ends**") {
        "dead"
    } else if header.starts_with("**Corrections**") {
        "corrections"
    } else {
        panic!("unknown block {header}")
    };
    let items = graphify_items(lines)
        .into_iter()
        .map(|it| match kind {
            "preferred" | "tentative" => {
                let (name, rest) = take_code(&it, true);
                let n = rest.trim_start_matches(" (").trim_end_matches("× useful)");
                format!("{name}|{n}")
            }
            "contested" => {
                let (name, rest) = take_code(&it, true);
                let rest = rest.trim_start_matches(" — ");
                let (counts, lean) = rest.split_once(" → ").unwrap();
                let (u, n) = counts.split_once(", ").unwrap();
                let u = u.trim_end_matches("× useful");
                let n = n.trim_end_matches("× dead end/corrected");
                let (lean, latest) = match lean.split_once(" (latest ") {
                    Some((l, d)) => (l, d.trim_end_matches(')')),
                    None => (lean, ""),
                };
                let lean = match lean {
                    "recency leans **useful**" => "useful",
                    "recency leans **dead end**" => "dead",
                    "evenly split" => "even",
                    other => panic!("lean {other}"),
                };
                format!("{name}|{u}|{n}|{lean}|{latest}")
            }
            "dead" => {
                let body = it.strip_prefix('"').unwrap();
                let (q, names) = match body.rsplit_once("\" — `") {
                    Some((q, rest)) => (q, codes(&format!("`{rest}"), true)),
                    None => (body.strip_suffix('"').unwrap(), Vec::new()),
                };
                format!("{q}|{}", names.join("\u{1f}"))
            }
            _ => {
                let body = it.strip_prefix('"').unwrap();
                let (q, fix) = body.split_once("\" → ").unwrap();
                format!("{q}|{}", fix.trim_end())
            }
        })
        .collect();
    (kind, items)
}

fn dijkstral_block(header: &str, lines: &[&str]) -> (&'static str, Vec<String>) {
    let kind = if header.starts_with("**Start here**") {
        "preferred"
    } else if header.starts_with("**Unconfirmed**") {
        "tentative"
    } else if header.starts_with("**Mixed signals**") {
        "contested"
    } else if header.starts_with("**Dead ends**") {
        "dead"
    } else if header.starts_with("**Corrections**") {
        "corrections"
    } else {
        panic!("unknown block {header}")
    };
    let items = lines
        .iter()
        .map(|l| l.strip_prefix("- ").expect("one line per item"))
        .map(|it| match kind {
            "preferred" | "tentative" => {
                let (name, rest) = take_code(it, false);
                format!("{name}|{}", rest.trim_start_matches(" — useful in "))
            }
            "contested" => {
                let (name, rest) = take_code(it, false);
                let rest = rest.trim_start_matches(" — useful in ");
                let (u, rest) = rest.split_once(", dead end or corrected in ").unwrap();
                let (n, lean) = rest.split_once("; ").unwrap();
                let (lean, latest) = match lean.split_once(" (last seen ") {
                    Some((l, d)) => (l, d.trim_end_matches(')')),
                    None => (lean, ""),
                };
                let lean = match lean {
                    "now mostly useful" => "useful",
                    "now mostly a dead end" => "dead",
                    "evenly balanced" => "even",
                    other => panic!("lean {other}"),
                };
                format!("{name}|{u}|{n}|{lean}|{latest}")
            }
            "dead" => {
                let mut de = serde_json::Deserializer::from_str(it).into_iter::<String>();
                let q = de.next().unwrap().unwrap();
                let rest = &it[de.byte_offset()..];
                let names = rest
                    .strip_prefix(" — ")
                    .map(|r| codes(r, false))
                    .unwrap_or_default();
                format!("{q}|{}", names.join("\u{1f}"))
            }
            _ => {
                let mut de = serde_json::Deserializer::from_str(it).into_iter::<String>();
                let q = de.next().unwrap().unwrap();
                let fix = it[de.byte_offset()..].strip_prefix(" → ").unwrap();
                format!("{q}|{}", fix.trim_end())
            }
        })
        .collect();
    (kind, items)
}

/// Parse a `LESSONS.md`; `ours` picks dijkstral's wording, else Graphify's.
fn parse_lessons(text: &str, ours: bool) -> Lessons {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut summary = (0, 0, 0, 0);
    let mut all: Blocks = Vec::new();
    let mut topics: Vec<(String, Blocks)> = Vec::new();
    let mut in_topics = false;
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l == "## Summary" {
            let s = lines[i + 2].trim_start_matches("- ");
            let n: Vec<u64> = s
                .split(" · ")
                .map(|p| p.split(' ').next().unwrap().parse().unwrap())
                .collect();
            summary = (n[0], n[1], n[2], n[3]);
        } else if l == "## By topic" || l == "## By community" {
            in_topics = true;
        } else if let Some(t) = l.strip_prefix("### ") {
            topics.push((t.to_string(), Vec::new()));
        } else if l.starts_with("**") {
            let mut j = i + 2;
            let mut body = Vec::new();
            while j < lines.len() && !lines[j].is_empty() {
                body.push(lines[j]);
                j += 1;
            }
            let block = if ours {
                dijkstral_block(l, &body)
            } else {
                graphify_block(l, &body)
            };
            if in_topics {
                topics.last_mut().unwrap().1.push(block);
            } else {
                all.push(block);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    Lessons {
        summary,
        all,
        topics,
    }
}

/// The counts in a `reflect` stdout line.
fn counts(line: &str) -> Vec<u64> {
    line.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect()
}

#[test]
fn reflect_matches_graphify() {
    let cases = cases();
    let mut checked = 0;
    for c in cases["reflect"].as_array().unwrap() {
        let name = c["case"].as_str().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(
            &fixtures().join(c["memory"].as_str().unwrap()),
            &tmp.path().join("memory"),
        );
        for (rel, src) in c["inputs"].as_object().unwrap() {
            let dest = tmp.path().join(rel);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::copy(fixtures().join(src.as_str().unwrap()), dest).unwrap();
        }
        let out = run_in(tmp.path(), &strings(&c["argv"]));
        assert!(out.status.success(), "{name}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        let want_counts = counts(c["stdout"].as_str().unwrap());
        let got_counts = counts(&stdout);
        // Graphify: total, useful, dead ends, corrected; dijkstral adds the unmarked count.
        assert_eq!(got_counts[..4], want_counts[..4], "{name}: {stdout}");
        let got = fs::read_to_string(tmp.path().join("out/LESSONS.md")).unwrap();
        assert!(got.ends_with('\n') && !got.ends_with("\n\n"), "{name}");
        let golden = fs::read_to_string(
            fixtures()
                .join("memory")
                .join(c["lessons"].as_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            parse_lessons(&got, true),
            parse_lessons(&golden, false),
            "{name}:\n{got}"
        );
        checked += 1;
    }
    assert_eq!(checked, 12);
}

#[test]
fn reflect_is_deterministic_and_if_stale_notices_deletions() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("dijkstral-out/memory");
    copy_dir(
        &fixtures().join("memory/reflect/decay_and_dedupe/memory"),
        &mem,
    );
    let reflect = |extra: &[&str]| {
        let mut args = vec!["reflect".to_string()];
        args.extend(extra.iter().map(|s| s.to_string()));
        run_in(tmp.path(), &args)
    };
    assert!(reflect(&[]).status.success());
    let lessons = tmp.path().join("dijkstral-out/reflections/LESSONS.md");
    let first = fs::read(&lessons).unwrap();
    assert!(reflect(&[]).status.success());
    assert_eq!(fs::read(&lessons).unwrap(), first);
    let skipped = reflect(&["--if-stale"]);
    assert!(String::from_utf8_lossy(&skipped.stdout).contains("up to date"));
    // Deleting a result changes the directory, so the lessons are stale.
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::remove_file(mem.join("q01.md")).unwrap();
    let rerun = reflect(&["--if-stale"]);
    assert!(!String::from_utf8_lossy(&rerun.stdout).contains("up to date"));
    assert_ne!(fs::read(&lessons).unwrap(), first);
}

#[test]
fn update_indexes_saved_results_like_graphify_out_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("python");
    copy_dir(&fixtures().join("python"), &root);
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let saved = run_in(
        &root,
        &args(&[
            "save-result",
            "--question",
            "Where is Circle area?",
            "--answer",
            "In models",
            "--nodes",
            "Circle",
            "--outcome",
            "dead_end",
        ]),
    );
    assert!(saved.status.success());
    let built = run_in(&root, &args(&["update", "."]));
    assert!(built.status.success(), "{built:?}");
    let g: Value =
        serde_json::from_slice(&fs::read(root.join("dijkstral-out/graph.json")).unwrap()).unwrap();
    let nodes: Vec<&Value> = g["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| {
            n["source_file"]
                .as_str()
                .is_some_and(|f| f.starts_with("dijkstral-out/memory/query_"))
        })
        .collect();
    // Graphify: one page and the four headings, the front matter on the page.
    let mut kinds: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in &nodes {
        kinds
            .entry(n["node_kind"].as_str().unwrap().to_string())
            .or_default()
            .push(n["label"].as_str().unwrap().to_string());
    }
    let mut headings = kinds["heading"].clone();
    headings.sort();
    assert_eq!(
        headings,
        [
            "Answer",
            "Outcome",
            "Q: Where is Circle area?",
            "Source Nodes"
        ]
    );
    let page = nodes.iter().find(|n| n["node_kind"] == "page").unwrap();
    assert!(page["id"]
        .as_str()
        .unwrap()
        .starts_with("dijkstral_out_memory_query_"));
    assert_eq!(page["frontmatter"]["outcome"], "dead_end");
    assert_eq!(page["frontmatter"]["contributor"], "dijkstral");

    // The semantic pass never sends saved results to a model: with the handoff backend (which
    // only writes requests to disk), a code corpus plus a saved result gives no request at all.
    let extracted = run_in(&root, &args(&["extract", ".", "--backend", "handoff"]));
    assert!(extracted.status.success(), "{extracted:?}");
    assert!(!root.join("dijkstral-out/semantic/pending").exists());
}
