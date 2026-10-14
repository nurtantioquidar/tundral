//! `benchmark`: Graphify's token-reduction estimate (`docs/graphify-query.md` §9).

use std::collections::HashSet;

use dijkstral_core::index::Index;

use crate::text::{py_len, thousands};

/// The fixed sample questions, in output order.
pub const QUESTIONS: &[&str] = &[
    "how does authentication work",
    "what is the main entry point",
    "how are errors handled",
    "what connects the data layer to the api",
    "what are the core abstractions",
];

/// Words assumed per node when only a graph is given.
const WORDS_PER_NODE: i64 = 50;
const DEPTH: usize = 3;
const SEEDS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Benchmark {
    pub words: i64,
    pub naive: i64,
    pub nodes: usize,
    pub edges: usize,
    /// `(question, cost in tokens)` for every question that matched.
    pub per_question: Vec<(&'static str, i64)>,
}

/// Seeds for a benchmark question (fitted by probes; not `query`'s seed model): the question's
/// `query` terms, then the three nodes whose lower-cased label contains the most terms, ties
/// broken by id, descending. Match tier, source files and degree play no part (750 of 750
/// probed questions on random graphs).
pub fn seeds(ix: &Index, question: &str) -> Vec<u32> {
    let terms = crate::query::terms(question);
    let mut scored: Vec<(usize, u32)> = (0..ix.node_count() as u32)
        .filter_map(|i| {
            let l = ix.lower_label(i);
            let s = terms.iter().filter(|t| l.contains(t.as_str())).count();
            (s > 0).then_some((s, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| ix.id(b.1).cmp(ix.id(a.1))));
    scored.into_iter().take(SEEDS).map(|(_, i)| i).collect()
}

/// The token cost of answering from `seeds`: a breadth-first expansion to depth 3, one line per
/// reached node and per link into a newly reached node, a token being four characters.
pub fn cost(ix: &Index, seeds: &[u32]) -> i64 {
    let mut seen: HashSet<u32> = seeds.iter().copied().collect();
    let mut order: Vec<u32> = Vec::new();
    for &s in seeds {
        if !order.contains(&s) {
            order.push(s);
        }
    }
    let mut frontier = order.clone();
    let mut edges: Vec<u32> = Vec::new();
    for _ in 0..DEPTH {
        let layer: HashSet<u32> = frontier.iter().copied().collect();
        let mut next = Vec::new();
        let mut next_set: HashSet<u32> = HashSet::new();
        for &u in &frontier {
            for (e, v) in ix.adjacent(u) {
                if layer.contains(&v) || (seen.contains(&v) && !next_set.contains(&v)) {
                    continue;
                }
                edges.push(e);
                if next_set.insert(v) {
                    next.push(v);
                }
            }
        }
        seen.extend(next.iter().copied());
        order.extend(next.iter().copied());
        frontier = next;
    }
    let mut lines: Vec<String> = order
        .iter()
        .map(|&i| {
            let n = ix.node(i);
            format!(
                "NODE {} src={} loc={}",
                n.label, n.source_file, n.source_location
            )
        })
        .collect();
    for e in edges {
        let e = ix.edge(e);
        lines.push(format!(
            "EDGE {} --{}--> {}",
            ix.label(e.source),
            e.relation,
            ix.label(e.target)
        ));
    }
    let chars: usize =
        lines.iter().map(|l| py_len(l)).sum::<usize>() + lines.len().saturating_sub(1);
    (chars / 4) as i64
}

pub fn benchmark(ix: &Index) -> Benchmark {
    let words = WORDS_PER_NODE * ix.node_count() as i64;
    let naive = words * 4 / 3;
    let per_question = QUESTIONS
        .iter()
        .filter_map(|q| {
            let s = seeds(ix, q);
            (!s.is_empty()).then(|| (*q, cost(ix, &s)))
        })
        .collect();
    Benchmark {
        words,
        naive,
        nodes: ix.node_count(),
        edges: ix.edge_count(),
        per_question,
    }
}

pub const NO_MATCH: &str =
    "Benchmark error: No matching nodes found for sample questions. Build the graph first.";

/// Graphify's benchmark report, including its leading and trailing blank lines (no final
/// newline).
pub fn render(b: &Benchmark) -> String {
    if b.per_question.is_empty() {
        return NO_MATCH.to_string();
    }
    // The average is floored before it is printed and divided into the naive cost (fitted on
    // random graphs: a 318.6 average prints ~318 and its ratio uses 318).
    let avg =
        (b.per_question.iter().map(|(_, c)| *c).sum::<i64>() / b.per_question.len() as i64) as f64;
    let ratio = |cost: f64| {
        if cost > 0.0 {
            b.naive as f64 / cost
        } else {
            0.0
        }
    };
    let mut out = vec![
        String::new(),
        "graphify token reduction benchmark".to_string(),
        "\u{2500}".repeat(50),
        format!(
            "  Corpus:          {} words \u{2192} ~{} tokens (naive)",
            thousands(b.words),
            thousands(b.naive)
        ),
        format!(
            "  Graph:           {} nodes, {} edges",
            thousands(b.nodes as i64),
            thousands(b.edges as i64)
        ),
        format!("  Avg query cost:  ~{} tokens", thousands(avg as i64)),
        format!(
            "  Reduction:       {:.1}x fewer tokens per query",
            ratio(avg)
        ),
        String::new(),
        "  Per question:".to_string(),
    ];
    for (q, c) in &b.per_question {
        out.push(format!("    [{:.1}x] {q}", ratio(*c as f64)));
    }
    out.push(String::new());
    out.join("\n")
}
