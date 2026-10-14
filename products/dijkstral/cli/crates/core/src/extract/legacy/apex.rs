//! Apex (Salesforce): `.cls`, `.trigger` (§44.4).
//!
//! A line scanner. `class X` becomes `<file>_<x>` contained by the file and becomes the current
//! class (`interface` and `enum` nest in it without replacing it); `extends`/`implements` targets
//! become nodes `slug(Name)` of the file (INFERRED `extends` / `implements`; a dotted name keeps
//! its first segment). A line that starts with an access or member modifier (or `void`) and
//! names `name(` declares a method of the current class (`.name()`); after `@AuraEnabled` or
//! `@InvocableMethod` the file also `contains` it (INFERRED). `trigger X on Object` becomes the
//! trigger node using the object. SOQL `FROM Object` and DML statements (`insert`, `update`, …)
//! become `uses` (INFERRED) of nodes `slug(Object)` / `dml_<verb>`, one edge per occurrence.

use super::{def, file_node, is_word, Lines};
use crate::extract::{slug, Ctx};
use crate::facts::{Confidence, Loc};

const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "global",
    "static",
    "override",
    "virtual",
    "abstract",
    "final",
    "transient",
    "webservice",
    "testmethod",
];
const DML: &[&str] = &["insert", "update", "delete", "upsert", "merge", "undelete"];
const ENTRY: &[&str] = &["auraenabled", "invocablemethod"];

fn inferred(cx: &mut Ctx, source: &str, target: &str, relation: &str, line: u32) {
    let mut e = cx.edge(source, target, relation, Loc::Line(line), None);
    e.confidence = Confidence::Inferred;
    cx.facts.edges.push(e);
}

/// The word after `kw` (as a whole word, case-insensitive) in `line`.
fn word_after<'l>(line: &'l str, kw: &str) -> Option<(&'l str, &'l str)> {
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(kw) {
        let at = from + i;
        from = at + kw.len();
        let before_ok = lower[..at].chars().next_back().is_none_or(|c| !is_word(c));
        let rest = &line[at + kw.len()..];
        if !before_ok || !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim_start();
        let end = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
        if end > 0 {
            return Some((&rest[..end], &rest[end..]));
        }
    }
    None
}

/// A method declaration line: `[modifiers] [Type] name(`; the name.
fn method_name(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let first = t.split(|c: char| !is_word(c)).next()?.to_ascii_lowercase();
    if !(MODIFIERS.contains(&first.as_str()) || first == "void") {
        return None;
    }
    let paren = t.find('(')?;
    let head = t[..paren].trim_end();
    if head.contains(['=', ';', '{']) {
        return None;
    }
    let start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map(|(k, _)| k)?;
    let name = &head[start..];
    let lower = name.to_ascii_lowercase();
    if [
        "class",
        "interface",
        "enum",
        "new",
        "if",
        "for",
        "while",
        "return",
    ]
    .contains(&lower.as_str())
    {
        return None;
    }
    Some(name)
}

pub fn extract(cx: &mut Ctx) {
    file_node(cx, vec![]);
    let file = cx.file_id.clone();
    let text = String::from_utf8_lossy(cx.src).into_owned();
    let lines = Lines::new(&text);
    let mut current: Option<String> = None;
    let mut annotations: Vec<String> = Vec::new();
    let mut at = 0usize;
    for (i, raw) in text.split('\n').enumerate() {
        let line = i as u32 + 1;
        let offset = at;
        at += raw.len() + 1;
        let t = raw.trim();
        if let Some(a) = t.strip_prefix('@') {
            let name: String = a.chars().take_while(|c| is_word(*c)).collect();
            if name.len() == a.trim_end().len() || a[name.len()..].starts_with('(') {
                annotations.push(name.to_ascii_lowercase());
                continue;
            }
        }
        if let Some((name, rest)) =
            word_after(t, "trigger").filter(|_| t.to_ascii_lowercase().starts_with("trigger"))
        {
            let id = Ctx::child_id(&file, name);
            def(cx, &id, name, line, vec![]);
            cx.add_edge(&file, &id, "contains", line, None);
            if let Some((obj, _)) = word_after(rest, "on") {
                let oid = slug(obj);
                def(cx, &oid, obj, line, vec![]);
                inferred(cx, &id, &oid, "uses", line);
            }
            current = Some(id);
        } else if let Some((name, rest)) = word_after(t, "class") {
            let id = Ctx::child_id(&file, name);
            def(cx, &id, name, line, vec![]);
            cx.add_edge(&file, &id, "contains", line, None);
            if let Some((base, _)) = word_after(rest, "extends") {
                let bid = slug(base);
                def(cx, &bid, base, line, vec![]);
                inferred(cx, &id, &bid, "extends", line);
            }
            let lower = rest.to_ascii_lowercase();
            if let Some(k) = lower.find("implements") {
                let list = &rest[k + "implements".len()..];
                let list = &list[..list.find('{').unwrap_or(list.len())];
                let mut depth = 0;
                let mut item = String::new();
                let mut items = Vec::new();
                for c in list.chars() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        ',' if depth == 0 => items.push(std::mem::take(&mut item)),
                        c if depth == 0 => item.push(c),
                        _ => {}
                    }
                }
                items.push(item);
                for it in items {
                    let first = it.trim().split('.').next().unwrap_or("").trim().to_string();
                    let iid = slug(&first);
                    if iid.is_empty() {
                        continue;
                    }
                    def(cx, &iid, &first, line, vec![]);
                    inferred(cx, &id, &iid, "implements", line);
                }
            }
            current = Some(id);
            annotations.clear();
            continue;
        } else if let Some((name, _)) = word_after(t, "interface").or_else(|| word_after(t, "enum"))
        {
            let id = Ctx::child_id(&file, name);
            def(cx, &id, name, line, vec![]);
            let parent = current.clone().unwrap_or(file.clone());
            cx.add_edge(&parent, &id, "contains", line, None);
            annotations.clear();
            continue;
        } else if let (Some(name), Some(owner)) = (method_name(t), current.clone()) {
            let id = Ctx::child_id(&owner, name);
            def(cx, &id, &format!(".{name}()"), line, vec![]);
            cx.add_edge(&owner, &id, "method", line, None);
            if annotations.iter().any(|a| ENTRY.contains(&a.as_str())) {
                inferred(cx, &file, &id, "contains", line);
            }
        }
        annotations.clear();
        let Some(owner) = current.clone() else {
            continue;
        };
        // DML statement.
        let lower = t.to_ascii_lowercase();
        let verb = lower.split(|c: char| !is_word(c)).next().unwrap_or("");
        let operand = lower[verb.len()..].trim_start();
        if DML.contains(&verb)
            && lower[verb.len()..].starts_with(char::is_whitespace)
            && operand.starts_with(|c: char| is_word(c))
        {
            let id = format!("dml_{verb}");
            def(cx, &id, verb, line, vec![]);
            inferred(cx, &owner, &id, "uses", line);
        }
        // SOQL: `[SELECT … FROM Object`.
        let mut from = 0;
        while let Some(k) = raw[from..].find('[') {
            let open = from + k;
            from = open + 1;
            let inner = raw[open + 1..].trim_start();
            if !inner.to_ascii_lowercase().starts_with("select") {
                continue;
            }
            let close = raw[open..].find(']').map_or(raw.len(), |c| open + c);
            if let Some((obj, _)) = word_after(&raw[open..close], "from") {
                let oid = slug(obj);
                let oline = lines.line(offset + open);
                def(cx, &oid, obj, oline, vec![]);
                inferred(cx, &owner, &oid, "uses", oline);
            }
        }
    }
}
