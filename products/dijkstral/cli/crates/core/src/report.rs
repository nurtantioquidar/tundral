//! `GRAPH_REPORT.md`: the no-model report Graphify writes on `update` (hub-named communities, corpus
//! statistics), computed over the extracted graph. Section order, wording and number formats follow
//! `docs/graphify-format.md` §17.5. Where the report tells the reader what to run next it names
//! `dijkstral` instead of `graphify`.

use std::fmt::Write;

use crate::analyze::{
    betweenness, import_cycles, is_hub_candidate, isolated, listed, py_int_set_order, Analysis,
};
use crate::cluster::{Communities, Topology};
use crate::facts::Confidence;
use crate::graph::Graph;
use crate::pyjson::float_repr;

const CYCLE_LIMIT: usize = 20;
const LISTED_SHOWN: usize = 8;
const THIN: usize = 3;
const QUESTION_LIMIT: usize = 7;
const LOW_COHESION: f64 = 0.15;
/// What the report and the `no_signal` question say when no question qualifies.
const NO_SIGNAL: &str = "Not enough signal to generate questions. This usually means the corpus has no AMBIGUOUS edges, no bridge nodes, no INFERRED relationships, and all communities are tightly cohesive. Add more files or run with --mode deep to extract richer edges.";

/// What the corpus check reports about the indexed files.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    /// Name of the indexed directory.
    pub name: String,
    /// `(files, whitespace-separated words)` over the indexed files; `None` when the graph was
    /// loaded rather than built (Graphify's `cluster-only` mode).
    pub stats: Option<(usize, u64)>,
    /// Walked files neither indexed nor documents or media `update` would index, sorted
    /// (`discover::Discovery::unclassified`).
    pub unclassified: Vec<String>,
    pub commit: Option<String>,
    /// `YYYY-MM-DD`.
    pub date: String,
}

/// One suggested question: its kind (`bridge_node`, `verify_inferred`, `isolated_nodes`,
/// `low_cohesion`), the question and the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub kind: &'static str,
    pub question: String,
    pub why: String,
}

