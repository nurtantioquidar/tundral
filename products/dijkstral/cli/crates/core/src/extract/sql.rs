//! SQL extraction (§45). See `docs/graphify-format.md`.
//!
//! Graphify reads `.sql` with derekstride's tree-sitter-sql 0.3.11 when its `[sql]` extra is
//! installed; without it every `.sql` file contributes nothing. dijkstral always extracts, as
//! Graphify 0.9.97 does with the extra (§45.1). One file yields, in this order:
//!
//! - its file node, with `source_location: null` even when the file has content;
//! - in a pre-order walk of the whole tree (function bodies and error nodes included): tables
//!   (`create_table`, `CREATE TABLE … AS` too), views (`create_view`, never materialized views),
//!   functions (`create_function`, label `name()`), triggers and named indexes, each
//!   `file_slug(label)` with `contains` from the file. A name defined twice in a file keeps its
//!   first node, which collects the edges of both definitions. Their edges:
//!   - `references` from a table to the object after each `REFERENCES` of a column definition or
//!     table constraint, at the table's line; an `ALTER TABLE` adds `references` from the table it
//!     names for each `ADD CONSTRAINT … REFERENCES`, at the `ALTER`'s line. A `REFERENCES` the
//!     grammar could not place (no column list: it sits in an error node) or of an
//!     `ADD COLUMN` gives nothing;
//!   - an `ALTER TABLE` names its table even without one: an undefined table gets its stub;
//!   - `reads_from` from a view or function to every relation in a `FROM` or `JOIN`, subqueries
//!     and CTE bodies included and CTE names excluded, at the relation's line;
//!   - `indexes` from an index and `triggers` from a trigger to its table, and `executes` from a
//!     trigger to its function when a function of that name (case-insensitively) is defined
//!     earlier in the file, all at the statement's line;
//! - a text pass over the raw source (S1): for each `CREATE TABLE <word> (`, every
//!   `REFERENCES <word>` up to the next line starting with `CREATE` or `ALTER` adds `references`
//!   to that word, at the table's line. So `REFERENCES s.t` also references `s`, and words in
//!   comments and strings count;
//! - only when the parse has an error anywhere, routines from the text with comments and one-line
//!   strings blanked (S2): this is how procedures, which the grammar cannot parse, and functions
//!   whose body it loses are read. Each `CREATE [OR REPLACE] FUNCTION|PROCEDURE name`, anywhere in
//!   a line (a dollar-quoted string's too), gives `name()` at its line unless that id is already
//!   defined, by the tree or an earlier routine; a skipped routine's body is skipped too. The body
//!   runs to the next line starting with `CREATE` or `ALTER`; in it, `writes_to` each
//!   `INSERT INTO`, `UPDATE`, `DELETE [FROM]` and `MERGE INTO` target and `reads_from` each
//!   `FROM`/`JOIN` target (not a `DELETE`'s, a CTE's, a function's, an `IS DISTINCT FROM`'s or
//!   one inside `EXTRACT(`, `SUBSTRING(`, `TRIM(` or `OVERLAY(` up to the first `)`), in text
//!   order at the keyword's line, each relation and name once (the first). A target whose slug
//!   is empty (`DELETE $$`) is dropped, as Graphify's build drops it.
//!
//! Names match case-insensitively, without quotes or backticks, schema included (`s.t` is not
//! `t`). A name defined in the file resolves to that definition wherever it is; any other name
//! becomes a stub `slug(name)` (label as written) in this file, which `crate::resolve::sql` binds
//! to the only definition elsewhere or folds onto one shared stub. Repeated edges are all
//! emitted (the build keeps the last). Top-level DML, schemas, types, sequences and grants give nothing.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node as TsNode, Tree};

use super::Ctx;
use crate::facts::{LangFacts, Loc};

/// Facts for the cross-file pass (`crate::resolve::sql`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SqlFacts {
    /// Definitions as `(key, local id)`.
    pub defs: Vec<(String, String)>,
    /// Stubs for names not defined in the file, as `(local id, key)`, in order of first use.
    pub stubs: Vec<(String, String)>,
}

