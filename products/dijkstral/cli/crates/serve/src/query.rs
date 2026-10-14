//! `query`: question → terms → scored seeds → depth-limited traversal → budgeted text
//! (`docs/graphify-query.md` §3).

use std::collections::{HashMap, HashSet};

use dijkstral_core::index::Index;
use memchr::memmem::Finder;

use crate::text::{is_alnum, py_len, py_repr_list};

/// Stopwords of three or more characters (§3.3); shorter tokens are dropped by length.
const STOPWORDS: &[&str] = &[
    "about", "all", "and", "any", "are", "been", "being", "but", "can", "could", "did", "does",
    "for", "from", "had", "has", "have", "here", "how", "into", "its", "may", "might", "must",
    "not", "off", "onto", "shall", "should", "some", "that", "the", "their", "them", "there",
    "these", "they", "this", "those", "was", "were", "what", "when", "where", "which", "who",
    "whom", "whose", "why", "will", "with", "without", "work", "working", "works", "would",
];

/// Terms that get no coverage seed unless every term is one (§3.6 step 4).
const RELATION_WORDS: &[&str] = &[
    "call",
    "calls",
    "called",
    "caller",
    "callers",
    "invoke",
    "invokes",
    "invoked",
    "use",
    "uses",
    "used",
    "using",
    "reference",
    "references",
    "referenced",
    "depend",
    "depends",
    "import",
    "imports",
    "imported",
    "export",
    "exports",
    "exported",
    "extend",
    "extends",
    "extended",
    "implement",
    "implements",
    "implemented",
];

/// The context heuristic (§3.4), in header order.
const CONTEXT_TRIGGERS: &[(&str, &[&str])] = &[
    (
        "call",
        &[
            "call", "calls", "called", "caller", "callers", "invoke", "invokes", "invoked",
        ],
    ),
    (
        "import",
        &["import", "imports", "imported", "module", "modules"],
    ),
    ("field", &["field", "fields", "member", "members"]),
    (
        "parameter_type",
        &[
            "parameter",
            "parameters",
            "param",
            "params",
            "argument",
            "arguments",
        ],
    ),
    ("return_type", &["return", "returns", "returned"]),
    ("generic_arg", &["generic", "generics"]),
];

/// Weight of a phrase-exact (or one-term id-exact) label, times the largest idf (§3.5; fitted).
const PHRASE_EXACT: f64 = 10.0;
/// Primary seeds score at least this share of the top score (§3.6).
const SEED_RATIO: f64 = 0.2;
const MAX_PRIMARY_SEEDS: usize = 3;
/// Hubs: degree at or above max(this, P99 of degrees) are reached but not expanded (§3.7).
const MIN_HUB_DEGREE: f64 = 50.0;

/// The question's search terms (§3.3).
pub fn terms(question: &str) -> Vec<String> {
    let lower = question.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        .filter(|t| !t.is_empty())
        .collect();
    let pass = |stop: bool| {
        let mut out: Vec<String> = Vec::new();
        for tok in &tokens {
            if tok.len() < 3 && !tok.bytes().any(|b| b.is_ascii_digit()) {
                continue;
            }
            if stop && STOPWORDS.contains(tok) {
                continue;
            }
            for part in tok.split('_').filter(|p| !p.is_empty()) {
                if !out.iter().any(|t| t == part) {
                    out.push(part.to_string());
                }
            }
        }
        out
    };
    let t = pass(true);
    if t.is_empty() {
        pass(false)
    } else {
        t
    }
}

/// An edge-context filter and how it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFilter {
    pub values: Vec<String>,
    pub explicit: bool,
}

/// The heuristic context filter for a question, if any trigger word occurs (§3.4).
pub fn heuristic_context(question: &str) -> Option<ContextFilter> {
    let words: HashSet<String> = question
        .split_whitespace()
        .map(|w| w.to_lowercase().trim_end_matches([',', '?']).to_string())
        .collect();
    let values: Vec<String> = CONTEXT_TRIGGERS
        .iter()
        .filter(|(_, triggers)| triggers.iter().any(|t| words.contains(*t)))
        .map(|(c, _)| c.to_string())
        .collect();
    (!values.is_empty()).then_some(ContextFilter {
        values,
        explicit: false,
    })
}

