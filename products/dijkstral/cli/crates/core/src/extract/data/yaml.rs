//! YAML (`.yaml`, `.yml`, §46): every mapping key becomes a node, like JSON manifests (§14.1), but
//! sequences are descended. Configuration files (known names, GitHub workflows) count whole; in
//! other files a document counts when it is a mapping that nests a collection or has a structural
//! key (`kind`, `jobs`, `services`…), and is neither an OpenAPI/Swagger spec nor a JSON Schema. A
//! file over 1 MiB yields nothing.
//!
//! The file is composed from parser events into a node graph (aliases share their anchor's node,
//! so they keep its lines), then checked the way a safe YAML 1.1 loader constructs it: a custom or
//! unknown tag, a collection used as a key, an impossible date or an ill-formed merge anywhere in
//! any document makes the whole file yield nothing. Keys are compared by their text, so `yes` and
//! `true` stay distinct and a repeated key keeps its first position but takes the last line and
//! value, as a Python dict does.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use crate::extract::{slug, Ctx};

/// Key nodes emitted per file, as for JSON (§14.1); document nodes do not count.
const MAX_KEYS: usize = 500;

/// Larger files yield nothing.
const MAX_BYTES: usize = 1 << 20;

/// Keys are emitted for mappings at most this many collections below the root (the document, or
/// the stream when several documents count).
const MAX_KEY_DEPTH: usize = 6;

/// Collections nested deeper than this (only reachable through an alias cycle) fail the file.
const MAX_DEPTH: usize = 400;

/// The YAML 1.1 core tag prefix (`!!`).
const CORE: &str = "tag:yaml.org,2002:";

/// Basenames (case-insensitive) whose documents all count, flat or not (probed).
const CONFIG_NAMES: &[&str] = &[
    "mkdocs.yml",
    "mkdocs.yaml",
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
    ".gitlab-ci.yml",
    ".gitlab-ci.yaml",
    "action.yml",
    "action.yaml",
    "pubspec.yaml",
    "chart.yaml",
    "chart.yml",
    "values.yaml",
    "values.yml",
    "kustomization.yaml",
    "kustomization.yml",
    ".travis.yml",
    ".pre-commit-config.yaml",
    "serverless.yml",
    "serverless.yaml",
    "environment.yml",
    "cloudbuild.yaml",
    "cloudbuild.yml",
    "azure-pipelines.yml",
    "appveyor.yml",
    "skaffold.yaml",
];

/// Top-level keys (case-insensitive) that make a flat mapping count outside configuration files.
const STRUCTURAL_KEYS: &[&str] = &[
    "apiversion",
    "dependencies",
    "functions",
    "hosts",
    "jobs",
    "kind",
    "metadata",
    "name",
    "networks",
    "on",
    "pipeline",
    "provider",
    "resources",
    "roles",
    "scripts",
    "services",
    "spec",
    "stages",
    "steps",
    "tasks",
    "version",
    "volumes",
    "workflows",
];

/// A configuration file: a [`CONFIG_NAMES`] basename, or anything below `.github/workflows`.
fn config_file(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    let (dirs, name) = lower.rsplit_once('/').unwrap_or(("", &lower));
    CONFIG_NAMES.contains(&name) || format!("/{dirs}/").contains("/.github/workflows/")
}

pub fn extract(cx: &mut Ctx) {
    if cx.src.len() > MAX_BYTES {
        return;
    }
    let Some(graph) = compose(cx.src) else {
        return;
    };
    if !graph.constructs() {
        return;
    }
    let config = config_file(cx.rel);
    let docs: Vec<usize> = graph
        .docs
        .iter()
        .copied()
        .filter(|&d| match graph.kind(d) {
            Kind::Map(_) | Kind::Seq(_) if config => true,
            _ => graph.qualifies(d),
        })
        .collect();
    if docs.is_empty() {
        return;
    }
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let mut budget = MAX_KEYS;
    if let [doc] = docs[..] {
        graph.walk(cx, doc, &file_id, None, &mut budget, 0);
        return;
    }
    for (i, &doc) in docs.iter().enumerate() {
        let label = graph
            .manifest_label(doc)
            .unwrap_or_else(|| format!("doc {i}"));
        let s = slug(&label);
        let id = format!("{file_id}_doc{i}_{s}");
        let at = graph.nodes[doc].line;
        cx.add_def(id.clone(), label, at, vec![]);
        cx.add_edge(&file_id, &id, "contains", at, None);
        graph.walk(cx, doc, &id, Some(&s), &mut budget, 1);
    }
}

#[derive(Debug)]
enum Kind {
    /// The scalar's text, and whether it was plain (unquoted, not a block scalar).
    Scalar(String, bool),
    Seq(Vec<usize>),
    /// `(key, value)` pairs in source order, before merges.
    Map(Vec<(usize, usize)>),
}

#[derive(Debug)]
struct YNode {
    kind: Kind,
    tag: Option<Tag>,
    line: u32,
}

/// The node graph of every document in a stream.
#[derive(Default)]
struct Graph {
    nodes: Vec<YNode>,
    docs: Vec<usize>,
}

