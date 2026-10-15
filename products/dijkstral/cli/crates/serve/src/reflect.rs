//! `reflect`: saved results folded into lessons (`docs/graphify-memory.md` §4). No model; the same
//! inputs give the same lessons.
//!
//! The aggregation, ordering and topic grouping are Graphify's; `LESSONS.md` has Graphify's
//! sections in Graphify's order, worded by dijkstral.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::clock;
use crate::memory::{Outcome, Saved};
use crate::Index;

/// Where lessons go by default, relative to the project.
pub const LESSONS_FILE: &str = "dijkstral-out/reflections/LESSONS.md";
/// The topic of results with no name in any community.
pub const UNCATEGORIZED: &str = "Uncategorized";
pub const DEFAULT_HALF_LIFE_DAYS: f64 = 30.0;
pub const DEFAULT_MIN_CORROBORATION: i64 = 2;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Weights halve every this many days; zero, negative or non-finite disables decay.
    pub half_life_days: f64,
    /// Distinct useful results a name needs to be preferred.
    pub min_corroboration: i64,
    /// Unix seconds the ages are measured from.
    pub now: f64,
}

/// What the graph contributes: the names that still exist, and each name's topic.
#[derive(Debug, Default, Clone)]
pub struct GraphContext {
    /// Node ids and labels; names outside it are stale and dropped.
    pub names: HashSet<String>,
    /// Name → topic, when an analysis assigns communities.
    pub topics: Option<HashMap<String, String>>,
}

impl GraphContext {
    /// From an index plus an optional `.graphify_analysis.json` and `.graphify_labels.json`
    /// (already parsed). A name's community is the one whose id sorts first among the nodes it
    /// names, by id or label (§4.7).
    pub fn new(ix: &Index, analysis: Option<&Value>, labels: Option<&Value>) -> GraphContext {
        let mut names = HashSet::new();
        let mut by_id: HashMap<&str, Vec<&str>> = HashMap::new();
        for i in 0..ix.node_count() as u32 {
            let n = ix.node(i);
            names.insert(n.id.to_string());
            names.insert(n.label.to_string());
            by_id.entry(n.id).or_default().push(n.label);
        }
        let topics = analysis
            .and_then(|a| a.get("communities"))
            .and_then(Value::as_object)
            .filter(|c| !c.is_empty())
            .map(|communities| {
                let mut ids: Vec<&String> = communities.keys().collect();
                ids.sort();
                let mut topic_of: HashMap<String, String> = HashMap::new();
                for cid in ids {
                    let topic = labels
                        .and_then(|l| l.get(cid.as_str()))
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map_or_else(|| format!("Community {cid}"), str::to_string);
                    let members = communities[cid].as_array().into_iter().flatten();
                    for m in members.filter_map(Value::as_str) {
                        topic_of
                            .entry(m.to_string())
                            .or_insert_with(|| topic.clone());
                        for label in by_id.get(m).into_iter().flatten() {
                            topic_of
                                .entry(label.to_string())
                                .or_insert_with(|| topic.clone());
                        }
                    }
                }
                topic_of
            });
        GraphContext { names, topics }
    }
}

/// [`GraphContext::new`] from files: the graph (`graph.dij` or any `graph.json`), and optional
/// analysis and labels JSON. `None` when the graph cannot be read; an unreadable analysis or
/// labels file counts as absent.
pub fn load_context(
    graph: &Path,
    analysis: Option<&Path>,
    labels: Option<&Path>,
) -> Option<GraphContext> {
    let (ix, _) = crate::load(graph).ok()?;
    let read = |p: &Path| -> Option<Value> { serde_json::from_slice(&std::fs::read(p).ok()?).ok() };
    let analysis = analysis.and_then(read);
    let labels = labels.and_then(read);
    Some(GraphContext::new(&ix, analysis.as_ref(), labels.as_ref()))
}

/// Counts by outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub useful: usize,
    pub dead_ends: usize,
    pub corrected: usize,
    pub unmarked: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.useful + self.dead_ends + self.corrected + self.unmarked
    }
}

/// A name with its useful results.
#[derive(Debug, Clone, PartialEq)]
pub struct Useful {
    pub name: String,
    pub count: usize,
}

/// Which way a contested name leans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lean {
    Useful,
    DeadEnd,
    Even,
}

/// A name with useful and negative results.
#[derive(Debug, Clone, PartialEq)]
pub struct Contested {
    pub name: String,
    pub useful: usize,
    pub negative: usize,
    pub lean: Lean,
    /// The greatest raw date, cut to 10 characters; `None` when every date is empty.
    pub latest: Option<String>,
}

/// One dead end or correction: the latest result for a question.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub question: String,
    pub nodes: Vec<String>,
    /// Corrections only.
    pub correction: String,
}

