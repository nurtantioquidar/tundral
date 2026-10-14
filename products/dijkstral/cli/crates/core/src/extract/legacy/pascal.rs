//! Pascal / Delphi / Free Pascal: `.pas .dpr .pp .inc .lpr .dpk` (§44.1).
//!
//! A scanner over the text with comments and strings blanked. `unit|program|library X;` becomes
//! `<file>_<x>` (`.dpk` and include files have none: their routines hang off the file node).
//! Every `uses` clause imports its units (bound to the unit's file, else an external).
//! `T = class(...)` / `interface(...)` declarations become types contained by the unit, with
//! their declared methods (`Name()`, `method`) and `inherits` edges to their bases (a local type,
//! a type of a used unit, else a node `slug(Base)` of this file). Routines defined in the
//! implementation section (the whole file when there is none) become `Name()` nodes.
//!
//! Calls: the identifiers between a routine's first `begin` and the next `end;` that name a
//! routine, resolved to the caller's class when it declares that name exactly once, else to the
//! only declaration of that name in the file; a method whose call finds neither looks through
//! its base classes in the cross-file pass. Self-calls are dropped. Graphify locates a call at the
//! routine's header line plus the lines between `begin` and the call (quirk PAS1, reproduced).

use serde::{Deserialize, Serialize};

use super::{def, file_node, is_word, words, LegacyFacts, Lines};
use crate::extract::{slug, Ctx};