/// An explicit filter from `--context` values: first-seen order, duplicates removed.
pub fn explicit_context<S: AsRef<str>>(values: &[S]) -> Option<ContextFilter> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        if !out.iter().any(|x| x == v.as_ref()) {
            out.push(v.as_ref().to_string());
        }
    }
    (!out.is_empty()).then_some(ContextFilter {
        values: out,
        explicit: true,
    })
}

#[derive(Debug, Clone)]
pub struct QueryOptions {
    pub dfs: bool,
    /// Traversal depth (2 on the CLI).
    pub depth: u32,
    /// `Some` for `--context`; `None` applies the heuristic.
    pub context: Option<Vec<String>>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        QueryOptions {
            dfs: false,
            depth: 2,
            context: None,
        }
    }
}

/// A query answer before rendering.
#[derive(Debug, Clone)]
pub struct QueryAnswer {
    pub terms: Vec<String>,
    /// Seeds in `Start:` order.
    pub seeds: Vec<u32>,
    /// Every reached node in output order: seeds first, then by distance, filtered degree, id.
    pub nodes: Vec<u32>,
    /// Reached nodes in traversal visit order (the order `EDGE` lines are collected in).
    pub visit: Vec<u32>,
    pub context: Option<ContextFilter>,
    pub dfs: bool,
    pub depth: u32,
}

/// A link filter: everything, or only links whose `context` is in the set.
struct LinkFilter<'a> {
    ix: &'a Index,
    allowed: Option<HashSet<&'a str>>,
    memo: std::cell::RefCell<HashMap<u32, bool>>,
}

impl<'a> LinkFilter<'a> {
    fn new(ix: &'a Index, ctx: Option<&'a ContextFilter>) -> Self {
        LinkFilter {
            ix,
            allowed: ctx.map(|c| c.values.iter().map(String::as_str).collect()),
            memo: Default::default(),
        }
    }
    fn active(&self) -> bool {
        self.allowed.is_some()
    }
    fn passes(&self, e: u32) -> bool {
        let Some(allowed) = &self.allowed else {
            return true;
        };
        let Some(key) = self.ix.context_key(e) else {
            return false;
        };
        *self
            .memo
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| allowed.contains(self.ix.edge(e).context.unwrap_or_default()))
    }
}

/// `ln(1 + N / (1 + df))`.
fn idf(n: usize, df: usize) -> f64 {
    (1.0 + n as f64 / (1.0 + df as f64)).ln()
}

/// Nodes whose lower-cased label contains `t`, ascending.
fn label_hits(ix: &Index, t: &str) -> Vec<u32> {
    let finder = Finder::new(t.as_bytes());
    let hit = |i: u32| finder.find(ix.lower_label(i).as_bytes()).is_some();
    match ix.substring_candidates(t) {
        // norm_label (NFKD, marks stripped, lower-cased) keeps every ASCII run of label.lower(),
        // so its trigram candidates are a superset for the ASCII terms questions produce.
        Some(c) if t.is_ascii() => c.into_iter().filter(|&i| hit(i)).collect(),
        _ => (0..ix.node_count() as u32).filter(|&i| hit(i)).collect(),
    }
}

/// For each term, the nodes whose lower-cased `source_file` contains it, ascending.
fn file_hits(ix: &Index, terms: &[String]) -> Vec<Vec<u32>> {
    let finders: Vec<Finder<'_>> = terms.iter().map(|t| Finder::new(t.as_bytes())).collect();
    let mut memo: HashMap<u32, Vec<bool>> = HashMap::new();
    let mut out = vec![Vec::new(); terms.len()];
    for i in 0..ix.node_count() as u32 {
        let key = ix.lower_file_key(i);
        let flags = memo.entry(key).or_insert_with(|| {
            let f = ix.lower_file(i).as_bytes();
            finders.iter().map(|fd| fd.find(f).is_some()).collect()
        });
        for (k, &yes) in flags.iter().enumerate() {
            if yes {
                out[k].push(i);
            }
        }
    }
    out
}

/// `[a-z0-9]+` runs of `s` joined by single spaces.
fn words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