/// S1: a table the text pass reads, its end, and the words it references.
static TEXT_TABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)CREATE\s+TABLE\s+(\w+)\s*\(").unwrap());
static TEXT_STOP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^(?:CREATE|ALTER)\b").unwrap());
static TEXT_REFERENCES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)REFERENCES\s+(\w+)").unwrap());
/// S2: a routine header anywhere in a line, its body's end, and the body's tables.
static ROUTINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bCREATE\s+(?:OR\s+REPLACE\s+)?(?:FUNCTION|PROCEDURE)\s+([^\s(]+)").unwrap()
});
static BODY_STOP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^[ \t]*(?:CREATE|ALTER)\b").unwrap());
/// Group 1 is the `FROM` of a `DELETE FROM`, which is not a read; group 2 the table.
static WRITES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(?:INSERT\s+INTO|UPDATE|DELETE(?:\s+(FROM))?|MERGE\s+INTO)\s+(\[?[\w$."]+\]?)"#,
    )
    .unwrap()
});
static READS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b(?:FROM|JOIN)\s+(\[?[\w$."]+\]?)"#).unwrap());
static CTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\w+)\s*(?:\([^)]*\))?\s+AS\s*\(").unwrap());
/// Text where a `FROM` reads nothing: `EXTRACT(YEAR FROM d)` up to the first `)`, so
/// `SUBSTRING(f(x) FROM t)` still reads `t`, and `IS DISTINCT FROM`.
static NOT_A_READ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:EXTRACT|SUBSTRING|TRIM|OVERLAY)\s*\([^)]*\)|\bDISTINCT\s+FROM\b").unwrap()
});
/// Words after a keyword that name no table (`UPDATE ONLY t` writes nothing).
const STOPWORDS: &[&str] = &["ONLY", "LATERAL", "SET", "DUAL"];
static COMMENTS_AND_STRINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/|--[^\n]*|'[^'\n]*'").unwrap());

/// A name as matched: quotes, backticks and whitespace dropped, lowercased.
fn key(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '"' | '`') && !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// One end of an edge: a definition of this file, or a name still to resolve.
enum End {
    Local(String),
    Name { key: String, label: String },
}

enum Item {
    Def {
        id: String,
        label: String,
        key: String,
        line: u32,
    },
    /// A name used without an edge, which still gets its stub.
    Use(End),
    Edge {
        source: End,
        relation: &'static str,
        target: End,
        line: u32,
    },
}

struct W<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    items: Vec<Item>,
    /// Functions walked so far, by key, for `executes`.
    functions: HashMap<String, String>,
}

pub fn extract(cx: &mut Ctx, tree: Option<&Tree>) {
    cx.add_file_node(Vec::new());
    if let Some(file) = cx.facts.nodes.first_mut() {
        file.loc = Loc::Null;
    }
    let Some(tree) = tree else {
        cx.facts.lang = LangFacts::Sql(SqlFacts::default());
        return;
    };
    let root = tree.root_node();
    let mut w = W {
        cx,
        items: Vec::new(),
        functions: HashMap::new(),
    };
    w.walk(root);
    w.text_references();
    if root.has_error() {
        w.routines();
    }
    let W { cx, items, .. } = w;
    emit(cx, items);
}

/// The id `e` names: a definition of this file, or its stub, created at first use.
fn resolve_end(
    cx: &mut Ctx,
    local: &HashMap<String, String>,
    nodes: &mut HashSet<String>,
    stubs: &mut Vec<(String, String)>,
    e: End,
) -> String {
    match e {
        End::Local(id) => id,
        End::Name { key, label } => match local.get(&key) {
            Some(id) => id.clone(),
            None => {
                let id = Ctx::child_id("", &label);
                if nodes.insert(id.clone()) {
                    cx.add_stub(id.clone(), label);
                    stubs.push((id.clone(), key));
                }
                id
            }
        },
    }
}

