//! Dart (§27) extraction. See `docs/graphify-format.md`.
//!
//! Graphify reads Dart with regular expressions, not a grammar, and this module reproduces them:
//! each pass runs one pattern over the whole text (`\s` crosses lines, so a type on one line and
//! a name on the next still match), a match counts only when the line of its name sits at brace
//! depth 0 or 1 (braces counted naively per line, strings and comments included), and the edges
//! come out pass by pass:
//!
//! 1. `class`/`mixin`/`enum` declarations in source order: `defines` from the file, then
//!    `inherits` (`extends`, a mixin's `on`), `mixes_in` (`with`), `implements`, each to the
//!    type's leading `[\w.]` text (`jni$_.JObject` → `jni`, `_i1.Mock` kept whole), and
//!    `references` (no context) to the base name of each generic argument of the superclass
//!    (`extends State<BookList>`; `implements X<Y>` references nothing).
//! 2. `configures` from the declaration after a run of annotation lines to `@name`.
//! 3. `typedef X = T;`: `defines X` and `references` (context `typedef`) from `X` to `T`'s last
//!    segment, unless `T` is a function type.
//! 4. `extension X on T`: `defines X` and `extends` to `T`'s leading word, at the `on` line.
//! 5. "Variables": `TYPE NAME` followed by `=`, `;` or the end of a line (fields, top-level and
//!    function-level locals, getters `T get x =>`, `library x;`, `return x == …`, `as x;`):
//!    `defines NAME` and `references` (context `variable_type`) to the normalised type.
//! 6. "Methods": `TYPE [x.]name(` with a lowercase name, and `factory C.name(`: `defines`.
//! 7. `import`/`export` URIs (and `if (…) 'uri'` alternatives, context `conditional_uri`).
//! 8. `references` (context `type_lookup`) for every generic call `f<T>(` in the file.
//!
//! Every node a reference names (types, URIs, `@annotations`) is a shared node with a null
//! `source_file` and `source_location`; `crate::resolve::dart` binds types to definitions. File
//! nodes have a null `source_location`. A `part of 'lib.dart'` file defines its names under the
//! library's id prefix (made absolute by the resolver, quirk DA1), from the library file, adds
//! `includes` from the library and `contains` from itself.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{origin, slug, Ctx};
use crate::facts::{Attr, Edge, LangFacts, Loc, Node, NodeKind};

/// Prefix of part-file definition ids until the resolver makes them absolute.
pub const PART: &str = "@dartpart:";

/// Facts the cross-file pass (`crate::resolve::dart`) needs from one file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DartFacts {
    /// Type names referenced by this file's edges, by node id: (id, label).
    pub types: Vec<(String, String)>,
}

const TYPE: &str = r"[A-Za-z_][\w<>?,\[\]. ]*?";

/// A variable's type may run over lines (`DartX\n    get`).
const VAR_TYPE: &str = r"[A-Za-z_][\w<>?,\[\].\s]*?";

static VAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?m)^[ \t]*({VAR_TYPE})\s+([A-Za-z_]\w*)[ \t]*(?:=|;|$)"
    ))
    .unwrap()
});
static METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?m)^[ \t]*(?:(?:static|external|abstract|override)\s+)*(?:{TYPE})\s+(?:\w+\.)*([a-z_]\w*)\("
    ))
    .unwrap()
});
static FACTORY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:const\s+)?factory\s+\w+\.([A-Za-z_]\w*)\s*\(").unwrap()
});
static CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^[ \t]*(?:(?:abstract|base|final|sealed|interface|mixin)\s+)*class\s+(\w+)([^{;]*)",
    )
    .unwrap()
});
static MIXIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:base\s+)?mixin\s+(\w+)(?:\s*<[^{]*?>)?(?:\s+on\s+([^{]*?))?\s*\{")
        .unwrap()
});
static ENUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*enum\s+(\w+)").unwrap());
static EXT_TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*extension\s+type\s+(\w+)").unwrap());
static EXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*extension\s+(\w+)(?:\s*<[^{]*?>)?\s+on\s+(\w+)").unwrap()
});
static IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[ \t]*(import|export)\s+['"]([^'"]+)['"]([^;]*);"#).unwrap()
});
static COND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"if\s*\([^)]*\)\s*['"]([^'"]+)['"]"#).unwrap());
static PART_OF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^[ \t]*part\s+of\s+['"]([^'"]+)['"]"#).unwrap());
static TYPEDEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*typedef\s+(\w+)(?:<[^=;\n]*?>)?\s*=\s*([^;]+);").unwrap()
});
static LOOKUP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w<([\w.<>]+)>\(").unwrap());
static FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bFunction\b").unwrap());
static MODS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:late\s+final|late\s+var|late|final|const|var)\b\s*").unwrap()
});