pub fn render(
    graph: &Graph,
    topo: &Topology,
    communities: &Communities,
    analysis: &Analysis,
    corpus: &Corpus,
) -> String {
    let adj = &analysis.adjacency;
    let node = |i: u32| &graph.nodes[i as usize];
    let label = |c: usize| analysis.labels[c].as_str();

    // Listed members per community, in id order.
    let listed_members: Vec<Vec<u32>> = communities
        .members
        .iter()
        .map(|m| {
            m.iter()
                .copied()
                .filter(|&i| listed(node(i), adj.degree[i as usize]))
                .collect()
        })
        .collect();
    let thin = listed_members.iter().filter(|m| m.len() < THIN).count();
    let total = communities.members.len();

    let mut s = String::new();
    let _ = writeln!(s, "# Graph Report - {}  ({})", corpus.name, corpus.date);

    s.push_str("\n## Corpus Check\n");
    match corpus.stats {
        Some((files, words)) => {
            let _ = writeln!(s, "- {files} files · ~{} words", thousands(words));
            s.push_str("- Verdict: corpus is large enough that graph structure adds value.\n");
            if !corpus.unclassified.is_empty() {
                let _ = writeln!(
                    s,
                    "- Unclassified: {} file(s) not represented in the graph (top: {})",
                    corpus.unclassified.len(),
                    top_suffixes(&corpus.unclassified, 3)
                );
            }
        }
        None => s.push_str("- cluster-only mode — file stats not available\n"),
    }

    s.push_str("\n## Summary\n");
    let _ = write!(
        s,
        "- {} nodes · {} edges · {} communities",
        graph.nodes.len(),
        graph.edges.len(),
        total
    );
    if thin > 0 {
        let _ = write!(s, " ({} shown, {thin} thin omitted)", total - thin);
    }
    s.push('\n');
    s.push_str(&extraction_line(graph));
    s.push_str("- Token cost: 0 input · 0 output\n");

    if let Some(commit) = corpus.commit.as_deref().filter(|c| !c.is_empty()) {
        s.push_str("\n## Graph Freshness\n");
        let _ = writeln!(
            s,
            "- Built from commit: `{}`",
            &commit[..commit.len().min(8)]
        );
        s.push_str("- Run `git rev-parse HEAD` and compare to check if the graph is stale.\n");
        s.push_str("- Run `dijkstral update .` after code changes (no API cost).\n");
    }

    s.push_str("\n## Community Hubs (Navigation)\n");
    for (c, m) in listed_members.iter().enumerate() {
        if !m.is_empty() {
            let _ = writeln!(s, "- {}", label(c));
        }
    }

    s.push_str("\n## God Nodes (most connected - your core abstractions)\n");
    for (rank, &g) in analysis.gods.iter().enumerate() {
        let _ = writeln!(
            s,
            "{}. `{}` - {} edges",
            rank + 1,
            node(g).label,
            adj.degree[g as usize]
        );
    }

    s.push_str("\n## Surprising Connections (you probably didn't know these)\n");
    if analysis.surprises.is_empty() {
        s.push_str("- None detected - all connections are within the same source files.\n");
    }
    for sp in &analysis.surprises {
        let e = &graph.edges[sp.edge as usize];
        let (a, b, _) = topo.edges[sp.edge as usize];
        let (u, v) = (node(a), node(b));
        let note = sp
            .note
            .as_ref()
            .map_or_else(String::new, |n| format!("  _{n}_"));
        let _ = writeln!(
            s,
            "- `{}` --{}--> `{}`  [{}]\n  {} → {}{note}",
            u.label,
            e.relation,
            v.label,
            e.confidence.as_str(),
            u.source_file,
            v.source_file
        );
    }

    s.push_str("\n## Import Cycles\n");
    let cycles = import_cycles(graph, topo, adj, CYCLE_LIMIT);
    if cycles.is_empty() {
        s.push_str("- None detected.\n");
    }
    for c in &cycles {
        let _ = writeln!(
            s,
            "- {}-file cycle: `{} -> {}`",
            c.len(),
            c.join(" -> "),
            c[0]
        );
    }

    let _ = writeln!(s, "\n## Communities ({total} total, {thin} thin omitted)");
    for (c, m) in listed_members.iter().enumerate() {
        if m.len() < THIN {
            continue;
        }
        let names: Vec<&str> = m
            .iter()
            .take(LISTED_SHOWN)
            .map(|&i| node(i).label.as_str())
            .collect();
        let more = if m.len() > LISTED_SHOWN {
            format!(" (+{} more)", m.len() - LISTED_SHOWN)
        } else {
            String::new()
        };
        let _ = writeln!(
            s,
            "\n### Community {c} - \"{}\"\nCohesion: {:.2}\nNodes ({}): {}{}",
            label(c),
            analysis.cohesion[c],
            m.len(),
            names.join(", "),
            more
        );
    }

    let lonely = isolated(graph, adj);
    if !lonely.is_empty() || thin > 0 {
        s.push_str("\n## Knowledge Gaps\n");
    }
    if !lonely.is_empty() {
        let shown: Vec<String> = lonely
            .iter()
            .take(5)
            .map(|&i| format!("`{}`", node(i).label))
            .collect();
        let more = if lonely.len() > 5 {
            format!(" (+{} more)", lonely.len() - 5)
        } else {
            String::new()
        };
        let weak_total = adj.degree.iter().filter(|&&d| d <= 1).count();
        let docs = graph.nodes.iter().any(|n| n.file_type == "document");
        let _ = writeln!(
            s,
            "- **{} isolated node(s):** {}{}\n  These have ≤1 connection - possible missing edges{}. (Counts symbols only; {} node(s) total have ≤1 connection when file, concept and rationale nodes are included.)",
            lonely.len(),
            shown.join(", "),
            more,
            if docs { " or undocumented components" } else { "" },
            weak_total
        );
    }
    if thin > 0 {
        let _ = writeln!(
            s,
            "- **{thin} thin communities (<{THIN} nodes) omitted from report** — run `dijkstral query` to explore isolated nodes."
        );
    }

    s.push_str("\n## Suggested Questions\n");
    let questions = questions(graph, communities, analysis);
    if questions.is_empty() {
        let _ = write!(s, "_{NO_SIGNAL}_");
    } else {
        s.push_str("_Questions this graph is uniquely positioned to answer:_\n");
        for q in &questions {
            let _ = write!(s, "\n- **{}**\n  _{}_", q.question, q.why);
        }
    }
    s
}