/// Turns the items into nodes and edges, resolving names against the file's definitions.
fn emit(cx: &mut Ctx, items: Vec<Item>) {
    let mut local: HashMap<String, String> = HashMap::new();
    let mut defs: Vec<(String, String)> = Vec::new();
    for item in &items {
        if let Item::Def { id, key, .. } = item {
            local.entry(key.clone()).or_insert_with(|| id.clone());
            if !defs.iter().any(|(k, i)| k == key && i == id) {
                defs.push((key.clone(), id.clone()));
            }
        }
    }
    let mut nodes: HashSet<String> = HashSet::new();
    let mut stubs: Vec<(String, String)> = Vec::new();
    let file = cx.file_id.clone();
    for item in items {
        match item {
            Item::Def {
                id,
                label,
                line,
                key: _,
            } => {
                if nodes.insert(id.clone()) {
                    cx.add_def(id.clone(), label, line, Vec::new());
                    cx.add_edge(&file, &id, "contains", line, None);
                }
            }
            Item::Use(name) => {
                resolve_end(cx, &local, &mut nodes, &mut stubs, name);
            }
            Item::Edge {
                source,
                relation,
                target,
                line,
            } => {
                let s = resolve_end(cx, &local, &mut nodes, &mut stubs, source);
                let t = resolve_end(cx, &local, &mut nodes, &mut stubs, target);
                cx.add_edge(&s, &t, relation, line, None);
            }
        }
    }
    cx.facts.lang = LangFacts::Sql(SqlFacts { defs, stubs });
}

fn children(node: TsNode) -> Vec<TsNode> {
    let mut c = node.walk();
    node.children(&mut c).collect()
}

fn first_child<'t>(node: TsNode<'t>, kind: &str) -> Option<TsNode<'t>> {
    children(node).into_iter().find(|c| c.kind() == kind)
}

/// The named node right after the first child of kind `after`, when it has kind `kind`.
fn after<'t>(node: TsNode<'t>, after: &[&str], kind: &str) -> Option<TsNode<'t>> {
    let kids = children(node);
    let i = kids.iter().position(|c| after.contains(&c.kind()))?;
    kids[i + 1..]
        .iter()
        .find(|c| c.is_named())
        .filter(|c| c.kind() == kind)
        .copied()
}

/// Every node of `node`'s subtree, `node` first, in pre-order.
fn preorder(node: TsNode) -> Vec<TsNode> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        out.push(n);
        let mut kids = children(n);
        kids.reverse();
        stack.extend(kids);
    }
    out
}

