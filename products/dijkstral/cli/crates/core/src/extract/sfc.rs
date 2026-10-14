//! Vue, Svelte and Astro single-file components (§38–§40). See `docs/graphify-format.md`.
//!
//! Graphify finds the embedded script by text, not by parsing the markup: every
//! `<script …>…</script>` pair (case-insensitive, also inside HTML comments) and, in Astro, the
//! `---` frontmatter fence. Everything else is blanked byte for byte (newlines kept), so the
//! script keeps the outer file's lines, and the result is extracted once by the TS/JS extractor.
//! The facts are ordinary [`ScriptFacts`](super::script::ScriptFacts), so
//! `crate::resolve::script` links components to TS/JS modules and back
//! (`import Foo from './Foo.vue'` resolves by its exact path; extensionless specifiers never try
//! `.vue`).
//!
//! The grammar follows the first `lang` attribute on a `<script>` tag (case-insensitive): Vue
//! `tsx` → TSX, `js`/`jsx` → JavaScript, Svelte `js`/`jsx` → JavaScript, anything else (and Astro)
//! → TypeScript. Graphify finds a module's default export with the TypeScript grammar whatever
//! the block's `lang` (`export default function F() { return <div/> }` is not importable); kept.
//!
//! Differences from a `.ts` file: no rationale nodes (tagged comments are ignored in all three).
//! Svelte and Astro go through a reduced path (§39, §40): no import-scoped call pass (calls from
//! module-level functions to imported functions carry `confidence_score` 1.0), no `type_only`, no
//! default-import `imports` links, no heritage or type references, and no node for an exported
//! variable without a value (Svelte props). Graphify's phantom nodes for their unresolved imports
//! (SV2) are not reproduced.

use tree_sitter::Node as TsNode;

use super::script::{self, DefaultExport};
use super::Ctx;
use crate::facts::{LangFacts, NodeKind};
use crate::lang::{parse, Lang};

pub fn extract(cx: &mut Ctx) {
    let masked = mask(cx.lang, cx.src);
    let grammar = grammar(cx.lang, cx.src);
    let tree = parse(grammar, &masked);
    let mut inner = Ctx::new(grammar, cx.rel, &masked);
    script::extract(&mut inner, tree.as_ref());
    let mut facts = inner.facts;
    // Svelte props (`export let name`) and other uninitialised exported variables have no node.
    let bare: Vec<String> = match (&tree, cx.lang) {
        (Some(t), Lang::Svelte | Lang::Astro) => uninitialised_exports(t.root_node(), &masked)
            .into_iter()
            .map(|name| Ctx::child_id(&cx.file_id, &name))
            .collect(),
        _ => Vec::new(),
    };
    facts.nodes.retain(|n| {
        n.kind != NodeKind::Rationale && !(n.kind == NodeKind::Def && bare.contains(&n.id))
    });
    facts.edges.retain(|e| {
        e.relation != "rationale_for" && !bare.contains(&e.source) && !bare.contains(&e.target)
    });
    if let LangFacts::Script(s) = &mut facts.lang {
        if grammar != Lang::TypeScript && s.0.default_export.is_some() {
            s.0.default_export = match ts_default(&masked) {
                Some(DefaultExport::Name(n)) => Some(DefaultExport::Name(n)),
                Some(DefaultExport::Def(name)) => {
                    s.0.top
                        .iter()
                        .find(|d| d.name == name)
                        .map(|d| DefaultExport::Def(d.id.clone()))
                }
                None => None,
            };
        }
        s.0.top.retain(|d| !bare.contains(&d.id));
        if cx.lang != Lang::Vue {
            // Svelte and Astro get no `type_only`, no default-import `imports`, no heritage and
            // no type references.
            for imp in &mut s.0.imports {
                imp.type_only = false;
                imp.names.retain(|n| n.imported != "default");
            }
            s.0.heritage.clear();
            s.0.type_refs.clear();
            for p in &mut s.0.pending {
                if let script::Pending::Call { top, root, .. } = p {
                    *top = false;
                    *root = None;
                }
            }
        }
    }
    cx.facts = facts;
}