/// Score candidates (§3.5). Returns `(node, score)` for every node with a positive score.
fn score(ix: &Index, terms: &[String], hits: &[Vec<u32>], files: &[Vec<u32>]) -> Vec<(u32, f64)> {
    let n = ix.node_count();
    let idfs: Vec<f64> = hits.iter().map(|h| idf(n, h.len())).collect();
    let max_idf = idfs.iter().copied().fold(f64::MIN, f64::max);
    let phrase = terms.join(" ");
    let nt = terms.len() as f64;

    let mut cands: Vec<u32> = hits.iter().chain(files).flatten().copied().collect();
    if terms.len() == 1 {
        if let Some(i) = ix.by_id(&terms[0]) {
            cands.push(i);
        }
    }
    cands.sort_unstable();
    cands.dedup();

    let file_sets: Vec<HashSet<u32>> = files.iter().map(|f| f.iter().copied().collect()).collect();
    let mut out = Vec::with_capacity(cands.len());
    for i in cands {
        let l = ix.lower_label(i);
        let core = l.strip_suffix("()").unwrap_or(l);
        let (mut m, mut strong, mut weak) = (0usize, 0.0f64, 0.0f64);
        for (k, t) in terms.iter().enumerate() {
            if core == t {
                m += 1;
                strong += idfs[k];
            } else if l.starts_with(t.as_str()) {
                m += 1;
                strong += 0.1 * idfs[k];
            } else if hits[k].binary_search(&i).is_ok() {
                m += 1;
                weak += 0.001 * idfs[k];
            }
            if file_sets[k].contains(&i) {
                weak += 0.0005 * idfs[k];
            }
        }
        let mut s = (m as f64 / nt).powi(2) * strong + weak;
        let id_exact = terms.len() == 1 && ix.id(i) == terms[0];
        if m == terms.len() || id_exact {
            let w = words(l);
            if w == phrase || id_exact {
                s += PHRASE_EXACT * max_idf;
            } else if w.starts_with(&phrase) {
                s += max_idf;
            }
        }
        if s > 0.0 {
            out.push((i, s));
        }
    }
    out
}

/// `l` without trailing non-alphanumerics.
fn trim_end_non_alnum(s: &str) -> &str {
    s.trim_end_matches(|c: char| !is_alnum(c))
}

/// `l` without any `()`, then without leading and trailing non-alphanumerics.
fn norm4(s: &str) -> String {
    s.replace("()", "")
        .trim_matches(|c: char| !is_alnum(c))
        .to_string()
}

/// The best node for one term's coverage seed (§3.6 step 4).
fn coverage_seed(ix: &Index, t: &str, hits: &[u32], files: &[u32]) -> Option<u32> {
    let id_node = ix.by_id(t);
    // Pools 1–7 hold labels that contain `t` (or the id match), pool 8 source-file matches.
    let mut tiered: Vec<(u8, u32)> = Vec::new();
    let mut label_nodes: Vec<u32> = hits.to_vec();
    if let Some(i) = id_node {
        if label_nodes.binary_search(&i).is_err() {
            label_nodes.push(i);
            label_nodes.sort_unstable();
        }
    }
    for &i in &label_nodes {
        let l = ix.lower_label(i);
        let core = l.strip_suffix("()").unwrap_or(l);
        let n4 = norm4(l);
        let tier = if core == t {
            1
        } else if id_node == Some(i) {
            2
        } else if trim_end_non_alnum(l) == t {
            3
        } else if n4 == t {
            4
        } else if l.starts_with(t) {
            5
        } else if n4.starts_with(t) {
            6
        } else if l.contains(t) {
            7
        } else {
            continue;
        };
        tiered.push((tier, i));
    }
    if tiered.is_empty() {
        tiered.extend(files.iter().map(|&i| (8, i)));
    }
    let best_tier = tiered.iter().map(|t| t.0).min()?;
    let pool: Vec<u32> = tiered
        .into_iter()
        .filter(|t| t.0 == best_tier)
        .map(|t| t.1)
        .collect();
    let file_set: HashSet<u32> = files.iter().copied().collect();
    pool.into_iter().min_by(|&a, &b| {
        let key = |i: u32| {
            (
                !file_set.contains(&i),
                std::cmp::Reverse(ix.degree(i)),
                py_len(ix.label(i)),
            )
        };
        key(a).cmp(&key(b)).then_with(|| ix.id(a).cmp(ix.id(b)))
    })
}