/// Builds a [`Graph`] from parser events.
#[derive(Default)]
struct Composer {
    graph: Graph,
    /// Parser anchor id → node.
    anchors: HashMap<usize, usize>,
    /// Open collections; a mapping also holds its pending key.
    stack: Vec<(usize, Option<usize>)>,
    broken: bool,
}

impl Composer {
    fn push(&mut self, kind: Kind, anchor: usize, tag: Option<Tag>, mark: Marker) -> usize {
        let idx = self.graph.nodes.len();
        self.graph.nodes.push(YNode {
            kind,
            tag,
            line: mark.line() as u32,
        });
        if anchor > 0 {
            self.anchors.insert(anchor, idx);
        }
        idx
    }

    fn attach(&mut self, idx: usize) {
        let Some((parent, pending)) = self.stack.last_mut() else {
            self.graph.docs.push(idx);
            return;
        };
        let parent = *parent;
        match &mut self.graph.nodes[parent].kind {
            Kind::Seq(items) => items.push(idx),
            Kind::Map(pairs) => match pending.take() {
                None => *pending = Some(idx),
                Some(key) => pairs.push((key, idx)),
            },
            Kind::Scalar(..) => self.broken = true,
        }
    }
}

impl MarkedEventReceiver for Composer {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        match ev {
            Event::Scalar(value, style, anchor, tag) => {
                let plain = style == TScalarStyle::Plain;
                let idx = self.push(Kind::Scalar(value, plain), anchor, tag, mark);
                self.attach(idx);
            }
            Event::SequenceStart(anchor, tag) => {
                let idx = self.push(Kind::Seq(Vec::new()), anchor, tag, mark);
                self.attach(idx);
                self.stack.push((idx, None));
            }
            Event::MappingStart(anchor, tag) => {
                let idx = self.push(Kind::Map(Vec::new()), anchor, tag, mark);
                self.attach(idx);
                self.stack.push((idx, None));
            }
            Event::SequenceEnd | Event::MappingEnd => {
                self.stack.pop();
            }
            Event::Alias(anchor) => match self.anchors.get(&anchor) {
                Some(&idx) => self.attach(idx),
                None => self.broken = true,
            },
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
        }
    }
}

