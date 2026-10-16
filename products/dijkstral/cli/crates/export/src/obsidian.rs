//! The Obsidian vault (`docs/graphify-exports.md`, Obsidian section): one note per node, one
//! `_COMMUNITY_<name>` note per community, `graph.canvas` (communities as groups on a grid),
//! `.obsidian/graph.json` (community colour groups) and the ownership manifest
//! `.graphify_obsidian_manifest.json`, which records the notes the exporter owns so a rerun
//! prunes its stale notes and never overwrites a note it did not write.
//!
//! The text is Graphify's byte for byte for any graph Graphify itself writes. Untrusted strings
//! differ only where Graphify's output would break: control characters in headings and list
//! text, raw tags, backticks in relations, and a node note named like a community note.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use dijkstral_core::pyjson::{self, Json};
use serde_json::Value;

use crate::html::PALETTE;
use crate::nx::{Attr, NxGraph};
use crate::view::Names;

/// The vault's directory next to the graph when `--dir` is not given.
pub const OBSIDIAN_DIR: &str = "obsidian";
pub const CANVAS_FILE: &str = "graph.canvas";
/// The notes the exporter owns (Graphify's name, so both tools respect each other's vaults).
pub const MANIFEST_FILE: &str = ".graphify_obsidian_manifest.json";
pub const SETTINGS_FILE: &str = ".obsidian/graph.json";
/// `graph.canvas` keeps this many links, heaviest first.
pub const CANVAS_EDGE_LIMIT: usize = 200;
const BRIDGE_LIMIT: usize = 5;

/// A rendered vault: every file but the manifest, by path relative to the vault.
pub struct Vault {
    /// Notes and settings, in write order (node notes, community notes, settings).
    pub files: Vec<(String, String)>,
    pub canvas: String,
}

/// What `write` did.
#[derive(Debug, Default)]
pub struct Written {
    /// Notes (node and community) written or already current.
    pub notes: usize,
    /// Existing files the exporter did not create, left untouched.
    pub skipped: Vec<String>,
    /// Notes of an earlier export no longer in the graph, removed.
    pub pruned: usize,
}

/// Python's `str.isspace` for the characters graph text can hold.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `\w` (approximately `str.isalnum()` or `_`).
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn control(c: char) -> bool {
    (c as u32) < 0x20 || c == '\u{7f}'
}

/// A note's file name (without `.md`) for a label: control characters become spaces;
/// `\ / : * ? " < > | [ ] # ^` are dropped; surrounding whitespace is trimmed; a name without a
/// word character is `unnamed`; leading dots become `dot-`; above 200 UTF-8 bytes the name is cut
/// to 191 bytes plus `_` and 8 hex digits of its SHA-1.
pub fn note_name(label: &str) -> String {
    let mut s: String = label
        .chars()
        .filter_map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '#' | '^' => None,
            c if control(c) => Some(' '),
            c => Some(c),
        })
        .collect();
    s = s.trim_matches(py_space).to_string();
    if !s.chars().any(word) {
        s = "unnamed".into();
    } else if s.starts_with('.') {
        s = format!("dot-{}", s.trim_start_matches('.'));
    }
    if s.len() > 200 {
        let hash = sha1_hex(s.as_bytes());
        let mut cut = 191;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s = format!("{}_{}", &s[..cut], &hash[..8]);
    }
    s
}

/// A tag body: spaces become `_`, everything but word characters, `/` and `-` is dropped.
fn tag(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            ' ' => Some('_'),
            c if word(c) || c == '/' || c == '-' => Some(c),
            _ => None,
        })
        .collect()
}

/// The community tag: `tag`, `unnamed` when nothing is left.
fn community_tag(name: &str) -> String {
    let t = tag(name);
    if t.is_empty() {
        "unnamed".into()
    } else {
        t
    }
}

/// Text on one Markdown line: control characters other than tab become spaces.
fn line_text(s: &str) -> String {
    s.chars()
        .map(|c| if control(c) && c != '\t' { ' ' } else { c })
        .collect()
}

