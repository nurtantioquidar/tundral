//! `extract --postgres DSN`: a PostgreSQL schema as graph nodes (`docs/graphify-inputs.md` §3).
//!
//! The CLI reads the catalog ([`Catalog`]); this module turns it into Graphify's shapes: one
//! database node (`postgresql:/<host>/<db>`), a node per table, view and function, `contains`
//! from the database, `references` per foreign key and `reads_from` for every relation a view or
//! function body selects from. Locations are lines of the DDL listing Graphify extracts from
//! (tables, views, foreign keys, then functions; rule P4), which this module numbers without
//! writing it. Nothing here touches the network.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::facts::{Attr, Confidence, Edge, Loc, Node, NodeKind};
use crate::graph::Graph;
use crate::ids::normalize;

/// A table or view's schema and name, as stored (unquoted).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Relation {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub schema: String,
    pub name: String,
    /// `pg_get_viewdef(oid)`.
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    pub schema: String,
    pub table: String,
    /// The constraint name (orders keys of one table).
    pub name: String,
    pub ref_schema: String,
    pub ref_table: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub schema: String,
    pub name: String,
    /// Orders overloads.
    pub oid: u32,
    /// `prosrc`: the body as written.
    pub source: String,
}

/// User objects of one database (system schemas excluded): ordinary and partitioned tables,
/// views, foreign keys, and functions and procedures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub tables: Vec<Relation>,
    pub views: Vec<View>,
    pub foreign_keys: Vec<ForeignKey>,
    pub functions: Vec<Function>,
}

impl Catalog {
    /// Put every list in a fixed order, whatever order the server returned rows in.
    pub fn sort(&mut self) {
        self.tables.sort();
        self.views
            .sort_by(|a, b| (&a.schema, &a.name).cmp(&(&b.schema, &b.name)));
        self.foreign_keys
            .sort_by(|a, b| (&a.schema, &a.table, &a.name).cmp(&(&b.schema, &b.table, &b.name)));
        self.functions
            .sort_by(|a, b| (&a.schema, &a.name, a.oid).cmp(&(&b.schema, &b.name, b.oid)));
    }
}

/// A catalog and the database it came from, as named in the graph (never the credentials).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub host: String,
    pub database: String,
    pub catalog: Catalog,
}

impl Snapshot {
    /// `postgresql:/<host>/<database>`: the `source_file` of every node (rule P2).
    pub fn source_file(&self) -> String {
        format!("postgresql:/{}/{}", self.host, self.database)
    }
}

/// The host and database a DSN names, the way Graphify labels the graph (rule P2): from a
/// `postgres[ql]://` URL or `key=value` pairs, `localhost` and `db` when absent. Credentials,
/// port and options are dropped.
pub fn dsn_source(dsn: &str) -> (String, String) {
    let (mut host, mut db) = (String::new(), String::new());
    let dsn = dsn.trim();
    if let Some(rest) = dsn
        .strip_prefix("postgresql://")
        .or_else(|| dsn.strip_prefix("postgres://"))
    {
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        host = if let Some(v6) = hostport.strip_prefix('[') {
            v6.split(']').next().unwrap_or("").to_string()
        } else {
            hostport.split([':', ',']).next().unwrap_or("").to_string()
        };
        db = percent_decode(path);
    } else {
        for (k, v) in key_values(dsn) {
            match k.as_str() {
                "host" => host = v,
                "dbname" => db = v,
                _ => {}
            }
        }
    }
    if host.is_empty() {
        host = "localhost".into();
    }
    if db.is_empty() {
        db = "db".into();
    }
    (host, db)
}

