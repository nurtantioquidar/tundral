//! The wiki: `index.md`, one article per community and one per god node
//! (`docs/graphify-exports.md` §3).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use dijkstral_core::analyze::is_god_node;
use dijkstral_core::facts::Confidence;

use crate::view::View;

pub const WIKI_DIR: &str = "wiki";
pub const INDEX_FILE: &str = "index.md";

const GOD_COUNT: usize = 10;
const KEY_CONCEPTS: usize = 25;
const RELATIONSHIPS: usize = 12;
const SOURCE_FILES: usize = 20;
const PER_RELATION: usize = 20;
const SLUG_CHARS: usize = 200;

const ARTICLE_FOOTER: &str =
    "---\n\n*Part of the dijkstral knowledge wiki. See [index](index.md) to navigate.*";

/// Every article: file name (without directory) and Markdown, `index.md` first.
pub fn render(view: &View) -> Result<Vec<(String, String)>> {
    if !view.has_communities() {
        bail!("the graph has no communities; build it with clustering before exporting the wiki");
    }
    let communities = view.communities();
    let gods = god_nodes(view);

    let mut names = FileNames::default();
    let community_files: HashMap<u32, String> = communities
        .iter()
        .map(|(c, _)| (*c, names.claim(&view.names.get(*c))))
        .collect();
    let god_files: Vec<(u32, String)> = gods
        .iter()
        .map(|&g| (g, names.claim(&view.graph.nodes[g as usize].label)))
        .collect();
    // Neighbours are linked by label (§3.4): any node labelled like an article links to it, the
    // first article claiming a label winning (communities before god nodes).
    let mut by_label: HashMap<String, &str> = HashMap::new();
    for (c, _) in &communities {
        by_label
            .entry(view.names.get(*c))
            .or_insert(community_files[c].as_str());
    }
    for (g, file) in &god_files {
        by_label
            .entry(view.graph.nodes[*g as usize].label.clone())
            .or_insert(file.as_str());
    }

    let mut out = Vec::with_capacity(1 + communities.len() + gods.len());
    out.push((
        INDEX_FILE.to_string(),
        index(view, &communities, &community_files, &god_files),
    ));
    for (c, members) in &communities {
        out.push((
            community_files[c].clone(),
            community_article(view, *c, members, &community_files),
        ));
    }
    for (g, file) in &god_files {
        out.push((
            file.clone(),
            god_article(view, *g, &community_files, &by_label),
        ));
    }
    Ok(out)
}