/// The default export as the TypeScript grammar sees it, by source name: the first
/// `export default` (pre-order, at any depth, since error recovery may nest it) of a function,
/// generator or class declaration (`Def(name)`) or of an identifier (`Name`). A function
/// expression (what `function F() { return <div/> }` becomes) is not a default.
fn ts_default(masked: &[u8]) -> Option<DefaultExport> {
    let tree = parse(Lang::TypeScript, masked)?;
    let text = |n: TsNode| String::from_utf8_lossy(&masked[n.byte_range()]).into_owned();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "export_statement" {
            let mut c = node.walk();
            let is_default = node.children(&mut c).any(|k| k.kind() == "default");
            if is_default {
                if let Some(decl) = node.child_by_field_name("declaration") {
                    if matches!(
                        decl.kind(),
                        "function_declaration"
                            | "generator_function_declaration"
                            | "class_declaration"
                            | "abstract_class_declaration"
                    ) {
                        let name = decl.child_by_field_name("name")?;
                        return Some(DefaultExport::Def(text(name)));
                    }
                }
                return match node.child_by_field_name("value") {
                    Some(v) if v.kind() == "identifier" => Some(DefaultExport::Name(text(v))),
                    _ => None,
                };
            }
        }
        let mut c = node.walk();
        let children: Vec<TsNode> = node.children(&mut c).collect();
        stack.extend(children.into_iter().rev());
    }
    None
}

/// Names declared without an initializer by a module-level `export let`/`export var`.
fn uninitialised_exports(root: TsNode, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut c = root.walk();
    for stmt in root.named_children(&mut c) {
        if stmt.kind() != "export_statement" {
            continue;
        }
        let Some(decl) = stmt.child_by_field_name("declaration") else {
            continue;
        };
        if !matches!(decl.kind(), "lexical_declaration" | "variable_declaration") {
            continue;
        }
        let mut d = decl.walk();
        for v in decl.named_children(&mut d) {
            if v.kind() != "variable_declarator" || v.child_by_field_name("value").is_some() {
                continue;
            }
            if let Some(name) = v.child_by_field_name("name") {
                out.push(String::from_utf8_lossy(&src[name.byte_range()]).into_owned());
            }
        }
    }
    out
}

/// The grammar the script regions are parsed with (see the module docs).
fn grammar(lang: Lang, src: &[u8]) -> Lang {
    let Some(value) = script_tags(src)
        .into_iter()
        .find_map(|(tag, _)| lang_attr(&src[tag]))
    else {
        return Lang::TypeScript;
    };
    let value = value.to_ascii_lowercase();
    match (lang, value.as_slice()) {
        (Lang::Vue, b"tsx") => Lang::Tsx,
        (Lang::Vue | Lang::Svelte, b"js" | b"jsx") => Lang::JavaScript,
        _ => Lang::TypeScript,
    }
}

/// The value of a `lang=` attribute in an opening tag (quoted or bare).
fn lang_attr(tag: &[u8]) -> Option<&[u8]> {
    let mut i = 0;
    while let Some(off) = find_ci(&tag[i..], b"lang") {
        let at = i + off;
        i = at + 4;
        if at > 0 && !tag[at - 1].is_ascii_whitespace() {
            continue;
        }
        let mut j = i;
        while j < tag.len() && tag[j].is_ascii_whitespace() {
            j += 1;
        }
        if tag.get(j) != Some(&b'=') {
            continue;
        }
        j += 1;
        while j < tag.len() && tag[j].is_ascii_whitespace() {
            j += 1;
        }
        let (start, end) = match tag.get(j) {
            Some(&q @ (b'"' | b'\'')) => {
                let len = tag[j + 1..].iter().position(|&b| b == q)?;
                (j + 1, j + 1 + len)
            }
            _ => {
                let len = tag[j..]
                    .iter()
                    .position(|&b| b.is_ascii_whitespace() || b == b'>' || b == b'/')
                    .unwrap_or(tag.len() - j);
                (j, j + len)
            }
        };
        return Some(&tag[start..end]);
    }
    None
}