/// `text` with comments and one-line strings blanked to spaces, newlines kept, so offsets and lines
/// still match the source.
fn blank(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in COMMENTS_AND_STRINGS.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        for c in m.as_str().chars() {
            if c == '\n' {
                out.push('\n');
            } else {
                out.extend(std::iter::repeat_n(' ', c.len_utf8()));
            }
        }
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

fn line_at(text: &str, offset: usize) -> u32 {
    text[..offset].matches('\n').count() as u32 + 1
}

impl W<'_, '_> {
    fn text(&self, node: TsNode) -> String {
        self.cx.text(node).to_string()
    }

    fn name(&self, obj: TsNode) -> End {
        let label = self.text(obj);
        End::Name {
            key: key(&label),
            label,
        }
    }

    fn def(&mut self, label: String, line: u32) -> String {
        let id = Ctx::child_id(&self.cx.file_id, &label);
        let k = key(label.strip_suffix("()").unwrap_or(&label));
        self.items.push(Item::Def {
            id: id.clone(),
            label,
            key: k,
            line,
        });
        id
    }

    fn edge(&mut self, source: End, relation: &'static str, target: End, line: u32) {
        self.items.push(Item::Edge {
            source,
            relation,
            target,
            line,
        });
    }

    fn walk(&mut self, root: TsNode) {
        for node in preorder(root) {
            match node.kind() {
                "create_table" => self.table(node),
                "create_view" => self.view(node),
                "create_function" => self.function(node),
                "create_trigger" => self.trigger(node),
                "create_index" => self.index(node),
                "alter_table" => self.alter(node),
                _ => {}
            }
        }
    }

    /// The objects named after each `REFERENCES` under `node` whose ancestors up to `node` are
    /// one of `places` (nearest first).
    fn referenced(&self, node: TsNode, places: &[&[&str]]) -> Vec<End> {
        preorder(node)
            .into_iter()
            .filter(|n| n.kind() == "keyword_references")
            .filter(|n| {
                let mut kinds = Vec::new();
                let mut p = n.parent();
                while let Some(a) = p.filter(|a| *a != node) {
                    kinds.push(a.kind());
                    p = a.parent();
                }
                places.contains(&kinds.as_slice())
            })
            .filter_map(|n| {
                let mut s = n.next_named_sibling()?;
                while s.kind() == "comment" {
                    s = s.next_named_sibling()?;
                }
                (s.kind() == "object_reference").then(|| self.name(s))
            })
            .collect()
    }

    /// `reads_from` targets under `node`: relations in a `FROM` or `JOIN`, minus CTE names.
    fn reads(&self, node: TsNode) -> Vec<(End, u32)> {
        let all = preorder(node);
        let ctes: HashSet<String> = all
            .iter()
            .filter(|n| n.kind() == "cte")
            .filter_map(|n| first_child(*n, "identifier"))
            .map(|i| key(self.cx.text(i)))
            .collect();
        all.into_iter()
            .filter_map(|n| self.relation(n, &ctes))
            .collect()
    }

    fn relation(&self, n: TsNode, ctes: &HashSet<String>) -> Option<(End, u32)> {
        if n.kind() != "relation" || !matches!(n.parent()?.kind(), "from" | "join") {
            return None;
        }
        let obj = n
            .named_child(0)
            .filter(|c| c.kind() == "object_reference")?;
        let End::Name { key, label } = self.name(obj) else {
            return None;
        };
        (!ctes.contains(&key)).then(|| (End::Name { key, label }, Ctx::line(obj)))
    }

    fn table(&mut self, node: TsNode) {
        let Some(name) = first_child(node, "object_reference") else {
            return;
        };
        let line = Ctx::line(node);
        let id = self.def(self.text(name), line);
        let places: &[&[&str]] = &[
            &["column_definition", "column_definitions"],
            &["constraint", "constraints", "column_definitions"],
        ];
        for target in self.referenced(node, places) {
            self.edge(End::Local(id.clone()), "references", target, line);
        }
    }

    fn alter(&mut self, node: TsNode) {
        let Some(name) = first_child(node, "object_reference") else {
            return;
        };
        let line = Ctx::line(node);
        self.items.push(Item::Use(self.name(name)));
        for target in self.referenced(node, &[&["constraint", "add_constraint"]]) {
            self.edge(self.name(name), "references", target, line);
        }
    }

    fn view(&mut self, node: TsNode) {
        let Some(name) = first_child(node, "object_reference") else {
            return;
        };
        let id = self.def(self.text(name), Ctx::line(node));
        for (target, line) in self.reads(node) {
            self.edge(End::Local(id.clone()), "reads_from", target, line);
        }
    }

    fn function(&mut self, node: TsNode) {
        let Some(name) = first_child(node, "object_reference") else {
            return;
        };
        let label = self.text(name);
        let id = self.def(format!("{label}()"), Ctx::line(node));
        self.functions.entry(key(&label)).or_insert(id.clone());
        let body = first_child(node, "function_body").unwrap_or(node);
        for (target, line) in self.reads(body) {
            self.edge(End::Local(id.clone()), "reads_from", target, line);
        }
    }

    fn trigger(&mut self, node: TsNode) {
        let Some(name) = first_child(node, "object_reference") else {
            return;
        };
        let line = Ctx::line(node);
        let id = self.def(self.text(name), line);
        if let Some(table) = after(node, &["keyword_on"], "object_reference") {
            self.edge(End::Local(id.clone()), "triggers", self.name(table), line);
        }
        let function = after(
            node,
            &["keyword_function", "keyword_procedure"],
            "object_reference",
        );
        if let Some(f) = function.and_then(|f| self.functions.get(&key(self.cx.text(f)))) {
            let f = f.clone();
            self.edge(End::Local(id), "executes", End::Local(f), line);
        }
    }

    fn index(&mut self, node: TsNode) {
        let Some(name) = node
            .child_by_field_name("column")
            .filter(|n| n.kind() == "identifier")
        else {
            return;
        };
        let line = Ctx::line(node);
        let id = self.def(self.text(name), line);
        if let Some(table) = after(node, &["keyword_on"], "object_reference") {
            self.edge(End::Local(id), "indexes", self.name(table), line);
        }
    }

    /// S1.
    fn text_references(&mut self) {
        let src = String::from_utf8_lossy(self.cx.src).into_owned();
        let lines: HashMap<String, u32> = self
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Def { key, line, .. } => Some((key.clone(), *line)),
                _ => None,
            })
            .rev()
            .collect();
        for table in TEXT_TABLE.captures_iter(&src) {
            let whole = table.get(0).unwrap();
            let end = TEXT_STOP
                .find_at(&src, whole.end())
                .map_or(src.len(), |m| m.start());
            let name = &table[1];
            let line = lines
                .get(&key(name))
                .copied()
                .unwrap_or_else(|| line_at(&src, whole.start()));
            for word in TEXT_REFERENCES.captures_iter(&src[whole.start()..end]) {
                let source = End::Name {
                    key: key(name),
                    label: name.to_string(),
                };
                let target = End::Name {
                    key: key(&word[1]),
                    label: word[1].to_string(),
                };
                self.edge(source, "references", target, line);
            }
        }
    }

    /// S2, over the source with comments and one-line strings blanked.
    fn routines(&mut self) {
        let clean = blank(&String::from_utf8_lossy(self.cx.src));
        let mut done: HashSet<String> = self
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Def { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        for header in ROUTINE.captures_iter(&clean) {
            let whole = header.get(0).unwrap();
            let name = &header[1];
            let line = line_at(&clean, whole.start());
            let id = Ctx::child_id(&self.cx.file_id, &format!("{name}()"));
            if !done.insert(id.clone()) {
                continue;
            }
            self.def(format!("{name}()"), line);
            let end = BODY_STOP
                .find_at(&clean, whole.end())
                .map_or(clean.len(), |m| m.start());
            let body = &clean[whole.end()..end];
            let ctes: HashSet<String> = CTE.captures_iter(body).map(|c| key(&c[1])).collect();
            let not_reads: Vec<(usize, usize)> = NOT_A_READ
                .find_iter(body)
                .map(|m| (m.start(), m.end()))
                .collect();
            let mut events: Vec<(usize, &'static str, &str)> = Vec::new();
            let mut deletes: HashSet<usize> = HashSet::new();
            for w in WRITES.captures_iter(body) {
                let (at, target) = (w.get(0).unwrap().start(), w.get(2).unwrap());
                if let Some(from) = w.get(1) {
                    deletes.insert(from.start());
                }
                if !STOPWORDS.contains(&target.as_str().to_uppercase().as_str()) {
                    events.push((at, "writes_to", target.as_str()));
                }
            }
            for r in READS.captures_iter(body) {
                let (keyword, target) = (r.get(0).unwrap(), r.get(1).unwrap());
                let word = target.as_str();
                if deletes.contains(&keyword.start())
                    || not_reads
                        .iter()
                        .any(|&(s, e)| (s..e).contains(&keyword.start()))
                    || body[target.end()..].starts_with('(')
                    || ctes.contains(&key(word))
                    || STOPWORDS.contains(&word.to_uppercase().as_str())
                {
                    continue;
                }
                events.push((keyword.start(), "reads_from", word));
            }
            events.sort_by_key(|e| e.0);
            let mut seen: HashSet<(&str, String)> = HashSet::new();
            for (at, relation, word) in events {
                let label = word
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string();
                if Ctx::child_id("", &label).is_empty() || !seen.insert((relation, key(&label))) {
                    continue;
                }
                let target = End::Name {
                    key: key(&label),
                    label,
                };
                let line = line_at(&clean, whole.end() + at);
                self.edge(End::Local(id.clone()), relation, target, line);
            }
        }
    }
}