/// Every document of the stream, or `None` when the text is not valid YAML (or not text).
fn compose(src: &[u8]) -> Option<Graph> {
    let src = src.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(src);
    let text = std::str::from_utf8(src).ok()?;
    // Characters a YAML reader refuses outright (PyYAML's `NON_PRINTABLE`).
    if text.chars().any(|c| {
        !matches!(c, '\t' | '\n' | '\r' | ' '..='~' | '\u{85}' | '\u{A0}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
    }) {
        return None;
    }
    let mut composer = Composer::default();
    Parser::new_from_str(text).load(&mut composer, true).ok()?;
    (!composer.broken).then_some(composer.graph)
}

/// The tag a node is constructed with: its explicit tag (`!` alone means a string), or for a plain
/// scalar the first YAML 1.1 implicit type its text matches.
enum Resolved {
    Core(&'static str),
    Explicit(String),
    /// A local or foreign tag (`!Ref`, `!!python/name:…`): no safe constructor.
    Unknown,
}

fn resolve(node: &YNode) -> Resolved {
    match (&node.tag, &node.kind) {
        (Some(tag), _) if tag.handle.is_empty() && tag.suffix == "!" => Resolved::Core("str"),
        (Some(tag), _) if tag.handle == CORE => Resolved::Explicit(tag.suffix.clone()),
        (Some(tag), _) => match tag.suffix.strip_prefix(CORE) {
            Some(rest) if tag.handle.is_empty() => Resolved::Explicit(rest.to_string()),
            _ => Resolved::Unknown,
        },
        (None, Kind::Scalar(text, true)) => Resolved::Core(implicit(text)),
        (None, Kind::Scalar(..)) => Resolved::Core("str"),
        (None, Kind::Seq(_)) => Resolved::Core("seq"),
        (None, Kind::Map(_)) => Resolved::Core("map"),
    }
}

/// PyYAML's implicit resolvers, in their registration order.
fn implicit(text: &str) -> &'static str {
    static RES: OnceLock<[(&str, Regex); 6]> = OnceLock::new();
    let res = RES.get_or_init(|| {
        [
            ("bool", r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$"),
            ("float", r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$"),
            ("int", r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$"),
            ("null", r"^(?:~|null|Null|NULL|)$"),
            ("timestamp", r"^(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}(?:[Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?)$"),
            ("value", r"^=$"),
        ]
        .map(|(tag, re)| (tag, Regex::new(re).unwrap()))
    });
    if text == "<<" {
        return "merge";
    }
    res.iter()
        .find(|(_, re)| re.is_match(text))
        .map_or("str", |(tag, _)| tag)
}

impl Graph {
    /// Follows nothing: aliases already share their anchor's node.
    fn kind(&self, idx: usize) -> &Kind {
        &self.nodes[idx].kind
    }

    /// Whether every document constructs under a safe loader.
    fn constructs(&self) -> bool {
        let mut seen = vec![false; self.nodes.len()];
        self.docs.iter().all(|&d| self.check(d, &mut seen, 0))
    }

    fn check(&self, idx: usize, seen: &mut [bool], depth: usize) -> bool {
        if seen[idx] {
            return true;
        }
        if depth > MAX_DEPTH {
            return false;
        }
        seen[idx] = true;
        let node = &self.nodes[idx];
        let tag = match resolve(node) {
            Resolved::Unknown => return false,
            Resolved::Core(t) => t.to_string(),
            Resolved::Explicit(t) => t,
        };
        match (&node.kind, tag.as_str()) {
            (Kind::Scalar(text, _), t) => scalar_constructs(t, text),
            (Kind::Seq(items), "seq") => items.iter().all(|&i| self.check(i, seen, depth + 1)),
            (Kind::Seq(items), "omap" | "pairs") => items.iter().all(|&i| {
                matches!(self.kind(i), Kind::Map(p) if p.len() == 1)
                    && self.check(i, seen, depth + 1)
            }),
            (Kind::Map(_), "map" | "set") => {
                let Some(pairs) = self.flatten(idx, 0) else {
                    return false;
                };
                pairs.iter().all(|&(k, v)| {
                    matches!(self.kind(k), Kind::Scalar(..))
                        && self.check(k, seen, depth + 1)
                        && self.check(v, seen, depth + 1)
                })
            }
            _ => false,
        }
    }

    /// The mapping's pairs with merge keys (`<<`) expanded as a YAML 1.1 safe loader does: merged
    /// pairs first (from a sequence of mappings, the last one's first), then the mapping's own;
    /// `None` when a merge value is not a mapping or a sequence of mappings.
    fn flatten(&self, idx: usize, depth: usize) -> Option<Vec<(usize, usize)>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let Kind::Map(pairs) = self.kind(idx) else {
            return None;
        };
        let mut merged = Vec::new();
        let mut own = Vec::new();
        for &(k, v) in pairs {
            let is_merge = matches!(resolve(&self.nodes[k]), Resolved::Core("merge"))
                || matches!(resolve(&self.nodes[k]), Resolved::Explicit(ref t) if t == "merge");
            if !is_merge {
                own.push((k, v));
                continue;
            }
            match self.kind(v) {
                Kind::Map(_) => merged.extend(self.flatten(v, depth + 1)?),
                Kind::Seq(items) => {
                    let mut parts = Vec::new();
                    for &item in items {
                        if !matches!(self.kind(item), Kind::Map(_)) {
                            return None;
                        }
                        parts.push(self.flatten(item, depth + 1)?);
                    }
                    for part in parts.into_iter().rev() {
                        merged.extend(part);
                    }
                }
                Kind::Scalar(..) => return None,
            }
        }
        merged.extend(own);
        Some(merged)
    }

    /// `(text, key line, value)` per distinct key text, in first-seen order, last value winning.
    fn entries(&self, idx: usize) -> Vec<(&str, u32, usize)> {
        let mut out: Vec<(&str, u32, usize)> = Vec::new();
        let mut at: HashMap<&str, usize> = HashMap::new();
        for (k, v) in self.flatten(idx, 0).unwrap_or_default() {
            let Kind::Scalar(text, _) = self.kind(k) else {
                continue;
            };
            let entry = (text.as_str(), self.nodes[k].line, v);
            match at.get(text.as_str()) {
                Some(&i) => out[i] = entry,
                None => {
                    at.insert(text.as_str(), out.len());
                    out.push(entry);
                }
            }
        }
        out
    }

    /// Outside configuration files, a document counts when it is a mapping with a collection value
    /// or a [`STRUCTURAL_KEYS`] key, and neither an OpenAPI/Swagger spec (`openapi` or `swagger`,
    /// and `paths`) nor a JSON Schema (`$schema`, and `definitions`, `$defs` or `$id`); top-level
    /// keys compare case-insensitively.
    fn qualifies(&self, doc: usize) -> bool {
        if !matches!(self.kind(doc), Kind::Map(_)) {
            return false;
        }
        let entries = self.entries(doc);
        let keys: Vec<String> = entries.iter().map(|e| e.0.to_lowercase()).collect();
        let has = |k: &str| keys.iter().any(|x| x == k);
        if (has("openapi") || has("swagger")) && has("paths") {
            return false;
        }
        if has("$schema") && (has("definitions") || has("$defs") || has("$id")) {
            return false;
        }
        keys.iter().any(|k| STRUCTURAL_KEYS.contains(&k.as_str()))
            || entries
                .iter()
                .any(|&(_, _, v)| matches!(self.kind(v), Kind::Map(_) | Kind::Seq(_)))
    }

    /// The constructed string of a scalar, when it constructs to a non-empty `str`.
    fn string(&self, idx: usize) -> Option<&str> {
        let node = &self.nodes[idx];
        let Kind::Scalar(text, _) = &node.kind else {
            return None;
        };
        let is_str = match resolve(node) {
            Resolved::Core(t) => t == "str",
            Resolved::Explicit(t) => t == "str",
            Resolved::Unknown => false,
        };
        (is_str && !text.is_empty()).then_some(text.as_str())
    }

    /// `Kind name` (or `Kind`) for a Kubernetes-style document: `kind` a non-empty string, and
    /// `metadata.name` when it is one too.
    fn manifest_label(&self, doc: usize) -> Option<String> {
        let entries = self.entries(doc);
        let find = |entries: &[(&str, u32, usize)], key: &str| {
            entries.iter().find(|(k, _, _)| *k == key).map(|e| e.2)
        };
        let kind = self.string(find(&entries, "kind")?)?;
        let name = find(&entries, "metadata")
            .filter(|&m| matches!(self.kind(m), Kind::Map(_)))
            .and_then(|m| find(&self.entries(m), "name"))
            .and_then(|n| self.string(n));
        Some(match name {
            Some(name) => format!("{kind} {name}"),
            None => kind.to_string(),
        })
    }

    /// Emit the keys of a mapping `depth` collections below the root, in pre-order under
    /// `parent`. Ids use only the immediate parent key (the JSON quirk, §14.1); a sequence's
    /// mappings belong to the key that holds the sequence.
    fn walk(
        &self,
        cx: &mut Ctx,
        map: usize,
        parent: &str,
        parent_key: Option<&str>,
        budget: &mut usize,
        depth: usize,
    ) {
        if depth > MAX_KEY_DEPTH {
            return;
        }
        for (text, at, value) in self.entries(map) {
            let s = slug(text);
            if s.is_empty() {
                continue;
            }
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let id = match parent_key {
                None => format!("{}_{s}", cx.file_id),
                Some(ps) => format!("{}_{ps}_{s}", cx.file_id),
            };
            cx.add_def(id.clone(), text.to_string(), at, vec![]);
            // `items` under `items` has its parent's id: no self-loop.
            if id != parent {
                cx.add_edge(parent, &id, "contains", at, None);
            }
            self.descend(cx, value, &id, &s, budget, depth + 1);
        }
    }

    fn descend(
        &self,
        cx: &mut Ctx,
        value: usize,
        parent: &str,
        key: &str,
        budget: &mut usize,
        depth: usize,
    ) {
        if depth > MAX_KEY_DEPTH {
            return;
        }
        match self.kind(value) {
            Kind::Map(_) => self.walk(cx, value, parent, Some(key), budget, depth),
            Kind::Seq(items) => {
                for &item in items {
                    self.descend(cx, item, parent, key, budget, depth + 1);
                }
            }
            Kind::Scalar(..) => {}
        }
    }
}

/// Whether a scalar with this tag constructs: PyYAML's safe constructors fail on `=` and `<<`
/// outside a merge, on an impossible date, and on text an explicit `!!int`, `!!float`, `!!bool`,
/// `!!binary` or `!!timestamp` cannot read.
fn scalar_constructs(tag: &str, text: &str) -> bool {
    match tag {
        "str" | "null" => true,
        "bool" => matches!(
            text.to_ascii_lowercase().as_str(),
            "yes" | "no" | "true" | "false" | "on" | "off"
        ),
        "int" => int_constructs(text),
        "float" => float_constructs(text),
        "timestamp" => timestamp_constructs(text),
        "binary" => {
            let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(clean.as_bytes())
                .is_ok()
        }
        _ => false,
    }
}

fn int_constructs(text: &str) -> bool {
    let v = text.replace('_', "");
    let v = v.strip_prefix(['-', '+']).unwrap_or(&v);
    if v == "0" {
        return true;
    }
    if let Some(b) = v.strip_prefix("0b") {
        return !b.is_empty() && b.chars().all(|c| c == '0' || c == '1');
    }
    if let Some(h) = v.strip_prefix("0x") {
        return !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(o) = v.strip_prefix('0') {
        return o.chars().all(|c| ('0'..='7').contains(&c));
    }
    if v.contains(':') {
        return v
            .split(':')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    }
    !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())
}

fn float_constructs(text: &str) -> bool {
    let v = text.replace('_', "").to_ascii_lowercase();
    let v = v.strip_prefix(['-', '+']).unwrap_or(&v);
    if v == ".inf" || v == ".nan" {
        return true;
    }
    if v.contains(':') {
        return v
            .split(':')
            .all(|p| !p.is_empty() && p.parse::<f64>().is_ok());
    }
    v.parse::<f64>().is_ok()
}

fn timestamp_constructs(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"^([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})(?:(?:[Tt]|[ \t]+)([0-9]{1,2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]*))?(?:[ \t]*(Z|([-+])([0-9]{1,2})(?::([0-9]{2}))?))?)?$",
        )
        .unwrap()
    });
    let Some(c) = re.captures(text) else {
        return false;
    };
    let num = |i: usize| {
        c.get(i)
            .map(|m| m.as_str().parse::<u32>().unwrap_or(u32::MAX))
    };
    let (year, month, day) = (num(1).unwrap(), num(2).unwrap(), num(3).unwrap());
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if year == 0 || day == 0 || day > days {
        return false;
    }
    match (num(4), num(5), num(6)) {
        (Some(h), Some(m), Some(s)) => h < 24 && m < 60 && s < 60,
        _ => true,
    }
}