/// `key=value` pairs of a libpq connection string (single-quoted values, `\` escapes).
fn key_values(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() {
                break;
            }
            key.push(c);
            chars.next();
        }
        if key.is_empty() {
            break;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.next() != Some('=') {
            break;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if chars.peek() == Some(&'\'') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '\'' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                chars.next();
                if c == '\\' {
                    value.extend(chars.next());
                } else {
                    value.push(c);
                }
            }
        }
        out.push((key, value));
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The nodes and edges of `snap`, in Graphify's extraction order.
pub fn facts(snap: &Snapshot) -> (Vec<Node>, Vec<Edge>) {
    let source = snap.source_file();
    let db_id = normalize(&source);
    let cat = &snap.catalog;
    let mut tables = cat.tables.clone();
    tables.sort();
    let mut views: Vec<&View> = cat.views.iter().collect();
    views.sort_by(|a, b| (&a.schema, &a.name).cmp(&(&b.schema, &b.name)));
    let mut fks: Vec<&ForeignKey> = cat.foreign_keys.iter().collect();
    fks.sort_by(|a, b| (&a.schema, &a.table, &a.name).cmp(&(&b.schema, &b.table, &b.name)));
    let mut funcs: Vec<&Function> = cat.functions.iter().collect();
    funcs.sort_by(|a, b| (&a.schema, &a.name, a.oid).cmp(&(&b.schema, &b.name, b.oid)));

    let object_id =
        |schema: &str, name: &str| format!("{db_id}_{}", normalize(&format!("{schema}_{name}")));
    // Relations a body can read, by (schema, name) and by bare name (rule P7).
    let mut relations: Vec<Relation> = tables.clone();
    relations.extend(views.iter().map(|v| Relation {
        schema: v.schema.clone(),
        name: v.name.clone(),
    }));
    relations.sort();
    let qualified: HashSet<(&str, &str)> = relations
        .iter()
        .map(|r| (r.schema.as_str(), r.name.as_str()))
        .collect();
    let mut bare: HashMap<&str, &str> = HashMap::new();
    for r in &relations {
        bare.entry(r.name.as_str()).or_insert(r.schema.as_str());
    }
    let resolve = |name: &QualifiedName| -> Option<String> {
        match name.parts.as_slice() {
            [n] => {
                if qualified.contains(&("public", n.as_str())) {
                    Some(object_id("public", n))
                } else {
                    bare.get(n.as_str()).map(|s| object_id(s, n))
                }
            }
            [.., s, n] => qualified
                .contains(&(s.as_str(), n.as_str()))
                .then(|| object_id(s, n)),
            [] => None,
        }
    };

    let mut nodes = vec![Node {
        id: db_id.clone(),
        label: snap.database.clone(),
        kind: NodeKind::File,
        file_type: "code".into(),
        source_file: source.clone(),
        loc: Loc::Null,
        attrs: Vec::new(),
    }];
    let mut edges = Vec::new();
    let node = |id: String, label: String, line: u32| Node {
        id,
        label,
        kind: NodeKind::Def,
        file_type: "code".into(),
        source_file: source.clone(),
        loc: Loc::Line(line),
        attrs: Vec::new(),
    };
    let edge = |s: &str, t: &str, relation: &str, line: u32| Edge {
        source: s.to_string(),
        target: t.to_string(),
        relation: relation.into(),
        confidence: Confidence::Extracted,
        score: None,
        context: None,
        source_file: source.clone(),
        loc: Loc::Line(line),
        weight: Some(1.0),
        attrs: Vec::new(),
    };
    let mut stubs: HashSet<String> = HashSet::new();
    let mut reads =
        |from: &str, body: &str, line: u32, nodes: &mut Vec<Node>, edges: &mut Vec<Edge>| {
            // Every read is emitted; a repeated one folds into the pair's link, the later location
            // winning (rule P6).
            for r in read_relations(body) {
                let target = resolve(&r).unwrap_or_else(|| {
                    let id = normalize(&r.raw);
                    if stubs.insert(id.clone()) {
                        nodes.push(Node {
                            id: id.clone(),
                            label: r.raw.clone(),
                            kind: NodeKind::Stub,
                            file_type: "code".into(),
                            source_file: String::new(),
                            loc: Loc::Empty,
                            attrs: vec![("origin_file".into(), Attr::Str(source.clone()))],
                        });
                    }
                    id
                });
                edges.push(edge(from, &target, "reads_from", line + r.line));
            }
        };

    let mut line: u32 = 1;
    for t in &tables {
        let id = object_id(&t.schema, &t.name);
        nodes.push(node(id.clone(), quoted(&t.schema, &t.name), line));
        edges.push(edge(&db_id, &id, "contains", line));
        line += 1;
    }
    for v in &views {
        let id = object_id(&v.schema, &v.name);
        nodes.push(node(id.clone(), quoted(&v.schema, &v.name), line));
        edges.push(edge(&db_id, &id, "contains", line));
        reads(&id, &v.definition, line, &mut nodes, &mut edges);
        line += 1 + newlines(&v.definition);
    }
    for fk in &fks {
        edges.push(edge(
            &object_id(&fk.schema, &fk.table),
            &object_id(&fk.ref_schema, &fk.ref_table),
            "references",
            line,
        ));
        line += 1;
    }
    let mut functions: HashSet<String> = HashSet::new();
    for f in &funcs {
        let id = object_id(&f.schema, &f.name);
        // An overload shares its first definition's node and `contains` link (rule P5).
        if functions.insert(id.clone()) {
            nodes.push(node(
                id.clone(),
                format!("{}()", quoted(&f.schema, &f.name)),
                line,
            ));
            edges.push(edge(&db_id, &id, "contains", line));
        }
        reads(&id, &f.source, line, &mut nodes, &mut edges);
        line += 1 + newlines(&f.source);
    }
    (nodes, edges)
}