/// The `n` most common `PurePath.suffix`es (`(none)` for none) of `paths`, as `ext count`; ties
/// in path order (`Counter.most_common` over the sorted walk).
fn top_suffixes(paths: &[String], n: usize) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for p in paths {
        let name = p.rsplit('/').next().unwrap_or(p);
        let suffix = match name.rfind('.') {
            Some(i) if i > 0 && i < name.len() - 1 => &name[i..],
            _ => "(none)",
        };
        match counts.iter_mut().find(|(s, _)| s == suffix) {
            Some(c) => c.1 += 1,
            None => counts.push((suffix.to_string(), 1)),
        }
    }
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    counts
        .iter()
        .take(n)
        .map(|(s, c)| format!("{s} {c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn extraction_line(graph: &Graph) -> String {
    let total = graph.edges.len().max(1) as f64;
    let count = |c: Confidence| graph.edges.iter().filter(|e| e.confidence == c).count();
    let (ex, inf, amb) = (
        count(Confidence::Extracted),
        count(Confidence::Inferred),
        count(Confidence::Ambiguous),
    );
    let pct = |n: usize| round_half_even(n as f64 * 100.0 / total);
    let mut line = format!(
        "- Extraction: {}% EXTRACTED · {}% INFERRED · {}% AMBIGUOUS",
        pct(ex),
        pct(inf),
        pct(amb)
    );
    if inf > 0 {
        let sum: f64 = graph
            .edges
            .iter()
            .filter(|e| e.confidence == Confidence::Inferred)
            // A link without a score counts 0.5 here, although `graph.json` writes it as 0.55
            // (§2.5): only a reloaded graph (`cluster-only`) averages the written 0.55.
            .map(|e| e.score.unwrap_or(0.5))
            .sum();
        let _ = write!(
            line,
            " · INFERRED: {inf} edges (avg confidence: {})",
            round2_repr(sum / inf as f64)
        );
    }
    line.push('\n');
    line
}

/// `repr(round(x, 2))`: two decimals at most, trailing zeros dropped but one (`0.5`, `1.0`).
fn round2_repr(x: f64) -> String {
    let s = format!("{x:.2}");
    let t = s.trim_end_matches('0');
    if t.ends_with('.') {
        format!("{t}0")
    } else {
        t.to_string()
    }
}

/// Python's `round()` to an integer (banker's rounding).
fn round_half_even(x: f64) -> i64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r as i64 % 2 != 0 {
        (r - x.signum()) as i64
    } else {
        r as i64
    }
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Suggested questions: bridge nodes, INFERRED-heavy god nodes, isolated nodes and loosely
/// connected communities, interleaved one of each kind per round, at most seven.
pub fn questions(graph: &Graph, communities: &Communities, analysis: &Analysis) -> Vec<Question> {
    let adj = &analysis.adjacency;
    let lonely = isolated(graph, adj);
    let node = |i: u32| &graph.nodes[i as usize];
    let comm = |i: u32| communities.of_node[i as usize];

    // Bridges: of the three hub candidates with the highest betweenness, those touching other
    // communities.
    let bc = betweenness(adj);
    let mut ranked: Vec<u32> = (0..graph.nodes.len() as u32)
        .filter(|&i| is_hub_candidate(node(i), adj.degree[i as usize]))
        .collect();
    ranked.sort_by(|&a, &b| bc[b as usize].total_cmp(&bc[a as usize]));
    let mut bridges = Vec::new();
    for &x in ranked.iter().take(3) {
        let own = comm(x);
        let others = py_int_set_order(
            adj.neighbors[x as usize]
                .iter()
                .map(|&(v, _)| comm(v))
                .filter(|&c| c != own),
        );
        if others.is_empty() {
            continue;
        }
        let to: Vec<String> = others
            .iter()
            .map(|&c| format!("`{}`", analysis.labels[c as usize]))
            .collect();
        bridges.push(q(
            "bridge_node",
            format!(
                "Why does `{}` connect `{}` to {}?",
                node(x).label,
                analysis.labels[own as usize],
                to.join(", ")
            ),
            format!(
                "High betweenness centrality ({:.3}) - this node is a cross-community bridge.",
                bc[x as usize]
            ),
        ));
        if bridges.len() == 3 {
            break;
        }
    }

    // God nodes among the top five with at least two INFERRED links.
    let mut verify = Vec::new();
    for &g in analysis.gods.iter().take(5) {
        let mut inferred: Vec<u32> = adj.neighbors[g as usize]
            .iter()
            .filter(|&&(_, e)| graph.edges[e as usize].confidence == Confidence::Inferred)
            .map(|&(v, _)| v)
            .collect();
        if inferred.len() < 2 {
            continue;
        }
        let n = inferred.len();
        inferred.sort_by(|&a, &b| node(a).id.cmp(&node(b).id));
        let label = &node(g).label;
        verify.push(q("verify_inferred",
            format!(
                "Are the {n} inferred relationships involving `{label}` (e.g. with `{}` and `{}`) actually correct?",
                node(inferred[0]).label,
                node(inferred[1]).label
            ),
            format!("`{label}` has {n} INFERRED edges - model-reasoned connections that need verification."),
        ));
    }

    let mut lonely_q = Vec::new();
    if !lonely.is_empty() {
        let names: Vec<String> = lonely
            .iter()
            .take(3)
            .map(|&i| format!("`{}`", node(i).label))
            .collect();
        lonely_q.push(q(
            "isolated_nodes",
            format!(
                "What connects {} to the rest of the system?",
                names.join(", ")
            ),
            format!(
                "{} weakly-connected nodes found - possible documentation gaps or missing edges.",
                lonely.len()
            ),
        ));
    }

    let mut loose = Vec::new();
    for (c, &cohesion) in analysis.cohesion.iter().enumerate() {
        if cohesion < LOW_COHESION {
            loose.push(q(
                "low_cohesion",
                format!(
                    "Should `{}` be split into smaller, more focused modules?",
                    analysis.labels[c]
                ),
                format!(
                    "Cohesion score {} - nodes in this community are weakly interconnected.",
                    float_repr(cohesion)
                ),
            ));
        }
    }

    let lists = [bridges, verify, lonely_q, loose];
    let rounds = lists.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::new();
    for r in 0..rounds {
        for l in &lists {
            if let Some(item) = l.get(r) {
                out.push(item.clone());
            }
        }
    }
    out.truncate(QUESTION_LIMIT);
    out
}

fn q(kind: &'static str, question: String, why: String) -> Question {
    Question {
        kind,
        question,
        why,
    }
}

/// `.graphify_analysis.json` as `graphify cluster-only` writes it (questions instead of tokens).
pub fn cluster_only_analysis_json(
    graph: &Graph,
    communities: &Communities,
    analysis: &Analysis,
) -> String {
    let qs = questions(graph, communities, analysis);
    let listed = if qs.is_empty() {
        vec![("no_signal", None, NO_SIGNAL)]
    } else {
        qs.iter()
            .map(|q| (q.kind, Some(q.question.as_str()), q.why.as_str()))
            .collect()
    };
    analysis.to_json_with(graph, communities, Some(listed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats() {
        assert_eq!(thousands(245858), "245,858");
        assert_eq!(thousands(454), "454");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(round_half_even(91.5887), 92);
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(3.5), 4);
        assert_eq!(round2_repr(0.5), "0.5");
        assert_eq!(round2_repr(0.5349999999999999), "0.53");
        assert_eq!(round2_repr(0.9055555555555556), "0.91");
        assert_eq!(round2_repr(1.0), "1.0");
    }

    #[test]
    fn unclassified_suffixes_by_count_then_path_order() {
        let paths: Vec<String> = [
            "LICENSE",
            "a/z1.zzz",
            "b.sbt",
            "b/a1.aaa",
            "c.sbt",
            ".gitignore",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(top_suffixes(&paths, 3), "(none) 2, .sbt 2, .zzz 1");
    }
}