/// A value as Python's `yaml.safe_load` builds it, for callers that render it themselves
/// (markdown front matter, `docs/graphify-semantic.md` §3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum PyValue {
    None,
    Bool(bool),
    /// An `int`, as its decimal digits (Python ints are unbounded).
    Int(String),
    Float(f64),
    Str(String),
    /// A date, datetime, `bytes` or `set`, as Python's `str()` of it.
    Text(String),
    /// A list, or the list of `(key, value)` tuples of `!!omap` and `!!pairs`.
    List(Vec<PyValue>),
    /// A dict in insertion order; a repeated key (by Python equality: `1`, `1.0` and `true` are
    /// one key) keeps its first position and takes the last value.
    Dict(Vec<(PyValue, PyValue)>),
}

impl PyValue {
    /// Python's `str()`.
    pub fn py_str(&self) -> String {
        match self {
            PyValue::None => "None".into(),
            PyValue::Bool(b) => if *b { "True" } else { "False" }.into(),
            PyValue::Int(d) => d.clone(),
            PyValue::Float(x) => py_float_str(*x),
            PyValue::Str(s) | PyValue::Text(s) => s.clone(),
            PyValue::List(_) | PyValue::Dict(_) => self.py_repr(),
        }
    }

    /// Python's `repr()` (a tuple pair renders as a list; never reached by front matter keys).
    fn py_repr(&self) -> String {
        match self {
            PyValue::Str(s) => py_str_repr(s),
            PyValue::List(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(PyValue::py_repr)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PyValue::Dict(pairs) => format!(
                "{{{}}}",
                pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.py_repr(), v.py_repr()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => other.py_str(),
        }
    }

    /// The number a numeric value equals, for Python's `1 == 1.0 == True`.
    fn number(&self) -> Option<f64> {
        match self {
            PyValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            PyValue::Int(d) => d.parse().ok(),
            PyValue::Float(x) => Some(*x),
            _ => None,
        }
    }

    fn same_key(&self, other: &PyValue) -> bool {
        match (self.number(), other.number()) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self == other,
            _ => false,
        }
    }
}

/// `str(float)`: `repr` with `inf`, `-inf` and `nan`.
fn py_float_str(x: f64) -> String {
    if x.is_nan() {
        "nan".into()
    } else if x.is_infinite() {
        if x > 0.0 { "inf" } else { "-inf" }.into()
    } else {
        crate::pyjson::float_repr(x)
    }
}

/// `repr(str)`: single quotes unless the text has one and no double quote.
fn py_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `repr(bytes)`.
fn py_bytes_repr(b: &[u8]) -> String {
    let quote = if b.contains(&b'\'') && !b.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut out = format!("b{quote}");
    for &c in b {
        match c {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            c if c as char == quote => {
                out.push('\\');
                out.push(c as char);
            }
            0x20..=0x7e => out.push(c as char),
            c => out.push_str(&format!("\\x{c:02x}")),
        }
    }
    out.push(quote);
    out
}

/// `yaml.safe_load(text)`: the single document's value (`None` for an empty stream), or `Err`
/// when a safe loader raises (invalid YAML, several documents, a custom tag, an impossible date…).
#[allow(clippy::result_unit_err)]
pub fn safe_load(text: &str) -> Result<PyValue, ()> {
    let graph = compose(text.as_bytes()).ok_or(())?;
    if graph.docs.len() > 1 || !graph.constructs() {
        return Err(());
    }
    match graph.docs.first() {
        None => Ok(PyValue::None),
        Some(&doc) => graph.construct(doc, 0).ok_or(()),
    }
}

impl Graph {
    fn construct(&self, idx: usize, depth: usize) -> Option<PyValue> {
        if depth > MAX_DEPTH {
            return None;
        }
        let node = &self.nodes[idx];
        let tag = match resolve(node) {
            Resolved::Unknown => return None,
            Resolved::Core(t) => t.to_string(),
            Resolved::Explicit(t) => t,
        };
        let sub = |i: usize| self.construct(i, depth + 1);
        Some(match (&node.kind, tag.as_str()) {
            (Kind::Scalar(text, _), t) => construct_scalar(t, text)?,
            (Kind::Seq(items), "seq") => {
                PyValue::List(items.iter().map(|&i| sub(i)).collect::<Option<_>>()?)
            }
            (Kind::Seq(items), "omap" | "pairs") => PyValue::List(
                items
                    .iter()
                    .map(|&i| {
                        let Kind::Map(p) = self.kind(i) else {
                            return None;
                        };
                        let &(k, v) = p.first()?;
                        Some(PyValue::List(vec![sub(k)?, sub(v)?]))
                    })
                    .collect::<Option<_>>()?,
            ),
            (Kind::Map(_), "map" | "set") => {
                let mut pairs: Vec<(PyValue, PyValue)> = Vec::new();
                for (k, v) in self.flatten(idx, 0)? {
                    let (k, v) = (sub(k)?, sub(v)?);
                    match pairs.iter_mut().find(|(have, _)| have.same_key(&k)) {
                        Some(slot) => slot.1 = v,
                        None => pairs.push((k, v)),
                    }
                }
                if tag == "set" {
                    // Python orders a set by hash; insertion order is the stable stand-in.
                    let items: Vec<String> = pairs.iter().map(|(k, _)| k.py_repr()).collect();
                    PyValue::Text(if items.is_empty() {
                        "set()".into()
                    } else {
                        format!("{{{}}}", items.join(", "))
                    })
                } else {
                    PyValue::Dict(pairs)
                }
            }
            _ => return None,
        })
    }
}

fn construct_scalar(tag: &str, text: &str) -> Option<PyValue> {
    Some(match tag {
        "str" => PyValue::Str(text.to_string()),
        "null" => PyValue::None,
        "bool" => PyValue::Bool(matches!(
            text.to_ascii_lowercase().as_str(),
            "yes" | "true" | "on"
        )),
        "int" => PyValue::Int(construct_int(text)?),
        "float" => PyValue::Float(construct_float(text)?),
        "timestamp" => PyValue::Text(construct_timestamp(text)?),
        "binary" => {
            let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(clean.as_bytes())
                .ok()?;
            PyValue::Text(py_bytes_repr(&bytes))
        }
        _ => return None,
    })
}

/// YAML 1.1 ints (`0b`, `0x`, a leading `0` for octal, `:` for base 60, `_` ignored), as decimal
/// digits.
fn construct_int(text: &str) -> Option<String> {
    let v = text.replace('_', "");
    let (neg, v) = match v.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, v.strip_prefix('+').unwrap_or(&v).to_string()),
    };
    let (digits, base): (Vec<u32>, u32) = if v == "0" {
        (vec![0], 10)
    } else if let Some(b) = v.strip_prefix("0b") {
        (b.chars().map(|c| c.to_digit(2)).collect::<Option<_>>()?, 2)
    } else if let Some(h) = v.strip_prefix("0x") {
        (
            h.chars().map(|c| c.to_digit(16)).collect::<Option<_>>()?,
            16,
        )
    } else if v.contains(':') {
        (
            v.split(':')
                .map(|p| p.parse().ok())
                .collect::<Option<_>>()?,
            60,
        )
    } else if let Some(o) = v.strip_prefix('0') {
        (o.chars().map(|c| c.to_digit(8)).collect::<Option<_>>()?, 8)
    } else {
        (
            v.chars().map(|c| c.to_digit(10)).collect::<Option<_>>()?,
            10,
        )
    };
    // Arbitrary precision: little-endian base-10^9 limbs.
    let mut limbs: Vec<u64> = vec![0];
    for d in digits {
        let mut carry = u64::from(d);
        for limb in limbs.iter_mut() {
            let x = *limb * u64::from(base) + carry;
            *limb = x % 1_000_000_000;
            carry = x / 1_000_000_000;
        }
        while carry > 0 {
            limbs.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    }
    let mut out = format!("{}", limbs.last().copied().unwrap_or(0));
    for limb in limbs.iter().rev().skip(1) {
        out.push_str(&format!("{limb:09}"));
    }
    Some(if neg && out != "0" {
        format!("-{out}")
    } else {
        out
    })
}