/// Add `snap` to `graph`; returns `(nodes, edges)` as extracted.
pub fn apply(graph: &mut Graph, snap: &Snapshot) -> (usize, usize) {
    let (nodes, edges) = facts(snap);
    let counts = (nodes.len(), edges.len());
    let old = std::mem::take(graph);
    let mut all = old.nodes;
    all.extend(nodes);
    let mut merged = Graph::assemble_batches(all, vec![old.edges, edges]);
    merged.dangling += old.dangling;
    merged.hyperedges = old.hyperedges;
    *graph = merged;
    counts
}

fn quoted(schema: &str, name: &str) -> String {
    format!("\"{schema}\".\"{name}\"")
}

fn newlines(s: &str) -> u32 {
    s.bytes().filter(|&b| b == b'\n').count() as u32
}

/// A relation name as written in SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedName {
    /// The text as written (`pg_catalog.pg_proc`, `"App"."Odd"`): a stub's label.
    pub raw: String,
    /// Parts as the catalog stores them: unquoted parts lower-cased, quoted ones verbatim.
    pub parts: Vec<String>,
    /// Lines before it in the text.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Identifier or keyword; `quoted` for `"..."`.
    Word {
        text: String,
        quoted: bool,
    },
    Punct(char),
    /// A literal or anything else that only separates.
    Other,
}

fn tokenize(s: &str) -> Vec<(Tok, u32)> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 0u32);
    let count = |from: usize, to: usize, b: &[char]| {
        b[from..to].iter().filter(|&&c| c == '\n').count() as u32
    };
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '-' && b.get(i + 1) == Some(&'-') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            line += count(start, i, &b);
        } else if c == '\'' {
            let start = i;
            i += 1;
            while i < b.len() {
                if b[i] == '\'' {
                    if b.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            line += count(start, i, &b);
            out.push((Tok::Other, line));
        } else if c == '"' {
            let start_line = line;
            let start = i;
            let mut text = String::new();
            i += 1;
            while i < b.len() {
                if b[i] == '"' {
                    if b.get(i + 1) == Some(&'"') {
                        text.push('"');
                        i += 2;
                        continue;
                    }
                    break;
                }
                text.push(b[i]);
                i += 1;
            }
            i = (i + 1).min(b.len());
            line += count(start, i, &b);
            out.push((Tok::Word { text, quoted: true }, start_line));
        } else if c == '$' && dollar_tag(&b, i).is_some() {
            // A dollar-quoted string: skipped whole.
            let tag = dollar_tag(&b, i).unwrap_or_default();
            let start = i;
            i += tag.len();
            let close: Vec<char> = tag.chars().collect();
            while i < b.len() && b[i..].iter().take(close.len()).ne(close.iter()) {
                i += 1;
            }
            i = (i + close.len()).min(b.len());
            line += count(start, i, &b);
            out.push((Tok::Other, line));
        } else if c.is_alphabetic() || c == '_' {
            let mut text = String::new();
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '$') {
                text.push(b[i]);
                i += 1;
            }
            out.push((
                Tok::Word {
                    text,
                    quoted: false,
                },
                line,
            ));
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '.') {
                i += 1;
            }
            out.push((Tok::Other, line));
        } else {
            out.push((Tok::Punct(c), line));
            i += 1;
        }
    }
    out
}

/// `$tag$` at `i` (tag may be empty), when it opens a dollar-quoted string.
fn dollar_tag(b: &[char], i: usize) -> Option<String> {
    let mut j = i + 1;
    while j < b.len() && (b[j].is_alphanumeric() || b[j] == '_') {
        j += 1;
    }
    (b.get(j) == Some(&'$') && !b.get(i + 1).is_some_and(|c| c.is_ascii_digit()))
        .then(|| b[i..=j].iter().collect())
}

fn kw(t: &Tok, word: &str) -> bool {
    matches!(t, Tok::Word { text, quoted: false } if text.eq_ignore_ascii_case(word))
}

