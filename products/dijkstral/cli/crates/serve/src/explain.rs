//! `explain`: one node and its connections (`docs/graphify-query.md` §6).

use std::collections::HashMap;

use dijkstral_core::index::Index;

/// Connections shown without `--all`.
pub const TOP: usize = 20;
/// File groups listed in the footer.
const TOP_GROUPS: usize = 20;

/// A connection of the explained node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connection {
    pub link: u32,
    /// The link leaves the explained node.
    pub outgoing: bool,
    pub neighbour: u32,
}

/// Outgoing links in link order, then incoming links in link order, stably sorted by the
/// neighbour's degree, descending.
pub fn connections(ix: &Index, node: u32) -> Vec<Connection> {
    let mut out: Vec<Connection> = Vec::new();
    for outgoing in [true, false] {
        for e in ix.incident(node) {
            let (s, t) = ix.endpoints(e);
            if outgoing && s == node {
                out.push(Connection {
                    link: e,
                    outgoing,
                    neighbour: t,
                });
            } else if !outgoing && t == node {
                out.push(Connection {
                    link: e,
                    outgoing,
                    neighbour: s,
                });
            }
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(ix.degree(c.neighbour)));
    out
}

fn connection_line(ix: &Index, c: &Connection) -> String {
    let e = ix.edge(c.link);
    let arrow = if c.outgoing { "-->" } else { "<--" };
    let at = if e.source_file.is_none() && e.source_location.is_none() {
        String::new()
    } else {
        format!(
            " {}:{}",
            e.source_file.unwrap_or(""),
            e.source_location.unwrap_or("")
        )
    };
    format!(
        "  {arrow} {} [{}] [{}]{at}",
        ix.label(c.neighbour),
        e.relation,
        e.confidence.unwrap_or("")
    )
}

/// Graphify's `explain` text (no trailing newline).
pub fn render(ix: &Index, node: u32, all: bool) -> String {
    render_only(ix, node, all, None)
}

/// [`render`] limited to one direction when `outgoing` is given: `Some(false)` keeps the links
/// into the node (its callers and users), `Some(true)` the links out of it (dijkstral's
/// `--incoming` / `--outgoing`, §6). Counts, the cut at [`TOP`] and the file summary then cover
/// only those links.
pub fn render_only(ix: &Index, node: u32, all: bool, outgoing: Option<bool>) -> String {
    let n = ix.node(node);
    let mut conns = connections(ix, node);
    if let Some(dir) = outgoing {
        conns.retain(|c| c.outgoing == dir);
    }
    let source = format!("  Source:    {} {}", n.source_file, n.source_location);
    let mut out = vec![
        format!("Node: {}", n.label),
        format!("  ID:        {}", n.id),
        source.trim_end().to_string(),
        format!("  Type:      {}", n.file_type),
        format!("  Community: {}", n.community),
        format!("  Degree:    {}", n.degree),
    ];
    // A node without connections ends at its header (probed on fastapi).
    if conns.is_empty() {
        return out.join("\n");
    }
    out.push(String::new());
    out.push(format!("Connections ({}):", conns.len()));
    let shown = if all {
        conns.len()
    } else {
        conns.len().min(TOP)
    };
    out.extend(conns[..shown].iter().map(|c| connection_line(ix, c)));
    if shown < conns.len() {
        let hidden = &conns[shown..];
        out.push(format!(
            "  ... and {} more (--all lists all {})",
            hidden.len(),
            conns.len()
        ));
        out.push("  Grouped by file:".into());
        let mut groups: HashMap<(bool, String), usize> = HashMap::new();
        for c in hidden {
            let file = ix
                .edge(c.link)
                .source_file
                .filter(|f| !f.is_empty())
                .unwrap_or("(unknown file)")
                .to_string();
            *groups.entry((c.outgoing, file)).or_default() += 1;
        }
        let mut groups: Vec<((bool, String), usize)> = groups.into_iter().collect();
        // Count descending, then incoming before outgoing, then path (probed on fastapi).
        groups.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0 .0.cmp(&b.0 .0))
                .then_with(|| a.0 .1.cmp(&b.0 .1))
        });
        for ((outgoing, file), k) in groups.iter().take(TOP_GROUPS) {
            let arrow = if *outgoing { "-->" } else { "<--" };
            let noun = if *k == 1 { "connection" } else { "connections" };
            out.push(format!("    {arrow} {file}: {k} {noun}"));
        }
        if groups.len() > TOP_GROUPS {
            out.push(format!(
                "    ... and {} more files",
                groups.len() - TOP_GROUPS
            ));
        }
    }
    out.join("\n")
}