/// NumPy's default (linear) percentile of `values`.
pub fn percentile(values: &mut [u32], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let pos = p / 100.0 * (values.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(values.len() - 1);
    let (_, &mut a, rest) = values.select_nth_unstable(lo);
    let b = if hi == lo {
        a
    } else {
        *rest.iter().min().unwrap()
    };
    let frac = pos - lo as f64;
    a as f64 + (b as f64 - a as f64) * frac
}

/// The hub threshold `max(50, P99(degree))` (§3.7).
pub fn hub_threshold(ix: &Index) -> f64 {
    let mut degrees: Vec<u32> = (0..ix.node_count() as u32).map(|i| ix.degree(i)).collect();
    percentile(&mut degrees, 99.0).max(MIN_HUB_DEGREE)
}

/// Pick the seeds for `terms` (§3.6). Empty when nothing scores.
pub fn seeds(ix: &Index, terms: &[String]) -> Vec<u32> {
    if terms.is_empty() {
        return Vec::new();
    }
    let hits: Vec<Vec<u32>> = terms.iter().map(|t| label_hits(ix, t)).collect();
    let files = file_hits(ix, terms);
    let mut scored = score(ix, terms, &hits, &files);
    if scored.is_empty() {
        return Vec::new();
    }
    scored.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| py_len(ix.label(a.0)).cmp(&py_len(ix.label(b.0))))
            .then_with(|| ix.id(a.0).cmp(ix.id(b.0)))
    });
    let top = scored[0].1;
    let mut taken_labels: HashSet<&str> = HashSet::new();
    let mut seeds: Vec<u32> = Vec::new();
    for &(i, s) in &scored {
        if !taken_labels.insert(ix.lower_label(i)) {
            continue;
        }
        if s < SEED_RATIO * top || seeds.len() == MAX_PRIMARY_SEEDS {
            break;
        }
        seeds.push(i);
    }
    let all_relation = terms.iter().all(|t| RELATION_WORDS.contains(&t.as_str()));
    let mut order: Vec<usize> = (0..terms.len()).collect();
    order.sort_by(|&a, &b| terms[a].cmp(&terms[b]));
    for k in order {
        let t = &terms[k];
        if !all_relation && RELATION_WORDS.contains(&t.as_str()) {
            continue;
        }
        let Some(best) = coverage_seed(ix, t, &hits[k], &files[k]) else {
            continue;
        };
        let lower = ix.lower_label(best);
        if seeds
            .iter()
            .any(|&s| s == best || ix.lower_label(s) == lower)
        {
            continue;
        }
        seeds.push(best);
    }
    seeds
}