/// Words that end a FROM item or cannot be an alias.
const STOP: &[&str] = &[
    "where",
    "join",
    "inner",
    "left",
    "right",
    "full",
    "cross",
    "natural",
    "on",
    "using",
    "group",
    "order",
    "limit",
    "offset",
    "fetch",
    "for",
    "union",
    "intersect",
    "except",
    "having",
    "window",
    "returning",
    "set",
    "into",
    "lateral",
    "tablesample",
    "with",
    "select",
    "values",
    "then",
    "else",
    "end",
    "loop",
    "when",
    "and",
    "or",
    "as",
];

/// Words that open a parenthesis that is not a call (`IN (`, `EXISTS (`, `AS (`, …).
const NOT_CALLS: &[&str] = &[
    "as",
    "in",
    "exists",
    "from",
    "join",
    "on",
    "and",
    "or",
    "not",
    "select",
    "where",
    "with",
    "lateral",
    "any",
    "all",
    "some",
    "values",
    "using",
    "union",
    "intersect",
    "except",
    "then",
    "else",
    "when",
    "case",
    "return",
    "query",
    "into",
    "by",
    "having",
    "materialized",
    "loop",
    "if",
    "elsif",
    "while",
    "perform",
    "execute",
    "is",
    "between",
    "like",
    "set",
];