/// The source with every byte outside the script regions replaced by a space (newlines kept).
pub fn mask(lang: Lang, src: &[u8]) -> Vec<u8> {
    let mut keep = vec![false; src.len()];
    if lang == Lang::Astro {
        if let Some((start, end)) = frontmatter(src) {
            keep[start..end].fill(true);
        }
    }
    for (_, body) in script_tags(src) {
        keep[body].fill(true);
    }
    src.iter()
        .zip(keep)
        .map(|(&b, k)| if k || b == b'\n' { b } else { b' ' })
        .collect()
}

type Range = std::ops::Range<usize>;

/// `(opening tag, body)` of every `<script…>…</script>` pair, case-insensitive, in order.
fn script_tags(src: &[u8]) -> Vec<(Range, Range)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = find_ci(&src[i..], b"<script") {
        let tag = i + off;
        let Some(gt) = src[tag..].iter().position(|&b| b == b'>') else {
            break;
        };
        let start = tag + gt + 1;
        let Some(len) = find_ci(&src[start..], b"</script>") else {
            break;
        };
        out.push((tag..start, start..start + len));
        i = start + len + b"</script>".len();
    }
    out
}

/// The Astro frontmatter body: the file opens with a `---` line and the next `---` line closes it.
fn frontmatter(src: &[u8]) -> Option<(usize, usize)> {
    let first = src.iter().position(|&b| b == b'\n')?;
    if trim(&src[..first]) != b"---" {
        return None;
    }
    let start = first + 1;
    let mut pos = start;
    while pos < src.len() {
        let end = src[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(src.len(), |p| pos + p);
        if trim(&src[pos..end]) == b"---" {
            return Some((start, pos));
        }
        pos = end + 1;
    }
    None
}

fn trim(line: &[u8]) -> &[u8] {
    let s = line
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(line.len());
    let e = line
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(s, |p| p + 1);
    &line[s..e]
}

fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_everything_but_script_bodies_and_frontmatter() {
        let vue = b"<template>\n<a/>\n</template>\n<SCRIPT lang=\"ts\">\nlet a\n</Script>\n";
        assert_eq!(
            String::from_utf8(mask(Lang::Vue, vue)).unwrap(),
            "          \n    \n           \n                  \nlet a\n         \n"
        );
        let astro = b"---\nconst x = 1\n---\n<p/>\n";
        assert_eq!(
            String::from_utf8(mask(Lang::Astro, astro)).unwrap(),
            "   \nconst x = 1\n   \n    \n"
        );
        // A `---` fence is only frontmatter in Astro, and only on the first line.
        assert!(mask(Lang::Svelte, astro)
            .iter()
            .all(|&b| b == b' ' || b == b'\n'));
        assert!(mask(Lang::Astro, b"\n---\nx\n---\n")
            .iter()
            .all(|&b| b == b' ' || b == b'\n'));
    }

    #[test]
    fn grammar_follows_the_first_script_lang() {
        let g = |lang, s: &str| grammar(lang, s.as_bytes());
        assert_eq!(g(Lang::Vue, "<script>\n</script>"), Lang::TypeScript);
        assert_eq!(
            g(Lang::Vue, "<style lang=\"tsx\"></style><script>\n</script>"),
            Lang::TypeScript
        );
        assert_eq!(
            g(
                Lang::Vue,
                "<script>\n</script><script setup lang=TSX>\n</script>"
            ),
            Lang::Tsx
        );
        assert_eq!(
            g(
                Lang::Vue,
                "<script lang='ts'></script><script lang=\"tsx\"></script>"
            ),
            Lang::TypeScript
        );
        assert_eq!(
            g(Lang::Vue, "<script lang=\"jsx\"></script>"),
            Lang::JavaScript
        );
        assert_eq!(
            g(Lang::Svelte, "<script lang=\"js\"></script>"),
            Lang::JavaScript
        );
        assert_eq!(
            g(Lang::Svelte, "<script lang=\"tsx\"></script>"),
            Lang::TypeScript
        );
        assert_eq!(
            g(Lang::Vue, "<script setup lang=\"typescript\"></script>"),
            Lang::TypeScript
        );
    }
}