fn construct_float(text: &str) -> Option<f64> {
    let v = text.replace('_', "").to_ascii_lowercase();
    let (neg, v) = match v.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, v.strip_prefix('+').unwrap_or(&v).to_string()),
    };
    let x = if v == ".inf" {
        f64::INFINITY
    } else if v == ".nan" {
        f64::NAN
    } else if v.contains(':') {
        v.split(':')
            .try_fold(0.0, |acc, p| p.parse::<f64>().ok().map(|x| acc * 60.0 + x))?
    } else {
        v.parse::<f64>().ok()?
    };
    Some(if neg { -x } else { x })
}

/// `str()` of the date or datetime a YAML 1.1 timestamp constructs (fractions to microseconds,
/// a zone as a fixed offset, `Z` as `+00:00`).
fn construct_timestamp(text: &str) -> Option<String> {
    if !timestamp_constructs(text) {
        return None;
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"^([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})(?:(?:[Tt]|[ \t]+)([0-9]{1,2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]*))?(?:[ \t]*(Z|([-+])([0-9]{1,2})(?::([0-9]{2}))?))?)?$",
        )
        .unwrap()
    });
    let c = re.captures(text)?;
    let num = |i: usize| {
        c.get(i)
            .map_or(0, |m| m.as_str().parse::<u32>().unwrap_or(0))
    };
    let mut out = format!("{:04}-{:02}-{:02}", num(1), num(2), num(3));
    if c.get(4).is_none() {
        return Some(out);
    }
    out.push_str(&format!(" {:02}:{:02}:{:02}", num(4), num(5), num(6)));
    if let Some(f) = c.get(7) {
        let mut micro: String = f.as_str().chars().take(6).collect();
        while micro.len() < 6 {
            micro.push('0');
        }
        if micro != "000000" {
            out.push('.');
            out.push_str(&micro);
        }
    }
    if let Some(z) = c.get(8) {
        if z.as_str() == "Z" {
            out.push_str("+00:00");
        } else {
            let sign = c.get(9).map_or("+", |m| m.as_str());
            out.push_str(&format!("{sign}{:02}:{:02}", num(10), num(11)));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use crate::extract::extract_file;
    use crate::facts::{FileFacts, Loc};
    use crate::lang::Lang;

    fn nodes(f: &FileFacts) -> Vec<(String, String, Loc)> {
        f.nodes
            .iter()
            .map(|n| (n.id.clone(), n.label.clone(), n.loc))
            .collect()
    }

    fn ids(f: &FileFacts) -> Vec<&str> {
        f.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    fn y(path: &str, src: &str) -> FileFacts {
        extract_file(Lang::Yaml, path, src.as_bytes())
    }

    #[test]
    fn safe_load_builds_python_values() {
        use super::{safe_load, PyValue};
        let s = |x: &str| PyValue::Str(x.into());
        let i = |x: &str| PyValue::Int(x.into());
        assert_eq!(
            safe_load("a: 0x1F\nb: 017\nc: 1:30\nd: 999999999999999999999999\ne: -0b101\n"),
            Ok(PyValue::Dict(vec![
                (s("a"), i("31")),
                (s("b"), i("15")),
                (s("c"), i("90")),
                (s("d"), i("999999999999999999999999")),
                (s("e"), i("-5")),
            ]))
        );
        // `1`, `1.0` and `true` are one key: first position, last value.
        assert_eq!(
            safe_load("1: a\n1.0: b\ntrue: c\n"),
            Ok(PyValue::Dict(vec![(i("1"), s("c"))]))
        );
        assert_eq!(
            safe_load("t: 2026-1-2T03:04:05.5Z\nd: 2026-01-02\n"),
            Ok(PyValue::Dict(vec![
                (
                    s("t"),
                    PyValue::Text("2026-01-02 03:04:05.500000+00:00".into())
                ),
                (s("d"), PyValue::Text("2026-01-02".into())),
            ]))
        );
        assert_eq!(safe_load(""), Ok(PyValue::None));
        assert_eq!(safe_load("x: !Ref y\n"), Err(()));
        assert_eq!(safe_load("a: 1\n---\nb: 2\n"), Err(()));
        assert_eq!(PyValue::Bool(true).py_str(), "True");
        assert_eq!(PyValue::Float(1e16).py_str(), "1e+16");
    }

    #[test]
    fn configuration_files_and_structural_keys() {
        assert_eq!(
            ids(&y("docs/MkDocs.yml", "INHERIT: x\n")),
            ["docs_mkdocs", "docs_mkdocs_inherit"]
        );
        assert_eq!(ids(&y("a/.github/workflows/ci/x.yml", "a: 1\n")).len(), 2);
        assert!(y("a/.github/x.yml", "a: 1\n").nodes.is_empty());
        // A configuration file counts sequences (no keys) but not scalars or empty documents.
        assert_eq!(ids(&y("mkdocs.yml", "- a\n")), ["mkdocs"]);
        assert!(y("mkdocs.yml", "just text\n").nodes.is_empty());
        let f = y(
            "values.yaml",
            "- a\n---\nb: 1\n---\nopenapi: 3\npaths: {}\n",
        );
        assert_eq!(
            ids(&f),
            [
                "values",
                "values_doc0_doc_0",
                "values_doc1_doc_1",
                "values_doc_1_b",
                "values_doc2_doc_2",
                "values_doc_2_openapi",
                "values_doc_2_paths"
            ]
        );
        // Elsewhere a flat mapping counts only with a structural key.
        assert_eq!(ids(&y("k.yaml", "KIND: x\n")), ["k", "k_kind"]);
        assert!(y("k.yaml", "image: x\n").nodes.is_empty());
        assert!(y("k.yaml", "openapi: 3\npaths: {}\nname: x\n")
            .nodes
            .is_empty());
    }

    #[test]
    fn only_documents_with_a_collection_value_count() {
        assert!(y("a.yaml", "a: 1\nb: 2\n").nodes.is_empty());
        assert!(y("a.yaml", "- a: 1\n").nodes.is_empty());
        assert!(y("a.yaml", "").nodes.is_empty());
        assert_eq!(ids(&y("a.yaml", "a: []\n")), ["a", "a_a"]);
        // A collection under a key with an empty slug still counts; the key is dropped.
        assert_eq!(ids(&y("a.yaml", "\"@/*\": {x: 1}\nb: 1\n")), ["a", "a_b"]);
    }

    #[test]
    fn keys_nest_under_the_immediate_parent_through_sequences() {
        let f = y(
            "conf/app.yaml",
            "s:\n  t:\n    u: 1\nl:\n  - k: 1\n  - - j: 2\n  - n:\n      x: 1\n",
        );
        assert_eq!(
            ids(&f),
            [
                "conf_app",
                "conf_app_s",
                "conf_app_s_t",
                "conf_app_t_u",
                "conf_app_l",
                "conf_app_l_k",
                "conf_app_l_j",
                "conf_app_l_n",
                "conf_app_n_x"
            ]
        );
    }

    #[test]
    fn a_key_with_its_parents_id_gets_no_self_loop() {
        let f = y(
            "n.yaml",
            "items:\n  - items:\n      - items:\n          - k: 1\n",
        );
        assert!(f.edges.iter().all(|e| e.source != e.target));
        assert!(f
            .edges
            .iter()
            .any(|e| e.source == "n_items_items" && e.target == "n_items_k"));
    }

    #[test]
    fn aliases_merges_and_repeated_keys() {
        let f = y(
            "m.yaml",
            "base: &b\n  x: 1\n  y: 2\nuse: *b\nm:\n  <<: *b\n  y: 3\ndup: 1\ndup:\n  z: 1\n",
        );
        let got = nodes(&f);
        let at = |id: &str| got.iter().find(|n| n.0 == id).map(|n| n.2);
        // An alias keeps its anchor's lines; merged keys come first; the last repeat wins.
        assert_eq!(at("m_use_x"), Some(Loc::Line(2)));
        assert_eq!(at("m_m_x"), Some(Loc::Line(2)));
        assert_eq!(at("m_m_y"), Some(Loc::Line(7)));
        assert_eq!(at("m_dup"), Some(Loc::Line(9)));
        assert_eq!(at("m_dup_z"), Some(Loc::Line(10)));
        let order: Vec<&str> = ids(&f)
            .into_iter()
            .filter(|i| i.starts_with("m_m_"))
            .collect();
        assert_eq!(order, ["m_m_x", "m_m_y"]);
    }

    #[test]
    fn labels_are_the_key_text() {
        let f = y("k.yaml", "n: {}\nyes: 1\n0x1F: 2\n\"x\\\"y\": 3\n~: 4\n");
        let labels: Vec<String> = f.nodes.iter().skip(1).map(|n| n.label.clone()).collect();
        assert_eq!(labels, ["n", "yes", "0x1F", "x\"y"]);
    }

    #[test]
    fn what_a_safe_loader_refuses_yields_nothing() {
        for src in [
            "n: {}\nt: !Ref x\n",
            "n: {}\n? {a: 1}\n: x\n",
            "n: {}\nd: 2019-02-30\n",
            "n: {}\nv: =\n",
            "n: {}\nm:\n  <<: 5\n",
            "n: {}\ni: !!int abc\n",
            "n: {}\np: !!python/name:os.path\n",
            "n: {}\n---\nt: !Sub x\n",
            "n: {}\nc: {{ .Values.x }}\n",
            "n: {}\n\tb: 2\n",
            "n: {}\nb: \u{7}\n",
        ] {
            assert!(y("a.yaml", src).nodes.is_empty(), "{src:?}");
        }
        for src in [
            "n: {}\ns: !!set {a, b}\n",
            "n: {}\no: !!omap [a: 1]\n",
            "n: {}\nb: !!binary aGVsbG8=\n",
            "n: {}\nq: !<tag:yaml.org,2002:str> x\n",
            "%YAML 1.1\n---\nn: {}\n",
            "\u{feff}n: {}\n",
        ] {
            assert!(!y("a.yaml", src).nodes.is_empty(), "{src:?}");
        }
    }

    #[test]
    fn several_counting_documents_get_document_nodes() {
        let f = y(
            "m.yaml",
            "kind: Pod\nmetadata:\n  name: p\n---\na: 1\n---\nkind: 5\nspec: {}\n---\nkind: Svc\nspec: []\n",
        );
        assert_eq!(
            nodes(&f)
                .iter()
                .filter(|n| n
                    .0
                    .strip_prefix("m_doc")
                    .is_some_and(|r| r.starts_with(char::is_numeric)))
                .map(|n| (n.0.as_str(), n.1.as_str(), n.2))
                .collect::<Vec<_>>(),
            [
                ("m_doc0_pod_p", "Pod p", Loc::Line(1)),
                ("m_doc1_doc_1", "doc 1", Loc::Line(7)),
                ("m_doc2_svc", "Svc", Loc::Line(10)),
            ]
        );
        assert!(ids(&f).contains(&"m_pod_p_metadata"));
        assert!(ids(&f).contains(&"m_doc_1_spec"));
        // One counting document: its keys hang off the file node.
        assert_eq!(
            ids(&y("o.yaml", "a: 1\n---\nn: {x: 1}\n")),
            ["o", "o_n", "o_n_x"]
        );
    }

    #[test]
    fn specs_schemas_depth_and_size() {
        assert!(y("o.yaml", "Openapi: 3\nPATHS:\n  /a: {}\n")
            .nodes
            .is_empty());
        assert!(!y("o.yaml", "openapi: 3\ninfo:\n  t: 1\n").nodes.is_empty());
        assert!(y("s.yaml", "$Schema: x\ndefinitions:\n  a: {}\n")
            .nodes
            .is_empty());
        assert!(!y("s.yaml", "$schema: x\nproperties:\n  a: {}\n")
            .nodes
            .is_empty());
        // Keys of mappings up to six collections below the document; one more level when several
        // documents count.
        let chain = "a:\n b:\n  c:\n   d:\n    e:\n     f:\n      g:\n       h: 1\n";
        assert_eq!(y("d.yaml", chain).nodes.last().unwrap().id, "d_f_g");
        let seq = "a:\n  - b:\n      - c:\n          - d:\n              - e: 1\n";
        assert_eq!(y("q.yaml", seq).nodes.last().unwrap().id, "q_c_d");
        let two = y("t.yaml", &format!("{chain}---\n{chain}"));
        assert_eq!(two.nodes.last().unwrap().id, "t_e_f");
        let pad = "x".repeat((1 << 20) - 10); // exactly 1 MiB with the rest
        assert!(!y("b.yaml", &format!("a: {{}}\np: {pad}\n"))
            .nodes
            .is_empty());
        assert!(y("b.yaml", &format!("a: {{}}\np: x{pad}\n"))
            .nodes
            .is_empty());
    }

    #[test]
    fn at_most_500_keys_per_file() {
        let body: Vec<String> = (0..600).map(|i| format!("  k{i:03}: 1")).collect();
        let f = y("b.yaml", &format!("top:\n{}\n", body.join("\n")));
        assert_eq!(f.nodes.len(), 501);
        assert_eq!(f.nodes.last().unwrap().id, "b_top_k498");
    }
}