/// Relations a SQL text selects from (rule P6): every `FROM` list item and `JOIN` target that is
/// a name (not a subquery, a function call or a CTE), outside call parentheses (`EXTRACT(… FROM
/// …)`), except `DELETE FROM` targets and `IS DISTINCT FROM`.
pub fn read_relations(text: &str) -> Vec<QualifiedName> {
    let toks = tokenize(text);
    // CTE names: `name AS [NOT] [MATERIALIZED] (` and `name (cols) AS (`.
    let mut ctes: HashSet<String> = HashSet::new();
    for (i, (t, _)) in toks.iter().enumerate() {
        if let Tok::Word { text, quoted } = t {
            let mut j = i + 1;
            if matches!(toks.get(j), Some((Tok::Punct('('), _))) {
                let mut depth = 0;
                while let Some((t, _)) = toks.get(j) {
                    match t {
                        Tok::Punct('(') => depth += 1,
                        Tok::Punct(')') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                j += 1;
            }
            if toks.get(j).is_some_and(|(t, _)| kw(t, "as")) {
                let mut k = j + 1;
                while toks
                    .get(k)
                    .is_some_and(|(t, _)| kw(t, "not") || kw(t, "materialized"))
                {
                    k += 1;
                }
                if matches!(toks.get(k), Some((Tok::Punct('('), _))) {
                    ctes.insert(if *quoted {
                        text.clone()
                    } else {
                        text.to_lowercase()
                    });
                }
            }
        }
    }

    let mut out = Vec::new();
    // For each open parenthesis: whether it is a call's (`EXTRACT(year FROM d)`).
    let mut calls: Vec<bool> = Vec::new();
    // After `FROM` (a list) or `JOIN` (one item), until the item starts.
    let mut expect: Option<bool> = None;
    let mut i = 0;
    while i < toks.len() {
        let (t, _) = &toks[i];
        match t {
            Tok::Punct('(') => {
                let call = i > 0
                    && match &toks[i - 1].0 {
                        Tok::Word {
                            text,
                            quoted: false,
                        } => !NOT_CALLS.iter().any(|k| text.eq_ignore_ascii_case(k)),
                        Tok::Word { quoted: true, .. } => true,
                        _ => false,
                    };
                calls.push(call);
                if call {
                    expect = None;
                }
            }
            Tok::Punct(')') => {
                calls.pop();
                expect = None;
            }
            _ if calls.last() == Some(&true) => {}
            _ if kw(t, "from") => {
                let prev = i.checked_sub(1).map(|p| &toks[p].0);
                let skip = prev.is_some_and(|p| kw(p, "distinct") || kw(p, "delete"));
                expect = (!skip).then_some(true);
            }
            _ if kw(t, "join") => expect = Some(false),
            Tok::Word { .. } if expect.is_some() => {
                let list = expect.take().unwrap_or(false);
                if !(kw(t, "select") || kw(t, "with") || kw(t, "values")) {
                    i = from_list(&toks, i, list, &ctes, &mut out);
                    continue;
                }
            }
            _ => expect = None,
        }
        i += 1;
    }
    out
}

/// Read FROM items starting at `i`; returns where scanning resumes.
fn from_list(
    toks: &[(Tok, u32)],
    mut i: usize,
    list: bool,
    ctes: &HashSet<String>,
    out: &mut Vec<QualifiedName>,
) -> usize {
    loop {
        while toks
            .get(i)
            .is_some_and(|(t, _)| kw(t, "only") || kw(t, "lateral"))
        {
            i += 1;
        }
        let start = i;
        let Some((Tok::Word { text, quoted }, line)) = toks.get(i) else {
            return i;
        };
        if !*quoted && STOP.iter().any(|k| text.eq_ignore_ascii_case(k)) {
            return start;
        }
        let line = *line;
        let mut raw = Vec::new();
        let mut parts = Vec::new();
        while let Some((Tok::Word { text, quoted }, _)) = toks.get(i) {
            if *quoted {
                raw.push(format!("\"{}\"", text.replace('"', "\"\"")));
                parts.push(text.clone());
            } else {
                raw.push(text.clone());
                parts.push(text.to_lowercase());
            }
            i += 1;
            if matches!(toks.get(i), Some((Tok::Punct('.'), _)))
                && matches!(toks.get(i + 1), Some((Tok::Word { .. }, _)))
            {
                i += 1;
            } else {
                break;
            }
        }
        let call = matches!(toks.get(i), Some((Tok::Punct('('), _)));
        let cte = parts.len() == 1 && ctes.contains(&parts[0]);
        if !call && !cte {
            out.push(QualifiedName {
                raw: raw.join("."),
                parts,
                line,
            });
        }
        if call || !list {
            return i;
        }
        // Optional alias, then `,` continues the list.
        if toks.get(i).is_some_and(|(t, _)| kw(t, "as")) {
            i += 1;
        }
        if let Some((Tok::Word { text, quoted }, _)) = toks.get(i) {
            if *quoted || !STOP.iter().any(|k| text.eq_ignore_ascii_case(k)) {
                i += 1;
            }
        }
        if matches!(toks.get(i), Some((Tok::Punct(','), _))) {
            i += 1;
            continue;
        }
        return i;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(String, u32)> {
        read_relations(text)
            .into_iter()
            .map(|q| (q.raw, q.line))
            .collect()
    }

    #[test]
    fn reads_from_views_and_bodies() {
        let v = " SELECT o.id AS order_id,\n    sum((i.quantity * p.price_micros)) AS total_micros\n   FROM ((orders o\n     JOIN order_items i ON ((i.order_id = o.id)))\n     JOIN products p ON ((p.sku = i.sku)))\n  GROUP BY o.id;";
        assert_eq!(
            names(v),
            [
                ("orders".to_string(), 2),
                ("order_items".to_string(), 3),
                ("products".to_string(), 4)
            ]
        );
        let cte = " WITH recent AS (\n         SELECT events.id\n           FROM events\n        )\n SELECT id,\n    ( SELECT count(*) AS count\n           FROM alpha) AS n\n   FROM recent r,\n    zeta z\n  WHERE (z.id = r.id);";
        let got: Vec<String> = names(cte).into_iter().map(|(n, _)| n).collect();
        assert_eq!(got, ["events", "alpha", "zeta"]);
        let dml = "\nINSERT INTO t2 (id, v) SELECT id, v FROM app.t1;\nDELETE FROM t3 WHERE id < 0;\nUPDATE t2 SET v = 0 FROM t3 WHERE t3.id = t2.id;\n";
        assert_eq!(
            names(dml),
            [("app.t1".to_string(), 1), ("t3".to_string(), 3)]
        );
        let misc = "SELECT extract(year FROM at), x IS DISTINCT FROM y FROM \"App\".\"Odd\", generate_series(1, 3) g WHERE a IN (SELECT 1 FROM pg_class)";
        let got: Vec<String> = names(misc).into_iter().map(|(n, _)| n).collect();
        assert_eq!(got, ["\"App\".\"Odd\"", "pg_class"]);
        assert!(names("SELECT $1 + 1").is_empty());
        assert!(names("SELECT 'FROM x' -- FROM y\n /* FROM z */").is_empty());
    }

    #[test]
    fn dsn_sources() {
        let s = |d: &str| dsn_source(d);
        let pair = |h: &str, d: &str| (h.to_string(), d.to_string());
        assert_eq!(
            s("postgresql://u:p@127.0.0.1:55432/shop"),
            pair("127.0.0.1", "shop")
        );
        assert_eq!(
            s("postgres://u:p@localhost:5/shop?sslmode=disable"),
            pair("localhost", "shop")
        );
        assert_eq!(
            s("host=127.0.0.1 port=1 user=u password='a b' dbname=shop"),
            pair("127.0.0.1", "shop")
        );
        assert_eq!(s(""), pair("localhost", "db"));
        assert_eq!(s("postgresql:///x"), pair("localhost", "x"));
        assert_eq!(s("postgresql://[::1]:5/x"), pair("::1", "x"));
    }
}