/// Types that never produce a reference.
const BUILTIN: &[&str] = &[
    "Object", "String", "bool", "double", "dynamic", "int", "num", "void",
];

/// Method names the method pattern skips.
const KEYWORDS: &[&str] = &[
    "Function", "assert", "await", "case", "catch", "do", "else", "for", "get", "if", "new", "on",
    "return", "set", "super", "switch", "this", "throw", "try", "while",
];

/// Annotations that configure nothing.
const SKIPPED_ANNOTATIONS: &[&str] = &["override"];

fn is_builtin(t: &str) -> bool {
    BUILTIN.contains(&t)
}

/// Line starts and per-line brace depth.
struct Text<'a> {
    text: &'a str,
    starts: Vec<usize>,
    depth: Vec<i64>,
}

impl<'a> Text<'a> {
    fn new(text: &'a str) -> Text<'a> {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let mut depth = Vec::with_capacity(starts.len());
        let mut d: i64 = 0;
        for line in text.split('\n') {
            depth.push(d);
            let opens = line.matches('{').count() as i64;
            let closes = line.matches('}').count() as i64;
            d = (d + opens - closes).max(0);
        }
        Text {
            text,
            starts,
            depth,
        }
    }

    /// 1-based line of a byte offset.
    fn line(&self, pos: usize) -> u32 {
        self.starts.partition_point(|&s| s <= pos) as u32
    }

    fn shallow(&self, line: u32) -> bool {
        self.depth[line as usize - 1] <= 1
    }
}

fn last_segment(s: &str) -> &str {
    s.rsplit('.').next().unwrap_or(s)
}

/// Split a type list on top-level commas.
fn split_types(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let rest = s[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// The type a variable declaration references, normalised as Graphify does.
fn var_type(raw: &str) -> Option<String> {
    let t = MODS_RE.replace(raw.trim(), "");
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    let base = t.split_once('<').map_or(t, |(b, _)| b.trim());
    let ty = last_segment(base);
    (!ty.is_empty() && !matches!(ty, "as" | "show")).then(|| ty.to_string())
}

/// `(keyword, list)` clauses of a class header (`extends A with B, C implements D`).
fn clauses(header: &str) -> Vec<(&str, &str)> {
    let words = ["extends", "with", "implements"];
    let mut marks: Vec<(usize, &str)> = Vec::new();
    let bytes = header.as_bytes();
    for w in words {
        for (i, _) in header.match_indices(w) {
            let before = i == 0 || bytes[i - 1].is_ascii_whitespace();
            let after = bytes.get(i + w.len()).is_some_and(u8::is_ascii_whitespace);
            if before && after {
                marks.push((i, w));
            }
        }
    }
    marks.sort();
    marks
        .iter()
        .enumerate()
        .map(|(k, &(i, w))| {
            let end = marks.get(k + 1).map_or(header.len(), |m| m.0);
            (w, header[i + w.len()..end].trim())
        })
        .collect()
}

struct Walker<'a, 'b> {
    cx: &'a mut Ctx<'b>,
    /// Prefix of definition ids: the file id, or `PART` + the library's file id.
    prefix: String,
    /// Source of `defines`: this file, or the library a part belongs to.
    definer: String,
    /// Definitions in emission order, for `contains` from a part file.
    defined: Vec<(String, u32)>,
    seen: std::collections::HashSet<String>,
    edges_seen: std::collections::HashSet<(String, String, String)>,
    types: Vec<(String, String)>,
}

impl Walker<'_, '_> {
    /// An edge; a repeated (source, target, relation) keeps the first (quirk DA2).
    fn edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: u32,
        context: Option<&str>,
    ) {
        let key = (source.to_string(), target.to_string(), relation.to_string());
        if !self.edges_seen.insert(key) {
            return;
        }
        let mut e: Edge = self
            .cx
            .edge(source, target, relation, Loc::Line(line), context);
        e.score = Some(1.0);
        self.cx.facts.edges.push(e);
    }

    /// A definition; an id the file already defines keeps its first edge (quirk DA2).
    fn define(&mut self, name: &str, line: u32) -> String {
        let id = Ctx::child_id(&self.prefix, name);
        if !self.seen.insert(id.clone()) {
            return id;
        }
        self.cx
            .add_def(id.clone(), name.to_string(), line, Vec::new());
        let definer = self.definer.clone();
        self.edge(&definer, &id, "defines", line, None);
        self.defined.push((id.clone(), line));
        id
    }

    /// A shared node with null `source_file` and `source_location`.
    fn shared(&mut self, id: String, label: String, file_type: &str) -> Option<String> {
        if id.is_empty() {
            return None;
        }
        self.cx.facts.nodes.push(Node {
            id: id.clone(),
            label,
            kind: NodeKind::External,
            file_type: file_type.into(),
            source_file: String::new(),
            loc: Loc::Null,
            attrs: vec![origin(), ("source_file".into(), Attr::Null)],
        });
        Some(id)
    }

    fn type_ref(&mut self, label: &str) -> Option<String> {
        let id = slug(label);
        if id.is_empty() {
            return None;
        }
        self.types.push((id.clone(), label.to_string()));
        self.shared(id, label.to_string(), "code")
    }

    fn reference(&mut self, source: &str, label: &str, line: u32, context: Option<&str>) {
        if let Some(t) = self.type_ref(label) {
            self.edge(source, &t, "references", line, context);
        }
    }

    fn relate(&mut self, source: &str, label: &str, relation: &str, line: u32) {
        if let Some(t) = self.type_ref(label) {
            self.edge(source, &t, relation, line, None);
        }
    }
}

pub fn extract(cx: &mut Ctx) {
    cx.add_file_node(Vec::new());
    if let Some(n) = cx.facts.nodes.first_mut() {
        n.loc = Loc::Null;
    }
    let source = String::from_utf8_lossy(cx.src).into_owned();
    let t = Text::new(&source);
    let part_of = PART_OF_RE.captures(&source).map(|c| {
        let uri = c.get(1).unwrap().as_str();
        (library_id(cx.rel, uri), t.line(c.get(0).unwrap().start()))
    });
    let (prefix, definer) = match &part_of {
        Some((lib, _)) => (format!("{PART}{lib}"), lib.clone()),
        None => (cx.file_id.clone(), cx.file_id.clone()),
    };
    let mut w = Walker {
        cx,
        prefix,
        definer,
        defined: Vec::new(),
        seen: std::collections::HashSet::new(),
        edges_seen: std::collections::HashSet::new(),
        types: Vec::new(),
    };
    declarations(&mut w, &t);
    annotations(&mut w, &t);
    typedefs(&mut w, &t);
    extensions(&mut w, &t);
    variables(&mut w, &t);
    methods(&mut w, &t);
    imports(&mut w, &t);
    lookups(&mut w, &t);
    if let Some((lib, line)) = &part_of {
        let file = w.cx.file_id.clone();
        w.edge(lib, &file, "includes", *line, None);
        for (id, line) in std::mem::take(&mut w.defined) {
            w.edge(&file, &id, "contains", line, None);
        }
    }
    let types = std::mem::take(&mut w.types);
    w.cx.facts.lang = LangFacts::Dart(DartFacts { types });
}

/// The file id of the library a `part of 'uri'` names, relative to the part's directory.
fn library_id(rel: &str, uri: &str) -> String {
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in uri.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    let path = parts.join("/");
    crate::ids::file_id(&path)
}

fn declarations(w: &mut Walker, t: &Text) {
    let mut decls: Vec<(usize, u8, regex::Captures)> = Vec::new();
    for c in CLASS_RE.captures_iter(t.text) {
        decls.push((c.get(1).unwrap().start(), 0, c));
    }
    for c in MIXIN_RE.captures_iter(t.text) {
        decls.push((c.get(1).unwrap().start(), 1, c));
    }
    for c in ENUM_RE.captures_iter(t.text) {
        decls.push((c.get(1).unwrap().start(), 2, c));
    }
    decls.sort_by_key(|d| d.0);
    for (pos, kind, c) in decls {
        let line = t.line(pos);
        if !t.shallow(line) {
            continue;
        }
        let id = w.define(c.get(1).unwrap().as_str(), line);
        match (kind, c.get(2)) {
            (0, Some(header)) => {
                // Skip the type parameters: `class A<T extends B> extends C`.
                let mut header = header.as_str().trim_start();
                if header.starts_with('<') {
                    let mut depth = 0;
                    for (i, ch) in header.char_indices() {
                        match ch {
                            '<' => depth += 1,
                            '>' => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 {
                            header = &header[i + 1..];
                            break;
                        }
                    }
                }
                for (word, list) in clauses(header) {
                    let relation = match word {
                        "extends" => "inherits",
                        "with" => "mixes_in",
                        _ => "implements",
                    };
                    for ty in split_types(list) {
                        let base_len = ty
                            .char_indices()
                            .find(|&(_, ch)| !(ch.is_alphanumeric() || ch == '_' || ch == '.'))
                            .map_or(ty.len(), |(i, _)| i);
                        if base_len > 0 {
                            w.relate(&id, &ty[..base_len], relation, line);
                        }
                        if let (true, Some(open), Some(close)) =
                            (word == "extends", ty.find('<'), ty.rfind('>'))
                        {
                            if open < close {
                                for arg in split_types(&ty[open + 1..close]) {
                                    let base = arg.split('<').next().unwrap_or("").trim();
                                    let base = base.trim_end_matches('?');
                                    if !base.is_empty() && !is_builtin(base) {
                                        w.reference(&id, base, line, None);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            (1, Some(on)) => {
                for ty in split_types(on.as_str()) {
                    let base = ty.split('<').next().unwrap_or("").trim();
                    if !base.is_empty() {
                        w.relate(&id, base, "inherits", line);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A run of annotation lines configures the declaration on the next line.
fn annotations(w: &mut Walker, t: &Text) {
    let lines: Vec<&str> = t.text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with('@') {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].trim_start().starts_with('@') {
            i += 1;
        }
        let Some(next) = lines.get(i) else {
            break;
        };
        let target = if let Some(c) = CLASS_RE
            .captures(next)
            .or_else(|| EXT_TYPE_RE.captures(next))
        {
            Some(c.get(1).unwrap().as_str().to_string())
        } else {
            METHOD_RE
                .captures(next)
                .map(|c| c.get(1).unwrap().as_str().to_string())
        };
        let Some(name) = target else {
            continue;
        };
        if !t.shallow(i as u32 + 1) {
            continue;
        }
        let source = Ctx::child_id(&w.prefix, &name);
        for (k, line) in lines.iter().enumerate().take(i).skip(start) {
            let a = line.trim_start()[1..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>();
            if a.is_empty() || SKIPPED_ANNOTATIONS.contains(&a.as_str()) {
                continue;
            }
            let id = format!("annotation_{}", slug(&a));
            w.types.push((id.clone(), a.clone()));
            if let Some(target) = w.shared(id, format!("@{a}"), "concept") {
                w.edge(&source, &target, "configures", k as u32 + 1, None);
            }
        }
    }
}

fn typedefs(w: &mut Walker, t: &Text) {
    for c in TYPEDEF_RE.captures_iter(t.text) {
        let name = c.get(1).unwrap();
        let line = t.line(name.start());
        if !t.shallow(line) {
            continue;
        }
        let value = c.get(2).unwrap().as_str();
        if FUNCTION_RE.is_match(value) {
            continue;
        }
        let id = w.define(name.as_str(), line);
        let base = last_segment(value.trim().split('<').next().unwrap_or("").trim()).to_string();
        let word = !base.is_empty() && base.chars().all(|ch| ch.is_alphanumeric() || ch == '_');
        if word && !is_builtin(&base) {
            w.reference(&id, &base, line, Some("typedef"));
        }
    }
}

fn extensions(w: &mut Walker, t: &Text) {
    let mut found: Vec<(usize, regex::Captures)> = EXT_RE
        .captures_iter(t.text)
        .chain(EXT_TYPE_RE.captures_iter(t.text))
        .map(|c| (c.get(0).unwrap().start(), c))
        .collect();
    found.sort_by_key(|f| f.0);
    for (_, c) in found {
        let Some(on) = c.get(2) else {
            let name = c.get(1).unwrap();
            let line = t.line(name.start());
            if t.shallow(line) {
                w.define(name.as_str(), line);
            }
            continue;
        };
        let line = t.line(on.start());
        if !t.shallow(line) {
            continue;
        }
        let id = w.define(c.get(1).unwrap().as_str(), line);
        w.relate(&id, on.as_str(), "extends", line);
    }
}

fn variables(w: &mut Walker, t: &Text) {
    for c in VAR_RE.captures_iter(t.text) {
        let name = c.get(2).unwrap();
        let line = t.line(name.start());
        if !t.shallow(line) {
            continue;
        }
        w.define(name.as_str(), line);
        if let Some(ty) = var_type(c.get(1).unwrap().as_str()) {
            if !is_builtin(&ty) && !ty.ends_with(',') {
                let file = w.cx.file_id.clone();
                w.reference(&file, &ty, line, Some("variable_type"));
            }
        }
    }
}

fn methods(w: &mut Walker, t: &Text) {
    let mut found: Vec<(usize, &str)> = Vec::new();
    for c in FACTORY_RE.captures_iter(t.text) {
        let m = c.get(1).unwrap();
        found.push((m.start(), m.as_str()));
    }
    for c in METHOD_RE.captures_iter(t.text) {
        let m = c.get(1).unwrap();
        if !KEYWORDS.contains(&m.as_str()) {
            found.push((m.start(), m.as_str()));
        }
    }
    found.sort_by_key(|f| f.0);
    for (pos, name) in found {
        let line = t.line(pos);
        if t.shallow(line) {
            w.define(name, line);
        }
    }
}

fn imports(w: &mut Walker, t: &Text) {
    let file = w.cx.file_id.clone();
    for c in IMPORT_RE.captures_iter(t.text) {
        let relation = if &c[1] == "import" {
            "imports"
        } else {
            "exports"
        };
        let uri = c.get(2).unwrap();
        if let Some(target) = w.shared(slug(uri.as_str()), uri.as_str().to_string(), "code") {
            w.edge(&file, &target, relation, t.line(uri.start()), None);
        }
        let rest = c.get(3).unwrap();
        for alt in COND_RE.captures_iter(rest.as_str()) {
            let a = alt.get(1).unwrap();
            let line = t.line(rest.start() + a.start());
            if let Some(target) = w.shared(slug(a.as_str()), a.as_str().to_string(), "code") {
                w.edge(&file, &target, relation, line, Some("conditional_uri"));
            }
        }
    }
}

fn lookups(w: &mut Walker, t: &Text) {
    let file = w.cx.file_id.clone();
    for c in LOOKUP_RE.captures_iter(t.text) {
        let label = last_segment(c.get(1).unwrap().as_str()).to_string();
        if !is_builtin(&label) {
            let line = t.line(c.get(0).unwrap().start());
            w.reference(&file, &label, line, Some("type_lookup"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_types() {
        assert_eq!(
            var_type("late final ffi.Pointer<X>").as_deref(),
            Some("Pointer")
        );
        assert_eq!(var_type("String? get").as_deref(), Some("String? get"));
        assert_eq!(
            var_type("objc.NSString get").as_deref(),
            Some("NSString get")
        );
        assert_eq!(
            var_type("static const int").as_deref(),
            Some("static const int")
        );
        assert_eq!(var_type("final").as_deref(), None);
        assert_eq!(var_type("final Address").as_deref(), Some("Address"));
    }

    #[test]
    fn class_clauses() {
        assert_eq!(
            clauses("extends A<B> with C, D implements E"),
            [("extends", "A<B>"), ("with", "C, D"), ("implements", "E")]
        );
        assert_eq!(library_id("lib/src/a.dart", "../lib.dart"), "lib_lib");
    }
}