/// A code span that a backtick in the text cannot end.
fn code_span(s: &str) -> String {
    let s = line_text(s);
    if s.contains('`') {
        format!("`` {s} ``")
    } else {
        format!("`{s}`")
    }
}

/// A YAML double-quoted scalar as Graphify writes it.
fn yaml_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => out.push_str("\\0"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\L"),
            '\u{2029}' => out.push_str("\\P"),
            c if control(c) => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Python's `str(value)` for a scalar attribute, `None` when the value is falsy.
fn py_str(v: Option<&Attr>) -> Option<String> {
    match v? {
        Attr::Str(s) if !s.is_empty() => Some(s.to_string()),
        Attr::Bool(true) => Some("True".into()),
        Attr::Int(i) => (*i != 0).then(|| i.to_string()),
        Attr::Float(f) => (*f != 0.0).then(|| pyjson::float_repr(*f)),
        Attr::Json(v) => match &**v {
            // An integer beyond 64 bits.
            Value::Number(n) => Some(n.to_string()),
            Value::Array(a) if !a.is_empty() => Some(v.to_string()),
            Value::Object(o) if !o.is_empty() => Some(v.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn str_attr<'a>(g: &'a NxGraph, i: u32, key: &str) -> &'a str {
    g.node_str(i, key).unwrap_or("")
}

fn community_name(names: &Names, c: i64) -> String {
    match u32::try_from(c) {
        Ok(c) => names.get(c),
        Err(_) => format!("Community {c}"),
    }
}

fn confidence(g: &NxGraph, e: u32) -> &str {
    match g.edges[e as usize].attrs.get("confidence") {
        Some(Attr::Str(s)) => s,
        Some(_) => "",
        None => "EXTRACTED",
    }
}

/// Claims names in order, unique ignoring case: the n-th claim of a taken name gets the first
/// free `_<k>`.
#[derive(Default)]
struct Claims {
    taken: HashSet<String>,
}

impl Claims {
    fn claim(&mut self, base: &str, also_taken: impl Fn(&str) -> bool) -> String {
        let free = |s: &str, taken: &HashSet<String>| {
            let key = s.to_lowercase();
            !taken.contains(&key) && !also_taken(&key)
        };
        let mut name = base.to_string();
        let mut k = 1;
        while !free(&name, &self.taken) {
            name = format!("{base}_{k}");
            k += 1;
        }
        self.taken.insert(name.to_lowercase());
        name
    }
}

/// The vault as Graphify lays it out.
pub fn render(g: &NxGraph, names: &Names) -> Vault {
    let n = g.nodes.len() as u32;
    let comm: Vec<Option<i64>> = (0..n).map(|i| g.community(i)).collect();
    let communities = g.communities();

    // Note names: nodes in order, then communities in order of first appearance, avoiding the
    // node notes (a node labelled `_COMMUNITY_x` would otherwise share a file with community x).
    let mut claims = Claims::default();
    let file: Vec<String> = (0..n)
        .map(|i| claims.claim(&note_name(g.label(i)), |_| false))
        .collect();
    let node_keys = claims.taken;
    let mut cclaims = Claims::default();
    let cfile: HashMap<i64, String> = communities
        .iter()
        .map(|(c, _)| {
            let base = note_name(&community_name(names, *c));
            let name = cclaims.claim(&base, |key| {
                node_keys.contains(&format!("_community_{key}"))
            });
            (*c, name)
        })
        .collect();

    let mut files = Vec::with_capacity(n as usize + communities.len() + 1);
    let by_label = |members: &mut Vec<u32>| members.sort_by(|&a, &b| g.label(a).cmp(g.label(b)));
    for i in 0..n {
        files.push((
            format!("{}.md", file[i as usize]),
            node_note(g, names, &comm, &file, i),
        ));
    }

    // Links between communities, counted in `G.edges()` order (that order breaks count ties).
    let mut cross: HashMap<i64, Vec<(i64, usize)>> = HashMap::new();
    let mut add = |a: i64, b: i64| {
        let list = cross.entry(a).or_default();
        match list.iter_mut().find(|(x, _)| *x == b) {
            Some(slot) => slot.1 += 1,
            None => list.push((b, 1)),
        }
    };
    for (u, v, _) in g.edge_walk() {
        if let (Some(cu), Some(cv)) = (comm[u as usize], comm[v as usize]) {
            if cu != cv {
                add(cu, cv);
                add(cv, cu);
            }
        }
    }
    for (c, members) in &communities {
        let mut sorted = members.clone();
        by_label(&mut sorted);
        let text = community_note(g, names, &comm, &file, &cfile, *c, members, &sorted, &cross);
        files.push((format!("_COMMUNITY_{}.md", cfile[c]), text));
    }
    files.push((SETTINGS_FILE.to_string(), settings(names)));

    // The canvas: a graph without communities is drawn as one group 0 holding every node.
    let mut groups: Vec<(i64, Vec<u32>)> = if communities.is_empty() {
        if n == 0 {
            Vec::new()
        } else {
            vec![(0, (0..n).collect())]
        }
    } else {
        communities
    };
    groups.sort_by_key(|(c, _)| *c);
    for (_, members) in &mut groups {
        by_label(members);
    }
    let canvas = canvas(g, names, &groups, &file);
    Vault { files, canvas }
}

fn node_note(g: &NxGraph, names: &Names, comm: &[Option<i64>], file: &[String], i: u32) -> String {
    let attrs = &g.nodes[i as usize].attrs;
    let ft = str_attr(g, i, "file_type");
    let cname = match comm[i as usize] {
        Some(c) => community_name(names, c),
        None => "Community None".into(),
    };
    // The most common confidence of the node's links, ties to the first met.
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for &(_, e) in &g.adj[i as usize] {
        let conf = confidence(g, e);
        match counts.iter_mut().find(|(c, _)| *c == conf) {
            Some(slot) => slot.1 += 1,
            None => counts.push((conf, 1)),
        }
    }
    let mut dominant = "EXTRACTED";
    let mut best = 0;
    for &(c, k) in &counts {
        if k > best {
            (dominant, best) = (c, k);
        }
    }
    let ft_tag = tag(if ft.is_empty() { "document" } else { ft });
    let conf_tag = tag(dominant);
    let c_tag = community_tag(&cname);

    let mut out = String::from("---\nsource_file: ");
    yaml_str(str_attr(g, i, "source_file"), &mut out);
    out.push_str("\ntype: ");
    yaml_str(ft, &mut out);
    out.push_str("\ncommunity: ");
    yaml_str(&cname, &mut out);
    if let Some(loc) = py_str(attrs.get("source_location")) {
        out.push_str("\nlocation: ");
        yaml_str(&loc, &mut out);
    }
    let _ = write!(
        out,
        "\ntags:\n  - graphify/{ft_tag}\n  - graphify/{conf_tag}\n  - community/{c_tag}\n---\n\n# {}\n\n",
        line_text(g.label(i))
    );
    // One line per neighbour: in a multigraph, its first parallel link.
    let mut listed = HashSet::new();
    let mut nbrs: Vec<(u32, u32)> = g.adj[i as usize]
        .iter()
        .copied()
        .filter(|&(v, _)| listed.insert(v))
        .collect();
    nbrs.sort_by(|a, b| g.label(a.0).cmp(g.label(b.0)));
    if !nbrs.is_empty() {
        out.push_str("## Connections\n");
        for (v, e) in nbrs {
            let _ = writeln!(
                out,
                "- [[{}]] - {} [{}]",
                file[v as usize],
                code_span(g.edge_str(e, "relation").unwrap_or("")),
                line_text(confidence(g, e))
            );
        }
        out.push('\n');
    }
    let _ = write!(
        out,
        "#graphify/{ft_tag} #graphify/{conf_tag} #community/{c_tag}"
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn community_note(
    g: &NxGraph,
    names: &Names,
    comm: &[Option<i64>],
    file: &[String],
    cfile: &HashMap<i64, String>,
    c: i64,
    members: &[u32],
    sorted: &[u32],
    cross: &HashMap<i64, Vec<(i64, usize)>>,
) -> String {
    let name = community_name(names, c);
    let mut out = format!(
        "---\ntype: community\nmembers: {k}\n---\n\n# {}\n\n**Members:** {k} nodes\n\n## Members\n",
        line_text(&name),
        k = members.len()
    );
    for &i in sorted {
        let _ = write!(out, "- [[{}]]", file[i as usize]);
        for key in ["file_type", "source_file"] {
            let v = str_attr(g, i, key);
            if !v.is_empty() {
                let _ = write!(out, " - {}", line_text(v));
            }
        }
        out.push('\n');
    }
    let _ = write!(
        out,
        "\n## Live Query (requires Dataview plugin)\n\n```dataview\nTABLE source_file, type FROM #community/{}\nSORT file.name ASC\n```\n",
        community_tag(&name)
    );
    let Some(links) = cross.get(&c) else {
        return out;
    };
    out.push_str("\n## Connections to other communities\n");
    let mut links = links.clone();
    links.sort_by_key(|l| std::cmp::Reverse(l.1));
    for (other, k) in links {
        let _ = writeln!(
            out,
            "- {k} edge{} to [[_COMMUNITY_{}]]",
            if k == 1 { "" } else { "s" },
            cfile[&other]
        );
    }
    out.push_str("\n## Top bridge nodes");
    // Members linked to other communities: most communities first, then degree, then node order.
    let mut bridges: Vec<(u32, usize, usize)> = members
        .iter()
        .filter_map(|&i| {
            let others: BTreeSet<i64> = g.adj[i as usize]
                .iter()
                .filter_map(|&(v, _)| comm[v as usize].filter(|&cv| cv != c))
                .collect();
            (!others.is_empty()).then(|| (i, others.len(), g.degree(i)))
        })
        .collect();
    bridges.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    for (i, k, d) in bridges.into_iter().take(BRIDGE_LIMIT) {
        let _ = write!(
            out,
            "\n- [[{}]] - degree {d}, connects to {k} communit{}",
            file[i as usize],
            if k == 1 { "y" } else { "ies" }
        );
    }
    out
}

/// `.obsidian/graph.json`: one colour group per named community (every id in a labels file,
/// in id order), coloured like `graph.html`.
fn settings(names: &Names) -> String {
    let mut groups = Vec::new();
    if let Names::Named(map) = names {
        let mut ids: Vec<&u32> = map.keys().collect();
        ids.sort();
        for c in ids {
            let rgb = i64::from_str_radix(&PALETTE[*c as usize % 10][1..], 16).unwrap_or(0);
            groups.push(Json::Object(vec![
                (
                    "query".into(),
                    Json::Str(format!("tag:#community/{}", community_tag(&map[c]))),
                ),
                (
                    "color".into(),
                    Json::Object(vec![
                        ("a".into(), Json::Int(1)),
                        ("rgb".into(), Json::Int(rgb)),
                    ]),
                ),
            ]));
        }
    }
    pyjson::dumps_indent(&Json::Object(vec![(
        "colorGroups".into(),
        Json::Array(groups),
    )]))
}

fn int(v: i64) -> Json {
    Json::Int(v)
}

/// `graph.canvas`: groups on a grid of ⌈√groups⌉ columns (column width and row height the
/// largest in it, 80 apart); members by label on a grid of ⌈√members⌉ columns, 200 × 80 apart.
fn canvas(g: &NxGraph, names: &Names, groups: &[(i64, Vec<u32>)], file: &[String]) -> String {
    let per_row = (groups.len() as f64).sqrt().ceil().max(1.0) as usize;
    let sizes: Vec<(i64, i64, usize)> = groups
        .iter()
        .map(|(_, m)| {
            let cols = (m.len() as f64).sqrt().ceil().max(1.0) as usize;
            let rows = m.len().div_ceil(cols);
            (
                (cols as i64 * 220).max(600),
                (rows as i64 * 100 + 120).max(400),
                cols,
            )
        })
        .collect();
    let mut col_w = vec![0i64; per_row];
    let mut row_h = vec![0i64; groups.len().div_ceil(per_row)];
    for (j, &(w, h, _)) in sizes.iter().enumerate() {
        col_w[j % per_row] = col_w[j % per_row].max(w);
        row_h[j / per_row] = row_h[j / per_row].max(h);
    }
    let mut nodes = Vec::new();
    let mut placed = vec![false; g.nodes.len()];
    for (j, (c, members)) in groups.iter().enumerate() {
        let gx: i64 = col_w[..j % per_row].iter().map(|w| w + 80).sum();
        let gy: i64 = row_h[..j / per_row].iter().map(|h| h + 80).sum();
        let (w, h, cols) = sizes[j];
        nodes.push(Json::Object(vec![
            ("id".into(), Json::Str(format!("g{c}"))),
            ("type".into(), Json::str("group")),
            ("label".into(), Json::Str(community_name(names, *c))),
            ("x".into(), int(gx)),
            ("y".into(), int(gy)),
            ("width".into(), int(w)),
            ("height".into(), int(h)),
            ("color".into(), Json::Str((j % 6 + 1).to_string())),
        ]));
        for (q, &i) in members.iter().enumerate() {
            placed[i as usize] = true;
            nodes.push(Json::Object(vec![
                (
                    "id".into(),
                    Json::Str(format!("n_{}", g.nodes[i as usize].id)),
                ),
                ("type".into(), Json::str("file")),
                ("file".into(), Json::Str(format!("{}.md", file[i as usize]))),
                ("x".into(), int(gx + 20 + (q % cols) as i64 * 200)),
                ("y".into(), int(gy + 80 + (q / cols) as i64 * 80)),
                ("width".into(), int(180)),
                ("height".into(), int(60)),
            ]));
        }
    }
    // Links between drawn nodes in `G.edges()` order, heaviest `weight` first (default 1).
    let mut links: Vec<(f64, u32)> = g
        .edge_walk()
        .into_iter()
        .filter(|&(_, _, e)| {
            let edge = &g.edges[e as usize];
            placed[edge.from as usize] && placed[edge.to as usize]
        })
        .map(|(_, _, e)| {
            let w = g.edges[e as usize].attrs.get("weight");
            (w.and_then(Attr::as_f64).unwrap_or(1.0), e)
        })
        .collect();
    links.sort_by(|a, b| b.0.total_cmp(&a.0));
    let edges = links
        .into_iter()
        .take(CANVAS_EDGE_LIMIT)
        .map(|(_, e)| {
            let edge = &g.edges[e as usize];
            let (s, t) = (
                &g.nodes[edge.from as usize].id,
                &g.nodes[edge.to as usize].id,
            );
            let rel = g.edge_str(e, "relation").unwrap_or("");
            let label = format!("{rel} [{}]", confidence(g, e));
            Json::Object(vec![
                ("id".into(), Json::Str(format!("e_{s}_{t}"))),
                ("fromNode".into(), Json::Str(format!("n_{s}"))),
                ("toNode".into(), Json::Str(format!("n_{t}"))),
                ("label".into(), Json::str(label.trim_matches(py_space))),
            ])
        })
        .collect();
    pyjson::dumps_indent(&Json::Object(vec![
        ("nodes".into(), Json::Array(nodes)),
        ("edges".into(), Json::Array(edges)),
    ]))
}

/// The manifest's file list, if it is one: plain relative paths only, so a doctored manifest
/// cannot make a rerun prune anything outside the vault.
fn read_manifest(dir: &Path) -> HashSet<String> {
    let Ok(bytes) = fs::read(dir.join(MANIFEST_FILE)) else {
        return HashSet::new();
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return HashSet::new();
    };
    doc.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(Value::as_str)
                .filter(|f| safe_rel(f))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn safe_rel(f: &str) -> bool {
    f == SETTINGS_FILE
        || (!f.is_empty()
            && !f.contains(['/', '\\', '\0'])
            && !f.starts_with('.')
            && f.ends_with(".md"))
}

/// Write the vault into `dir`. Files the previous export owned (its manifest) are rewritten when
/// they changed or removed when gone from the graph; any other existing file is left alone and
/// reported. `graph.canvas` is always written.
pub fn write(g: &NxGraph, names: &Names, dir: &Path) -> Result<Written> {
    let vault = render(g, names);
    let owned_before = read_manifest(dir);
    fs::create_dir_all(dir.join(".obsidian"))
        .with_context(|| format!("creating {}", dir.display()))?;
    let mut done = Written::default();
    let mut owned = Vec::with_capacity(vault.files.len());
    let mut written_now: HashSet<String> = HashSet::new();
    for (rel, text) in &vault.files {
        let path = dir.join(rel);
        let exists = path.exists();
        if exists && (!owned_before.contains(rel) || written_now.contains(&rel.to_lowercase())) {
            done.skipped.push(rel.clone());
            continue;
        }
        if !exists || fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
            fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        }
        written_now.insert(rel.to_lowercase());
        if rel != SETTINGS_FILE {
            done.notes += 1;
        }
        owned.push(rel.clone());
    }
    let now: HashSet<&str> = owned.iter().map(String::as_str).collect();
    let mut stale: Vec<&String> = owned_before
        .iter()
        .filter(|f| !now.contains(f.as_str()))
        .collect();
    stale.sort();
    for f in stale {
        let path = dir.join(f);
        if path.is_file() && fs::remove_file(&path).is_ok() {
            done.pruned += 1;
        }
    }
    owned.sort();
    let manifest = Json::Object(vec![(
        "files".into(),
        Json::Array(owned.into_iter().map(Json::Str).collect()),
    )]);
    fs::write(dir.join(MANIFEST_FILE), pyjson::dumps_indent(&manifest))
        .with_context(|| format!("writing {}", dir.join(MANIFEST_FILE).display()))?;
    fs::write(dir.join(CANVAS_FILE), &vault.canvas)
        .with_context(|| format!("writing {}", dir.join(CANVAS_FILE).display()))?;
    Ok(done)
}

/// SHA-1 as lowercase hex (FIPS 180-4), for the long-name suffix.
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (t, word) in block.chunks(4).enumerate() {
            w[t] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for t in 16..80 {
            w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (t, &wt) in w.iter().enumerate() {
            let (f, k) = match t {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wt);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_known_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(&sha1_hex("y".repeat(300).as_bytes())[..8], "8214d8f1");
    }

    #[test]
    fn note_names_follow_graphify() {
        let cases = [
            (".promote()", "dot-promote()"),
            ("..x", "dot-x"),
            (". x", "dot- x"),
            (".", "unnamed"),
            ("...", "unnamed"),
            (".%", "unnamed"),
            ("%%", "unnamed"),
            ("_", "_"),
            ("x.", "x."),
            (" lead space", "lead space"),
            ("\tlead", "lead"),
            ("tab\there", "tab here"),
            ("a\u{7f}b", "a b"),
            ("../../etc/passwd", "dot-etcpasswd"),
            ("a/b\\c:d*e?f\"g<h>i|j", "abcdefghij"),
            ("[[wiki]] | #tag ^block", "wiki  tag block"),
            ("}) DETACH DELETE n //", "}) DETACH DELETE n"),
            ("a\u{2028}b", "a\u{2028}b"),
        ];
        for (label, want) in cases {
            assert_eq!(note_name(label), want, "{label:?}");
        }
        let long = note_name(&"y".repeat(300));
        assert_eq!(long, format!("{}_8214d8f1", "y".repeat(191)));
        // Cut at a character boundary of the UTF-8 encoding.
        assert_eq!(note_name(&"é".repeat(250)).chars().count(), 95 + 9);
    }

    #[test]
    fn yaml_escapes_like_graphify() {
        let mut s = String::new();
        yaml_str("q\"\\\n\0\u{1}\u{7f}\u{85}\u{2028}'", &mut s);
        assert_eq!(s, "\"q\\\"\\\\\\n\\0\\x01\\x7f\u{85}\\L'\"");
    }

    #[test]
    fn tags_keep_words_slashes_and_dashes() {
        assert_eq!(community_tag("My: \"Comm\" #1/x"), "My_Comm_1/x");
        assert_eq!(community_tag("a b\tc"), "a_bc");
        assert_eq!(community_tag("../evil"), "/evil");
        assert_eq!(community_tag("..."), "unnamed");
        assert_eq!(tag("code file"), "code_file");
    }
}