/// Prefix of the virtual edge targets the cross-file pass redirects.
pub const VIRTUAL: &str = "@pas:";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PascalFacts {
    /// The unit, program or library name (empty when there is none).
    pub unit: String,
    /// `uses` entries: (unit name, virtual target of the `imports` edge).
    pub uses: Vec<(String, String)>,
    pub types: Vec<PasType>,
    /// Base types that are not declared in the file.
    pub bases: Vec<PasBase>,
    /// Calls left for the base classes of the caller's class.
    pub pending: Vec<PasCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasType {
    pub name: String,
    pub id: String,
    pub bases: Vec<String>,
    /// (name, method id), one per declaration.
    pub methods: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasBase {
    pub target: String,
    pub name: String,
    /// The file's own node for the base, used when no other file declares it.
    pub node: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasCall {
    pub caller: String,
    /// The calling method's class.
    pub class: String,
    pub name: String,
    /// The line Graphify reports (0-based).
    pub line: u32,
}

/// Comments (`{…}`, `(*…*)`, `//…`) and string literals replaced by spaces, newlines kept, so
/// byte offsets and lines are unchanged.
fn blank(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let fill = |out: &mut String, c: char| {
        if c == '\n' {
            out.push('\n');
        } else {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        }
    };
    while let Some((_, c)) = chars.next() {
        match c {
            '{' => {
                fill(&mut out, c);
                for (_, d) in chars.by_ref() {
                    fill(&mut out, d);
                    if d == '}' {
                        break;
                    }
                }
            }
            '(' if chars.peek().map(|p| p.1) == Some('*') => {
                fill(&mut out, c);
                let mut prev = ' ';
                for (_, d) in chars.by_ref() {
                    fill(&mut out, d);
                    if prev == '*' && d == ')' {
                        break;
                    }
                    prev = d;
                }
            }
            '/' if chars.peek().map(|p| p.1) == Some('/') => {
                fill(&mut out, c);
                while let Some(&(_, d)) = chars.peek() {
                    if d == '\n' {
                        break;
                    }
                    fill(&mut out, d);
                    chars.next();
                }
            }
            '\'' => {
                out.push('\'');
                for (_, d) in chars.by_ref() {
                    if d == '\'' || d == '\n' {
                        out.push(d);
                        break;
                    }
                    fill(&mut out, d);
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// The first word of `s` (case kept) and the rest.
fn first_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s.find(|c: char| !is_word(c)).unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// `[class] procedure|function|constructor|destructor Name` at the start of a line.
fn routine_header(line: &str) -> Option<&str> {
    let (mut w, mut rest) = first_word(line);
    if w.eq_ignore_ascii_case("class") {
        (w, rest) = first_word(rest);
    }
    let kw = w.to_ascii_lowercase();
    if !matches!(
        kw.as_str(),
        "procedure" | "function" | "constructor" | "destructor"
    ) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(is_word(c) || c == '.'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

struct Decl {
    name: String,
    id: String,
    /// Owning class name for methods.
    class: Option<String>,
}

pub fn extract(cx: &mut Ctx) -> LegacyFacts {
    file_node(cx, vec![]);
    let raw = String::from_utf8_lossy(cx.src).into_owned();
    let clean = blank(&raw);
    let lines = Lines::new(&clean);
    let file = cx.file_id.clone();
    let mut facts = PascalFacts::default();
    let mut serial = 0;
    let mut virt = |rel: &str| {
        serial += 1;
        format!("{VIRTUAL}{rel}|{serial}")
    };

    // Line starts of the text, for line-wise scanning.
    let line_texts: Vec<(usize, &str)> = {
        let mut out = Vec::new();
        let mut at = 0;
        for l in clean.split('\n') {
            out.push((at, l));
            at += l.len() + 1;
        }
        out
    };

    // unit / program / library.
    let mut container = file.clone();
    for (at, l) in &line_texts {
        let (w, rest) = first_word(l);
        let kw = w.to_ascii_lowercase();
        if matches!(kw.as_str(), "unit" | "program" | "library") {
            let rest = rest.trim_start();
            let end = rest
                .find(|c: char| !(is_word(c) || c == '.'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if !name.is_empty() && rest[end..].trim_start().starts_with(';') {
                let id = Ctx::child_id(&file, name);
                let line = lines.line(*at);
                def(cx, &id, name, line, vec![]);
                cx.add_edge(&file, &id, "contains", line, None);
                facts.unit = name.to_string();
                container = id;
            }
            break;
        }
        if !w.is_empty() {
            break;
        }
    }

    // uses clauses.
    for (off, w) in words(&clean) {
        if !w.eq_ignore_ascii_case("uses") {
            continue;
        }
        let rest = &clean[off + w.len()..];
        let list = &rest[..rest.find(';').unwrap_or(rest.len())];
        let line = lines.line(off);
        for item in list.split(',') {
            let item = item.trim_start();
            let end = item
                .find(|c: char| !(is_word(c) || c == '.'))
                .unwrap_or(item.len());
            let name = &item[..end];
            if name.is_empty() {
                continue;
            }
            let target = virt(cx.rel);
            cx.add_edge(&container, &target, "imports", line, Some("import"));
            facts.uses.push((name.to_string(), target));
        }
    }

    // Types and their declared methods.
    let mut decls: Vec<Decl> = Vec::new();
    let mut in_body = vec![false; line_texts.len()];
    let mut i = 0;
    while i < line_texts.len() {
        let (at, l) = line_texts[i];
        let Some((name, kind_rest)) = type_decl(l) else {
            i += 1;
            continue;
        };
        let line = lines.line(at);
        let id = Ctx::child_id(&file, name);
        if def(cx, &id, name, line, vec![]) {
            cx.add_edge(&container, &id, "contains", line, None);
        }
        // A forward declaration (`TFoo = class;`) opens a body like any other: everything up to
        // the next `end;` is read as its members (quirk PAS2, reproduced).
        let bases = bases(kind_rest);
        // A type declared again (after a forward declaration) keeps the first declaration's
        // entry: its later bases get edges but are never bound across files (quirk PAS2).
        let first = !facts
            .types
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(name));
        let mut ty = PasType {
            name: name.to_string(),
            id: id.clone(),
            bases: if first { bases.clone() } else { Vec::new() },
            methods: Vec::new(),
        };
        for b in &bases {
            let local = facts
                .types
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(b))
                .map(|t| t.id.clone());
            match local {
                Some(t) => cx.add_edge(&id, &t, "inherits", line, None),
                None => {
                    let node = slug(b);
                    if node.is_empty() {
                        continue;
                    }
                    def(cx, &node, b, line, vec![]);
                    if !first {
                        cx.add_edge(&id, &node, "inherits", line, None);
                        continue;
                    }
                    let target = virt(cx.rel);
                    cx.add_edge(&id, &target, "inherits", line, None);
                    facts.bases.push(PasBase {
                        target,
                        name: b.clone(),
                        node,
                    });
                }
            }
        }
        // Members up to the closing `end;`.
        let mut j = i + 1;
        while j < line_texts.len() {
            let (mat, ml) = line_texts[j];
            in_body[j] = true;
            let t = ml.trim();
            if t.len() >= 3
                && t[..3].eq_ignore_ascii_case("end")
                && t[3..].trim_start().starts_with(';')
            {
                break;
            }
            if let Some(m) = routine_header(ml) {
                if !m.contains('.') {
                    let mid = Ctx::child_id(&id, m);
                    let mline = lines.line(mat);
                    if def(cx, &mid, &format!("{m}()"), mline, vec![]) {
                        cx.add_edge(&id, &mid, "method", mline, None);
                    }
                    ty.methods.push((m.to_string(), mid.clone()));
                    decls.push(Decl {
                        name: m.to_string(),
                        id: mid,
                        class: Some(name.to_string()),
                    });
                }
            }
            j += 1;
        }
        match facts
            .types
            .iter_mut()
            .find(|t| t.name.eq_ignore_ascii_case(name))
        {
            Some(t) => t.methods.extend(ty.methods),
            None => facts.types.push(ty),
        }
        i = j + 1;
    }

    // Routine definitions: the implementation section, or the whole file without one.
    let start = section_start(&line_texts);
    let mut routines: Vec<(usize, Option<String>, Option<String>)> = Vec::new(); // (offset, caller id, class)
    for (k, (at, l)) in line_texts.iter().enumerate() {
        if *at < start || in_body[k] {
            continue;
        }
        let Some(name) = routine_header(l) else {
            continue;
        };
        let line = lines.line(*at);
        match name.split_once('.') {
            Some((class, method)) => {
                let caller = facts
                    .types
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(class))
                    .and_then(|t| {
                        t.methods
                            .iter()
                            .find(|(m, _)| m.eq_ignore_ascii_case(method))
                            .map(|(_, id)| (id.clone(), t.name.clone()))
                    });
                match caller {
                    Some((id, cls)) => routines.push((*at, Some(id), Some(cls))),
                    None => routines.push((*at, None, None)),
                }
            }
            None => {
                let id = Ctx::child_id(&file, name);
                if def(cx, &id, &format!("{name}()"), line, vec![]) {
                    cx.add_edge(&container, &id, "contains", line, None);
                }
                decls.push(Decl {
                    name: name.to_string(),
                    id: id.clone(),
                    class: None,
                });
                routines.push((*at, Some(id), None));
            }
        }
    }

    // Calls in each routine's first begin … end; block.
    for (at, caller, class) in &routines {
        let Some(caller) = caller else {
            continue;
        };
        let Some((from, to)) = body(&clean, *at) else {
            continue;
        };
        let mut seen: Vec<String> = Vec::new();
        for (off, w) in words(&clean[from..to]) {
            let abs = from + off;
            if !w
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                continue;
            }
            // `x.Name` is a member of something else; `Self.Name` is ours.
            let before = clean[..abs].trim_end();
            if let Some(q) = before.strip_suffix('.') {
                let q = q.trim_end();
                let start = q
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| is_word(*c))
                    .last()
                    .map_or(q.len(), |(k, _)| k);
                if !q[start..].eq_ignore_ascii_case("self") {
                    continue;
                }
            }
            let line = lines.line(*at) + lines.line(abs) - lines.line(from);
            let own: Vec<&Decl> = match class {
                Some(c) => decls
                    .iter()
                    .filter(|d| {
                        d.class.as_deref() == Some(c.as_str()) && d.name.eq_ignore_ascii_case(w)
                    })
                    .collect(),
                None => Vec::new(),
            };
            let target = if own.len() == 1 {
                Some(own[0].id.clone())
            } else {
                let all: Vec<&Decl> = decls
                    .iter()
                    .filter(|d| d.name.eq_ignore_ascii_case(w))
                    .collect();
                if all.len() == 1 {
                    Some(all[0].id.clone())
                } else {
                    None
                }
            };
            match target {
                Some(t) if t != *caller => {
                    if !seen.contains(&t) {
                        seen.push(t.clone());
                        cx.add_edge(caller, &t, "calls", line, Some("call"));
                    }
                }
                Some(_) => {}
                None => {
                    // Left for the caller's base classes in the cross-file pass.
                    let has_bases = class.as_ref().is_some_and(|c| {
                        facts
                            .types
                            .iter()
                            .any(|t| t.name == *c && !t.bases.is_empty())
                    });
                    let key = format!("@{}", w.to_ascii_lowercase());
                    if has_bases && !seen.contains(&key) {
                        seen.push(key);
                        facts.pending.push(PasCall {
                            caller: caller.clone(),
                            class: class.clone().unwrap_or_default(),
                            name: w.to_string(),
                            line,
                        });
                    }
                }
            }
        }
    }
    LegacyFacts {
        pascal: Some(facts),
        ..Default::default()
    }
}

/// `Name[<…>] = class|interface|dispinterface …` that is not a forward or `class of`.
fn type_decl(line: &str) -> Option<(&str, &str)> {
    let (mut name, mut rest) = first_word(line);
    if name.eq_ignore_ascii_case("type") {
        (name, rest) = first_word(rest);
    }
    if name.is_empty() {
        return None;
    }
    let mut rest = rest.trim_start();
    if let Some(r) = rest.strip_prefix('<') {
        rest = r[r.find('>')? + 1..].trim_start();
    }
    let rest = rest.strip_prefix('=')?;
    let (kw, after) = first_word(rest);
    let kw = kw.to_ascii_lowercase();
    if !matches!(kw.as_str(), "class" | "interface" | "dispinterface") {
        return None;
    }
    let after = after.trim_start();
    let (next, _) = first_word(after);
    if next.eq_ignore_ascii_case("of") {
        return None;
    }
    Some((name, after))
}

/// `(A, B<T>)` after `class` / `interface` (`sealed`/`abstract` allowed before it).
fn bases(rest: &str) -> Vec<String> {
    let mut r = rest.trim_start();
    loop {
        let (w, after) = first_word(r);
        if w.eq_ignore_ascii_case("sealed") || w.eq_ignore_ascii_case("abstract") {
            r = after.trim_start();
        } else {
            break;
        }
    }
    let Some(inner) = r.strip_prefix('(') else {
        return Vec::new();
    };
    let inner = &inner[..inner.find(')').unwrap_or(inner.len())];
    inner
        .split(',')
        .map(|b| {
            let b = b.trim();
            b[..b.find('<').unwrap_or(b.len())].trim().to_string()
        })
        .filter(|b| !b.is_empty())
        .collect()
}

/// Byte offset where routine definitions start: after `implementation`, else 0.
fn section_start(lines: &[(usize, &str)]) -> usize {
    for (at, l) in lines {
        let (w, _) = first_word(l);
        if w.eq_ignore_ascii_case("implementation") {
            return *at + l.len();
        }
    }
    0
}

/// The first `begin` after `from` up to the next `end` followed by `;`.
fn body(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut start = None;
    for (off, w) in words(&text[from..]) {
        let abs = from + off;
        match start {
            None if w.eq_ignore_ascii_case("begin") => start = Some(abs + w.len()),
            Some(s)
                if w.eq_ignore_ascii_case("end")
                    && text[abs + 3..].trim_start().starts_with(';') =>
            {
                return Some((s, abs));
            }
            _ => {}
        }
    }
    None
}