/// Write the articles into `dir` (created if needed). Articles already there are replaced; files
/// this export did not produce are left alone.
pub fn write(view: &View, dir: &Path) -> Result<usize> {
    let articles = render(view)?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for (name, text) in &articles {
        let path = dir.join(name);
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(articles.len())
}

/// The ten best-connected symbols (§3.3): `analyze`'s god-node rule, ties in node order.
pub fn god_nodes(view: &View) -> Vec<u32> {
    let mut cands: Vec<u32> = view
        .nodes
        .iter()
        .copied()
        .filter(|&i| {
            let n = &view.graph.nodes[i as usize];
            is_god_node(&n.label, &n.source_file, view.degree[i as usize])
        })
        .collect();
    cands.sort_by_key(|&i| std::cmp::Reverse(view.degree[i as usize]));
    cands.truncate(GOD_COUNT);
    cands
}

/// Article file names (§3.2): one per label, unique without regard to case, `index` reserved.
#[derive(Default)]
struct FileNames {
    counts: HashMap<String, u32>,
    taken: HashSet<String>,
}

impl FileNames {
    fn claim(&mut self, label: &str) -> String {
        if self.taken.is_empty() {
            self.taken.insert("index".into());
            self.counts.insert("index".into(), 1);
        }
        let base = slug(label);
        let key = base.to_lowercase();
        let n = self.counts.entry(key).or_insert(0);
        loop {
            *n += 1;
            let name = if *n == 1 {
                base.clone()
            } else {
                format!("{base}_{n}")
            };
            if self.taken.insert(name.to_lowercase()) {
                return format!("{name}.md");
            }
        }
    }
}

/// A label as a file stem: `(` `)` dropped, `/` `:` → `-`, `"` `#` `%` `*` `<` `>` `?` `\` `|`
/// whitespace and control characters → `_`, at most 200 characters, `unnamed` when nothing but
/// dots is left.
pub fn slug(label: &str) -> String {
    let mut s = String::with_capacity(label.len());
    let mut chars = 0;
    for c in label.chars() {
        if chars == SLUG_CHARS {
            break;
        }
        match c {
            '(' | ')' => continue,
            '/' | ':' => s.push('-'),
            '"' | '#' | '%' | '*' | '<' | '>' | '?' | '\\' | '|' => s.push('_'),
            c if (c as u32) <= 0x20 || c == '\u{7f}' => s.push('_'),
            c => s.push(c),
        }
        chars += 1;
    }
    if s.chars().all(|c| c == '.') {
        "unnamed".into()
    } else {
        s
    }
}

/// Link text: square brackets escaped, nothing else.
fn link_text(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

fn index(
    view: &View,
    communities: &[(u32, Vec<u32>)],
    files: &HashMap<u32, String>,
    gods: &[(u32, String)],
) -> String {
    let mut s = String::new();
    s.push_str("# Knowledge Graph Index\n\n");
    s.push_str("> Auto-generated by dijkstral. Start here — read community articles for context, then drill into god nodes for detail.\n\n");
    let _ = writeln!(
        s,
        "**{} nodes · {} edges · {} communities**\n",
        view.nodes.len(),
        view.edges.len(),
        communities.len()
    );
    s.push_str("---\n\n## Communities\n(sorted by size, largest first)\n\n");
    let mut by_size: Vec<&(u32, Vec<u32>)> = communities.iter().collect();
    by_size.sort_by_key(|(_, m)| std::cmp::Reverse(m.len()));
    for (c, members) in by_size {
        let _ = writeln!(
            s,
            "- [{}]({}) — {} nodes",
            link_text(&view.names.get(*c)),
            files[c],
            members.len()
        );
    }
    s.push_str("\n## God Nodes\n(most connected concepts — the load-bearing abstractions)\n\n");
    for (g, file) in gods {
        let _ = writeln!(
            s,
            "- [{}]({}) — {} connections",
            link_text(&view.graph.nodes[*g as usize].label),
            file,
            view.degree[*g as usize]
        );
    }
    s.push_str("\n---\n\n*Generated by dijkstral*");
    s
}

fn percent(part: usize, total: usize) -> String {
    if total == 0 {
        return "0".into();
    }
    format!("{:.0}", 100.0 * part as f64 / total as f64)
}

fn community_article(view: &View, c: u32, members: &[u32], files: &HashMap<u32, String>) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "# {}\n\n> {} nodes\n\n## Key Concepts\n\n",
        view.names.get(c),
        members.len()
    );
    let mut ranked = members.to_vec();
    ranked.sort_by_key(|&i| std::cmp::Reverse(view.degree[i as usize]));
    for &i in ranked.iter().take(KEY_CONCEPTS) {
        let n = &view.graph.nodes[i as usize];
        let _ = write!(
            s,
            "- **{}** ({} connections)",
            n.label, view.degree[i as usize]
        );
        if !n.source_file.is_empty() {
            let _ = write!(s, " — `{}`", n.source_file);
        }
        s.push('\n');
    }
    if members.len() > KEY_CONCEPTS {
        let _ = writeln!(
            s,
            "- *... and {} more nodes in this community*",
            members.len() - KEY_CONCEPTS
        );
    }

    // Links to other communities, counted from each member's side; ties keep first-seen order.
    let mut order: Vec<u32> = Vec::new();
    let mut counts: HashMap<u32, usize> = HashMap::new();
    let mut audit: BTreeMap<u8, usize> = BTreeMap::new();
    let mut seen_edges: HashSet<u32> = HashSet::new();
    for &i in members {
        for &(v, e) in &view.adj[i as usize] {
            if seen_edges.insert(e) {
                let conf = match view.graph.edges[e as usize].confidence {
                    Confidence::Extracted => 0,
                    Confidence::Inferred => 1,
                    Confidence::Ambiguous => 2,
                };
                *audit.entry(conf).or_default() += 1;
            }
            match view.community[v as usize] {
                Some(other) if other != c => {
                    let n = counts.entry(other).or_insert(0);
                    if *n == 0 {
                        order.push(other);
                    }
                    *n += 1;
                }
                _ => {}
            }
        }
    }
    s.push_str("\n## Relationships\n\n");
    if order.is_empty() {
        s.push_str("- No strong cross-community connections detected\n");
    } else {
        let mut ranked: Vec<u32> = order;
        ranked.sort_by_key(|o| std::cmp::Reverse(counts[o]));
        for o in ranked.into_iter().take(RELATIONSHIPS) {
            let _ = writeln!(
                s,
                "- [{}]({}) ({} shared connections)",
                link_text(&view.names.get(o)),
                files[&o],
                counts[&o]
            );
        }
    }

    let files: BTreeSet<&str> = members
        .iter()
        .map(|&i| view.graph.nodes[i as usize].source_file.as_str())
        .filter(|f| !f.is_empty())
        .collect();
    if !files.is_empty() {
        s.push_str("\n## Source Files\n\n");
        for f in files.into_iter().take(SOURCE_FILES) {
            let _ = writeln!(s, "- `{f}`");
        }
    }

    let total: usize = audit.values().sum();
    s.push_str("\n## Audit Trail\n\n");
    for (k, name) in ["EXTRACTED", "INFERRED", "AMBIGUOUS"].iter().enumerate() {
        let n = audit.get(&(k as u8)).copied().unwrap_or(0);
        let _ = writeln!(s, "- {name}: {n} ({}%)", percent(n, total));
    }
    s.push('\n');
    s.push_str(ARTICLE_FOOTER);
    s
}