/// The lessons of a set of results.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Section {
    pub preferred: Vec<Useful>,
    pub tentative: Vec<Useful>,
    pub contested: Vec<Contested>,
    pub dead_ends: Vec<Entry>,
    pub corrections: Vec<Entry>,
}

impl Section {
    pub fn is_empty(&self) -> bool {
        self.preferred.is_empty()
            && self.tentative.is_empty()
            && self.contested.is_empty()
            && self.dead_ends.is_empty()
            && self.corrections.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lessons {
    pub counts: Counts,
    pub all: Section,
    /// Topic name → its lessons, in output order (by name, `Uncategorized` last).
    pub topics: Vec<(String, Section)>,
}

/// Python's `round(x, 9)`.
fn round9(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.9}").parse().unwrap_or(x)
}

fn weight(r: &Saved, o: &Options) -> f64 {
    let hl = o.half_life_days;
    if !(hl.is_finite() && hl > 0.0) {
        return 1.0;
    }
    let Some(t) = clock::parse_iso(&r.date) else {
        return 1.0;
    };
    let age_days = ((o.now - t) / 86_400.0).max(0.0);
    0.5f64.powf(age_days / hl)
}

/// Fold `results` (already stripped of stale names) into lessons.
fn section(results: &[(&Saved, f64)], o: &Options) -> Section {
    #[derive(Default)]
    struct Acc {
        useful: usize,
        wu: f64,
        negative: usize,
        wn: f64,
        latest: Option<String>,
    }
    let mut acc: BTreeMap<&str, Acc> = BTreeMap::new();
    for &(r, w) in results {
        let Some(outcome) = r.outcome else { continue };
        for n in &r.nodes {
            let a = acc.entry(n.as_str()).or_default();
            if outcome == Outcome::Useful {
                a.useful += 1;
                a.wu += w;
            } else {
                a.negative += 1;
                a.wn += w;
            }
            if !r.date.is_empty() && a.latest.as_deref().is_none_or(|l| r.date.as_str() > l) {
                a.latest = Some(r.date.clone());
            }
        }
    }
    let mut sec = Section::default();
    let mut ranked: Vec<(f64, &str, &Acc)> = Vec::new();
    let mut contested: Vec<(f64, &str, &Acc)> = Vec::new();
    for (name, a) in &acc {
        if a.useful == 0 {
            continue;
        }
        if a.negative > 0 {
            contested.push((round9(a.wu - a.wn), name, a));
        } else {
            ranked.push((round9(a.wu), name, a));
        }
    }
    let by_score = |x: &(f64, &str, &Acc), y: &(f64, &str, &Acc)| {
        y.0.partial_cmp(&x.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.1.cmp(y.1))
    };
    ranked.sort_by(by_score);
    contested.sort_by(by_score);
    for (_, name, a) in ranked {
        let u = Useful {
            name: name.to_string(),
            count: a.useful,
        };
        if a.useful as i64 >= o.min_corroboration {
            sec.preferred.push(u);
        } else {
            sec.tentative.push(u);
        }
    }
    for (lean, name, a) in contested {
        sec.contested.push(Contested {
            name: name.to_string(),
            useful: a.useful,
            negative: a.negative,
            lean: if lean > 0.0 {
                Lean::Useful
            } else if lean < 0.0 {
                Lean::DeadEnd
            } else {
                Lean::Even
            },
            latest: a.latest.as_ref().map(|d| d.chars().take(10).collect()),
        });
    }
    sec.dead_ends = latest_per_question(results, Outcome::DeadEnd);
    sec.corrections = latest_per_question(results, Outcome::Corrected);
    sec
}

/// One entry per question: the greatest date string wins, then the file sorting last.
fn latest_per_question(results: &[(&Saved, f64)], outcome: Outcome) -> Vec<Entry> {
    let mut best: HashMap<&str, &Saved> = HashMap::new();
    for &(r, _) in results {
        if r.outcome != Some(outcome) {
            continue;
        }
        let slot = best.entry(r.question.as_str()).or_insert(r);
        if (r.date.as_str(), r.file.as_str()) > (slot.date.as_str(), slot.file.as_str()) {
            *slot = r;
        }
    }
    let mut kept: Vec<&Saved> = best.into_values().collect();
    kept.sort_by(|a, b| (&a.date, &a.question).cmp(&(&b.date, &b.question)));
    kept.into_iter()
        .map(|r| Entry {
            question: r.question.clone(),
            nodes: r.nodes.clone(),
            correction: r.correction.clone().unwrap_or_default(),
        })
        .collect()
}

/// The lessons of `results` (in file-name order, as [`crate::memory::load_dir`] returns them).
pub fn reflect(results: &[Saved], graph: Option<&GraphContext>, o: &Options) -> Lessons {
    let mut counts = Counts::default();
    let kept: Vec<Saved> = results
        .iter()
        .map(|r| {
            match r.outcome {
                Some(Outcome::Useful) => counts.useful += 1,
                Some(Outcome::DeadEnd) => counts.dead_ends += 1,
                Some(Outcome::Corrected) => counts.corrected += 1,
                None => counts.unmarked += 1,
            }
            let mut r = r.clone();
            if let Some(g) = graph {
                r.nodes.retain(|n| g.names.contains(n));
            }
            r
        })
        .collect();
    let weighted: Vec<(&Saved, f64)> = kept.iter().map(|r| (r, weight(r, o))).collect();
    let all = section(&weighted, o);
    let mut topics = Vec::new();
    if let Some(topic_of) = graph.and_then(|g| g.topics.as_ref()) {
        let mut groups: BTreeMap<String, Vec<(&Saved, f64)>> = BTreeMap::new();
        let mut uncategorized = Vec::new();
        for &(r, w) in &weighted {
            let mut votes: BTreeMap<&str, usize> = BTreeMap::new();
            for n in &r.nodes {
                if let Some(t) = topic_of.get(n) {
                    *votes.entry(t.as_str()).or_default() += 1;
                }
            }
            // Most votes; ties to the name sorting first (BTreeMap iterates in name order).
            let mut best: Option<(&str, usize)> = None;
            for (t, v) in votes {
                if best.is_none_or(|(_, bv)| v > bv) {
                    best = Some((t, v));
                }
            }
            match best {
                Some((t, _)) if t != UNCATEGORIZED => {
                    groups.entry(t.to_string()).or_default().push((r, w))
                }
                _ => uncategorized.push((r, w)),
            }
        }
        for (t, rs) in groups {
            topics.push((t, section(&rs, o)));
        }
        if !uncategorized.is_empty() {
            topics.push((UNCATEGORIZED.to_string(), section(&uncategorized, o)));
        }
    }
    Lessons {
        counts,
        all,
        topics,
    }
}

/// Inline code that survives backticks and line breaks in the name.
fn code(s: &str) -> String {
    let flat = flatten(s);
    let mut fence = String::from("`");
    while flat.contains(fence.as_str()) {
        fence.push('`');
    }
    if flat.starts_with('`') || flat.ends_with('`') || flat.is_empty() {
        format!("{fence} {flat} {fence}")
    } else {
        format!("{fence}{flat}{fence}")
    }
}

/// Line breaks shown as spaces, so one item stays on one line.
fn flatten(s: &str) -> String {
    s.replace("\r\n", " ")
        .replace(['\n', '\r', '\u{2028}', '\u{2029}'], " ")
}

fn render_section(sec: &Section, o: &Options, out: &mut String) {
    if sec.is_empty() {
        out.push_str("_Nothing marked useful, dead end or corrected here yet._\n\n");
        return;
    }
    let plural = |n: i64| if n == 1 { "" } else { "s" };
    if !sec.preferred.is_empty() {
        let _ = writeln!(
            out,
            "**Start here** — useful in at least {} result{}.\n",
            o.min_corroboration,
            plural(o.min_corroboration)
        );
        for u in &sec.preferred {
            let _ = writeln!(out, "- {} — useful in {}", code(&u.name), u.count);
        }
        out.push('\n');
    }
    if !sec.tentative.is_empty() {
        let _ = writeln!(
            out,
            "**Unconfirmed** — useful in fewer than {} result{}; check before trusting.\n",
            o.min_corroboration,
            plural(o.min_corroboration)
        );
        for u in &sec.tentative {
            let _ = writeln!(out, "- {} — useful in {}", code(&u.name), u.count);
        }
        out.push('\n');
    }
    if !sec.contested.is_empty() {
        out.push_str("**Mixed signals** — the more recent results weigh more.\n\n");
        for c in &sec.contested {
            let lean = match c.lean {
                Lean::Useful => "now mostly useful",
                Lean::DeadEnd => "now mostly a dead end",
                Lean::Even => "evenly balanced",
            };
            let _ = write!(
                out,
                "- {} — useful in {}, dead end or corrected in {}; {lean}",
                code(&c.name),
                c.useful,
                c.negative
            );
            if let Some(d) = &c.latest {
                let _ = write!(out, " (last seen {})", flatten(d));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    if !sec.dead_ends.is_empty() {
        out.push_str("**Dead ends** — these led nowhere; skip them unless the code changed.\n\n");
        for e in &sec.dead_ends {
            let _ = write!(out, "- {}", serde_json::Value::String(flatten(&e.question)));
            if !e.nodes.is_empty() {
                let names: Vec<String> = e.nodes.iter().map(|n| code(n)).collect();
                let _ = write!(out, " — {}", names.join(", "));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    if !sec.corrections.is_empty() {
        out.push_str("**Corrections** — what the right answer turned out to be.\n\n");
        for e in &sec.corrections {
            let _ = writeln!(
                out,
                "- {} → {}",
                serde_json::Value::String(flatten(&e.question)),
                flatten(&e.correction)
            );
        }
        out.push('\n');
    }
}

/// `LESSONS.md`, naming the memory directory it was made from; ends with one newline.
pub fn render(l: &Lessons, memory_dir: &Path, o: &Options) -> String {
    let mut out = String::from("# Lessons\n\n");
    let total = l.counts.total();
    let _ = writeln!(
        out,
        "_Written by `dijkstral reflect` from {total} saved result{} in `{}`, without a model. \
         A starting point, not a verdict: confirm before relying on it, and retry dead ends once \
         the code has moved on._\n",
        if total == 1 { "" } else { "s" },
        flatten(&memory_dir.display().to_string())
    );
    let c = l.counts;
    let _ = writeln!(
        out,
        "## Summary\n\n- {} useful · {} dead ends · {} corrected · {} unmarked\n",
        c.useful, c.dead_ends, c.corrected, c.unmarked
    );
    out.push_str("## Lessons\n\n");
    render_section(&l.all, o, &mut out);
    if !l.topics.is_empty() {
        out.push_str("## By community\n\n");
        for (name, sec) in &l.topics {
            let _ = writeln!(out, "### {}\n", flatten(name));
            render_section(sec, o, &mut out);
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(file: &str, date: &str, q: &str, o: Option<Outcome>, nodes: &[&str]) -> Saved {
        Saved {
            file: file.into(),
            date: date.into(),
            question: q.into(),
            outcome: o,
            correction: None,
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn opts(now: f64) -> Options {
        Options {
            half_life_days: 30.0,
            min_corroboration: 2,
            now,
        }
    }

    #[test]
    fn near_ties_round_to_nine_places() {
        // Weights within 5e-10 of each other are equal: the names fall back to code-point order.
        let now = clock::parse_iso("2026-10-15T16:00:00Z").unwrap();
        let rs = [
            saved(
                "a.md",
                "2026-01-01T00:00:00+00:00",
                "q",
                Some(Outcome::Useful),
                &["B"],
            ),
            saved(
                "b.md",
                "2026-01-01T00:00:00.030000+00:00",
                "q",
                Some(Outcome::Useful),
                &["A"],
            ),
            saved(
                "c.md",
                "2026-01-01T00:00:00+00:00",
                "q",
                Some(Outcome::Useful),
                &["C"],
            ),
        ];
        let o = Options {
            half_life_days: 1000.0,
            ..opts(now)
        };
        let l = reflect(&rs, None, &o);
        let names: Vec<&str> = l.all.tentative.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C"]);
        // A split within the tolerance is even; beyond it, the newer side wins.
        let rs = [
            saved(
                "a.md",
                "2026-01-01T00:00:00+00:00",
                "q",
                Some(Outcome::Useful),
                &["S"],
            ),
            saved(
                "b.md",
                "2026-01-01T00:00:00.030000+00:00",
                "d",
                Some(Outcome::DeadEnd),
                &["S"],
            ),
            saved(
                "c.md",
                "2026-01-01T00:00:00+00:00",
                "q",
                Some(Outcome::Useful),
                &["T"],
            ),
            saved(
                "d.md",
                "2026-01-01T00:00:01+00:00",
                "d",
                Some(Outcome::DeadEnd),
                &["T"],
            ),
        ];
        let l = reflect(&rs, None, &o);
        let leans: Vec<(&str, Lean)> = l
            .all
            .contested
            .iter()
            .map(|c| (c.name.as_str(), c.lean))
            .collect();
        assert_eq!(leans, [("S", Lean::Even), ("T", Lean::DeadEnd)]);
    }

    #[test]
    fn decay_can_be_disabled() {
        let rs = [
            saved(
                "a.md",
                "2020-01-01",
                "q",
                Some(Outcome::Useful),
                &["Old", "Old2"],
            ),
            saved("b.md", "2020-01-02", "q", Some(Outcome::Useful), &["Old"]),
            saved("c.md", "2026-01-01", "q", Some(Outcome::Useful), &["New"]),
        ];
        for hl in [0.0, -5.0, f64::INFINITY, f64::NAN] {
            let o = Options {
                half_life_days: hl,
                min_corroboration: 1,
                now: 1.9e9,
            };
            let l = reflect(&rs, None, &o);
            let names: Vec<&str> = l.all.preferred.iter().map(|u| u.name.as_str()).collect();
            assert_eq!(names, ["Old", "New", "Old2"], "half-life {hl}");
        }
    }

    #[test]
    fn markdown_stays_intact() {
        assert_eq!(code("a`b"), "``a`b``");
        assert_eq!(code("`x"), "`` `x ``");
        assert_eq!(code("e\nf"), "`e f`");
        assert_eq!(code(""), "`  `");
    }
}