/// Answer a question. `None` when nothing matched.
pub fn query(ix: &Index, question: &str, opts: &QueryOptions) -> Option<QueryAnswer> {
    let terms = terms(question);
    let seeds = seeds(ix, &terms);
    if seeds.is_empty() {
        return None;
    }
    let context = match &opts.context {
        Some(values) => explicit_context(values),
        None => heuristic_context(question),
    };
    let filter = LinkFilter::new(ix, context.as_ref());
    // With a context filter the percentile is taken over the filtered degrees (kubernetes `who
    // calls infeasible`: call-degree P99 is 14, so a 92-link node is a hub at 50; over all links
    // P99 is 156 and it would be expanded). The node's own degree stays the full one (§3.7).
    let hub = if filter.active() {
        let mut degrees = vec![0u32; ix.node_count()];
        for e in 0..ix.edge_count() as u32 {
            if filter.passes(e) {
                let (s, t) = ix.endpoints(e);
                degrees[s as usize] += 1;
                degrees[t as usize] += 1;
            }
        }
        percentile(&mut degrees, 99.0).max(MIN_HUB_DEGREE)
    } else {
        hub_threshold(ix)
    };
    let is_seed: HashSet<u32> = seeds.iter().copied().collect();
    let expandable = |u: u32| is_seed.contains(&u) || (ix.degree(u) as f64) < hub;

    let mut visited: HashSet<u32> = HashSet::new();
    let mut visit: Vec<u32> = Vec::new();
    if opts.dfs {
        let mut stack: Vec<(u32, u32)> = seeds.iter().rev().map(|&s| (s, 0)).collect();
        while let Some((u, d)) = stack.pop() {
            if !visited.insert(u) {
                continue;
            }
            visit.push(u);
            if d < opts.depth && expandable(u) {
                for (e, v) in ix.adjacent(u) {
                    if filter.passes(e) && !visited.contains(&v) {
                        stack.push((v, d + 1));
                    }
                }
            }
        }
    } else {
        let mut frontier: Vec<u32> = Vec::new();
        for &s in &seeds {
            if visited.insert(s) {
                visit.push(s);
                frontier.push(s);
            }
        }
        for _ in 0..opts.depth {
            let mut next = Vec::new();
            for &u in &frontier {
                if !expandable(u) {
                    continue;
                }
                for (e, v) in ix.adjacent(u) {
                    if filter.passes(e) && visited.insert(v) {
                        visit.push(v);
                        next.push(v);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
    }

    // Distance from the seed set over the filtered links, ignoring the hub rule, for reached
    // nodes; every reached node is within `depth`.
    let mut dist: HashMap<u32, u32> = seeds.iter().map(|&s| (s, 0)).collect();
    let mut remaining = visit.iter().filter(|v| !dist.contains_key(v)).count();
    let mut frontier: Vec<u32> = seeds.clone();
    let mut seen: HashSet<u32> = seeds.iter().copied().collect();
    let mut d = 0;
    while remaining > 0 && d + 1 < opts.depth.max(1) && !frontier.is_empty() {
        d += 1;
        let mut next = Vec::new();
        for &u in &frontier {
            for (e, v) in ix.adjacent(u) {
                if filter.passes(e) && seen.insert(v) {
                    next.push(v);
                    if visited.contains(&v) && !dist.contains_key(&v) {
                        dist.insert(v, d);
                        remaining -= 1;
                    }
                }
            }
        }
        frontier = next;
    }
    let far = opts.depth.max(1);
    let filtered_degree = |u: u32| -> u32 {
        if !filter.active() {
            return ix.degree(u);
        }
        ix.adjacent(u)
            .filter(|&(e, _)| filter.passes(e))
            .map(|(e, _)| {
                let (s, t) = ix.endpoints(e);
                if s == t {
                    2
                } else {
                    1
                }
            })
            .sum()
    };
    let mut rest: Vec<(u32, u32, u32)> = visit
        .iter()
        .filter(|v| !is_seed.contains(v))
        .map(|&v| (*dist.get(&v).unwrap_or(&far), filtered_degree(v), v))
        .collect();
    rest.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| ix.id(a.2).cmp(ix.id(b.2)))
    });
    let mut nodes = seeds.clone();
    nodes.extend(rest.into_iter().map(|(_, _, v)| v));
    Some(QueryAnswer {
        terms,
        seeds,
        nodes,
        visit,
        context,
        dfs: opts.dfs,
        depth: opts.depth,
    })
}

/// Links among the reached nodes that pass the filter, one per node pair: for each node in visit
/// order, its links in graph.json order (§3.9).
pub fn answer_edges(ix: &Index, ans: &QueryAnswer) -> Vec<u32> {
    let filter = LinkFilter::new(ix, ans.context.as_ref());
    let reached: HashSet<u32> = ans.visit.iter().copied().collect();
    let mut pairs: HashSet<(u32, u32)> = HashSet::new();
    let mut out = Vec::new();
    for &u in &ans.visit {
        for (e, v) in ix.adjacent(u) {
            // Self-loops never print (observed on the go and typescript goldens).
            if v == u || !reached.contains(&v) || !filter.passes(e) {
                continue;
            }
            let key = (u.min(v), u.max(v));
            if pairs.insert(key) {
                out.push(e);
            }
        }
    }
    out
}

pub fn node_line(ix: &Index, i: u32) -> String {
    let n = ix.node(i);
    format!(
        "NODE {} [src={} loc={} community={}]",
        n.label, n.source_file, n.source_location, n.community
    )
}

pub fn edge_line(ix: &Index, e: u32) -> String {
    let e = ix.edge(e);
    let ctx = e
        .context
        .map(|c| format!(" context={c}"))
        .unwrap_or_default();
    let at = if e.source_file.is_none() && e.source_location.is_none() {
        String::new()
    } else {
        format!(
            " at={}:{}",
            e.source_file.unwrap_or(""),
            e.source_location.unwrap_or("")
        )
    };
    format!(
        "EDGE {} --{} [{}{}]--> {}{}",
        ix.label(e.source),
        e.relation,
        e.confidence.unwrap_or(""),
        ctx,
        ix.label(e.target),
        at
    )
}

/// Graphify's `query` text for an answer (no trailing newline). `graph` is the path shown in the
/// header.
pub fn render(ix: &Index, graph: &str, ans: &QueryAnswer, budget: i64) -> String {
    let start = py_repr_list(ans.seeds.iter().map(|&s| ix.label(s)));
    let context = ans
        .context
        .as_ref()
        .map(|c| {
            format!(
                "Context: {} ({}) | ",
                c.values.join(", "),
                if c.explicit { "explicit" } else { "heuristic" }
            )
        })
        .unwrap_or_default();
    let total = ans.nodes.len();
    let header = format!(
        "Graph: {graph} ({} nodes) | Traversal: {} depth={} | Start: {start} | {context}{total} nodes found",
        ix.node_count(),
        if ans.dfs { "DFS" } else { "BFS" },
        ans.depth,
    );
    let node_lines: Vec<String> = ans.nodes.iter().map(|&i| node_line(ix, i)).collect();
    let costs: Vec<i64> = node_lines.iter().map(|l| py_len(l) as i64 + 1).collect();
    let node_chars: i64 = costs.iter().sum();
    let kept = if budget < 0 {
        total
    } else {
        let mut used = 0i64;
        let mut k = 0;
        for c in &costs {
            if used + c > budget * 3 {
                break;
            }
            used += c;
            k += 1;
        }
        k.max(ans.seeds.len()).min(total)
    };
    let mut out = vec![header, String::new()];
    if kept < total {
        let need = (node_chars + 2) / 3;
        let cut = total - kept;
        out.push(format!(
            "[!] TRUNCATED: showing {kept} of {total} nodes (~{budget}-token budget). The answer may be among the {cut} cut nodes — raise the token budget to {need} to get all {total} (CLI: --budget {need}, MCP: token_budget={need}) or narrow the query (e.g. context_filter=['call'], or get_node for a specific symbol)."
        ));
        out.push(String::new());
        out.extend(node_lines.into_iter().take(kept));
        out.push(format!(
            "... (truncated — {cut} more nodes cut by ~{budget}-token budget. Narrow with context_filter=['call'] or use get_node for a specific symbol)"
        ));
        return out.join("\n");
    }
    let edge_lines: Vec<String> = answer_edges(ix, ans)
        .into_iter()
        .map(|e| edge_line(ix, e))
        .collect();
    let edge_chars: i64 = edge_lines.iter().map(|l| py_len(l) as i64 + 1).sum();
    // The total is the length of the lines joined by newlines (one less than the per-line
    // costs): the typescript golden reports ~3600 for 10 803 per-line characters.
    let tokens = (node_chars + edge_chars - 1).div_euclid(3);
    if tokens > budget {
        out.push(format!(
            "[i] Complete answer over budget: all {total} nodes and {} edges shown (~{tokens} tokens vs the requested ~{budget}-token budget). Edges are never dropped once every node fits, so this is already the full answer — raising --budget further will not shrink it. Narrow with context_filter=['call'] or use get_node for a specific symbol to reduce size instead.",
            edge_lines.len()
        ));
        out.push(String::new());
    }
    out.extend(node_lines);
    out.extend(edge_lines);
    out.join("\n")
}

pub const NO_MATCH: &str = "No matching nodes found.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_follow_the_spec() {
        assert_eq!(terms("UserService.register()"), ["userservice", "register"]);
        assert_eq!(terms("how does it work"), ["how", "does", "work"]);
        assert_eq!(terms("the"), ["the"]);
        assert_eq!(terms("n_1 is v2"), ["n", "1", "v2"]);
        assert_eq!(terms("build_from_json"), ["build", "from", "json"]);
        assert_eq!(terms("re fo xy"), Vec::<String>::new());
        assert_eq!(terms("user-service"), ["user", "service"]);
    }

    #[test]
    fn context_heuristic_uses_column_order() {
        let c = heuristic_context("which modules return values, who calls?").unwrap();
        assert_eq!(c.values, ["call", "import", "return_type"]);
        assert!(heuristic_context("calling (calls)").is_none());
    }

    #[test]
    fn percentile_is_linear() {
        let mut v = vec![1, 2, 3, 4];
        assert!((percentile(&mut v, 50.0) - 2.5).abs() < 1e-12);
        let mut v = vec![5];
        assert_eq!(percentile(&mut v, 99.0), 5.0);
    }
}