fn god_article(
    view: &View,
    g: u32,
    community_files: &HashMap<u32, String>,
    by_label: &HashMap<String, &str>,
) -> String {
    let n = &view.graph.nodes[g as usize];
    let mut s = String::new();
    let _ = write!(
        s,
        "# {}\n\n> God node · {} connections",
        n.label, view.degree[g as usize]
    );
    if !n.source_file.is_empty() {
        let _ = write!(s, " · `{}`", n.source_file);
    }
    s.push_str("\n\n");
    if let Some(c) = view.community[g as usize] {
        let name = view.names.get(c);
        if !name.is_empty() {
            let _ = write!(
                s,
                "**Community:** [{}]({})\n\n",
                link_text(&name),
                community_files[&c]
            );
        }
    }
    s.push_str("## Connections by Relation\n\n");
    let mut groups: BTreeMap<&str, Vec<(u32, u32)>> = BTreeMap::new();
    for &(v, e) in &view.adj[g as usize] {
        groups
            .entry(view.graph.edges[e as usize].relation.as_str())
            .or_default()
            .push((v, e));
    }
    for (relation, mut items) in groups {
        let _ = writeln!(s, "### {relation}");
        items.sort_by_key(|&(v, _)| std::cmp::Reverse(view.degree[v as usize]));
        for &(v, e) in items.iter().take(PER_RELATION) {
            let label = &view.graph.nodes[v as usize].label;
            match by_label.get(label) {
                Some(file) => {
                    let _ = write!(s, "- [{}]({file})", link_text(label));
                }
                None => {
                    let _ = write!(s, "- {label}");
                }
            }
            if view.no_confidence.contains(&e) {
                s.push('\n');
            } else {
                let _ = writeln!(s, " `{}`", view.graph.edges[e as usize].confidence.as_str());
            }
        }
        if items.len() > PER_RELATION {
            let _ = writeln!(
                s,
                "- *…and {} more `{relation}` connection(s) not listed (lowest-degree first to go)*",
                items.len() - PER_RELATION
            );
        }
        s.push('\n');
    }
    s.push_str(ARTICLE_FOOTER);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_follow_graphify() {
        assert_eq!(slug("run()"), "run");
        assert_eq!(slug("Community 0"), "Community_0");
        assert_eq!(
            slug("</script><script>alert(1)</script>"),
            "_-script__script_alert1_-script_"
        );
        assert_eq!(
            slug("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"),
            "!__$_&'_+,-.--;_=__@[_]^_`{_}~"
        );
        assert_eq!(slug("\u{0}ctl\u{7}\ttab"), "_ctl__tab");
        assert_eq!(
            slug("\u{a0}nbsp\u{3000}ideo\u{2003}em\u{7f}del\u{1f}us"),
            "\u{a0}nbsp\u{3000}ideo\u{2003}em_del_us"
        );
        assert_eq!(slug(""), "unnamed");
        assert_eq!(slug(".."), "unnamed");
        assert_eq!(slug("   "), "___");
        assert_eq!(slug("trailing. "), "trailing._");
        assert_eq!(slug(&"é".repeat(250)).chars().count(), 200);
        assert_eq!(slug(&format!("{}()", "a".repeat(199))), "a".repeat(199));
    }

    #[test]
    fn names_are_unique_ignoring_case() {
        let mut f = FileNames::default();
        let got: Vec<String> = ["Foo", "foo", "run()", "run", "Foo", "index", "", ".."]
            .iter()
            .map(|l| f.claim(l))
            .collect();
        assert_eq!(
            got,
            [
                "Foo.md",
                "foo_2.md",
                "run.md",
                "run_2.md",
                "Foo_3.md",
                "index_2.md",
                "unnamed.md",
                "unnamed_2.md"
            ]
        );
    }
}
