//! JSON manifests (§14.1): every object key becomes a node, dependency entries and `extends`
//! values become per-file `ref_*` concept nodes.

use tree_sitter::Node as TsNode;

use super::{line, parse, Grammar};
use crate::extract::{origin, slug, Ctx};
use crate::facts::{Loc, Node, NodeKind};

/// Basenames (case-insensitive) that are always treated as manifests.
const MANIFEST_NAMES: &[&str] = &[
    "package.json",
    "tsconfig.json",
    "jsconfig.json",
    "composer.json",
    "app.json",
    "manifest.json",
    "deno.json",
    ".eslintrc.json",
    ".babelrc.json",
    ".prettierrc.json",
    "babel.config.json",
    "angular.json",
    "biome.json",
    "bower.json",
    "renovate.json",
    "vercel.json",
    "nest-cli.json",
    "now.json",
];

/// A top-level key that makes any `.json` file a manifest.
const TRIGGER_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
    "bundledDependencies",
    "bundleDependencies",
    "compilerOptions",
    "extends",
    "$schema",
];

/// Objects whose string-valued entries are package references.
const DEP_SECTIONS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

pub fn extract(cx: &mut Ctx) {
    let src = cx.src.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(cx.src);
    let Some(tree) = parse(Grammar::Json, src) else {
        return;
    };
    let root = tree.root_node();
    let mut cursor = root.walk();
    let Some(top) = root
        .named_children(&mut cursor)
        .find(|n| n.kind() == "object")
    else {
        return;
    };
    let base = cx.rel.rsplit('/').next().unwrap_or(cx.rel).to_lowercase();
    let keys: Vec<String> = pairs(top).map(|(k, _)| decode(src, k)).collect();
    let has = |key: &str| keys.iter().any(|k| k == key);
    // JSON Schema documents are data, not manifests.
    if has("$schema") && (has("definitions") || has("$defs") || has("$id")) {
        return;
    }
    let manifest = MANIFEST_NAMES.contains(&base.as_str())
        || keys.iter().any(|k| TRIGGER_KEYS.contains(&k.as_str()));
    if !manifest {
        return;
    }
    cx.add_file_node(vec![]);
    let file_id = cx.file_id.clone();
    let mut budget = MAX_KEYS;
    walk(cx, src, top, &file_id, None, &mut budget);
}

/// Key nodes emitted per file; the walk stops at the 500th (`ref_*` nodes do not count).
const MAX_KEYS: usize = 500;

/// `(key, value)` of every well-formed pair of an object.
fn pairs(obj: TsNode) -> impl Iterator<Item = (TsNode, TsNode)> {
    (0..obj.named_child_count()).filter_map(move |i| {
        let pair = obj.named_child(i)?;
        if pair.kind() != "pair" {
            return None;
        }
        Some((
            pair.child_by_field_name("key")?,
            pair.child_by_field_name("value")?,
        ))
    })
}

/// Walk an object's pairs in pre-order. `parent` is the node that `contains` each key; `parent_key`
/// is the enclosing key (slug and decoded text), absent at depth 1. Key ids use only the immediate
/// parent key (quirk Q16, reproduced for id parity).
fn walk(
    cx: &mut Ctx,
    src: &[u8],
    obj: TsNode,
    parent: &str,
    parent_key: Option<(&str, &str)>,
    budget: &mut usize,
) {
    for (k, v) in pairs(obj) {
        let key = decode(src, k);
        let s = slug(&key);
        if s.is_empty() {
            continue;
        }
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let id = match parent_key {
            None => format!("{}_{s}", cx.file_id),
            Some((ps, _)) => format!("{}_{ps}_{s}", cx.file_id),
        };
        let at = line(k);
        // A dependency with a non-empty version string is labelled `name@version` and
        // `declares` its package; `""`, numbers and objects are plain keys.
        let version = (parent_key.is_some_and(|(_, raw)| DEP_SECTIONS.contains(&raw))
            && v.kind() == "string")
            .then(|| decode(src, v))
            .filter(|v| !v.is_empty());
        let text = match &version {
            Some(ver) => label(&format!("{key}@{ver}")),
            None => label(&key),
        };
        cx.add_def(id.clone(), text, at, vec![]);
        cx.add_edge(parent, &id, "contains", at, None);

        let file_id = cx.file_id.clone();
        if version.is_some() {
            add_ref(cx, &key, at);
            let r = format!("ref_{s}");
            cx.add_edge(&file_id, &r, "imports", at, Some("import"));
            cx.add_edge(&id, &r, "declares", at, None);
        }
        if key == "extends" {
            match v.kind() {
                "string" => {
                    let target = decode(src, v);
                    if let Some(r) = add_ref(cx, &target, at) {
                        cx.add_edge(&file_id, &r, "extends", at, Some("import"));
                    }
                }
                "array" => {
                    let mut cursor = v.walk();
                    for el in v
                        .named_children(&mut cursor)
                        .filter(|n| n.kind() == "string")
                    {
                        let target = decode(src, el);
                        if let Some(r) = add_ref(cx, &target, at) {
                            cx.add_edge(&id, &r, "extends", at, Some("import"));
                        }
                    }
                }
                _ => {}
            }
        }
        if v.kind() == "object" {
            walk(cx, src, v, &id, Some((&s, &key)), budget);
        }
    }
}

/// A per-file reference node `ref_<slug(name)>` (`file_type: "concept"`); `None` when the name
/// has an empty slug.
fn add_ref(cx: &mut Ctx, name: &str, at: u32) -> Option<String> {
    let s = slug(name);
    if s.is_empty() {
        return None;
    }
    let id = format!("ref_{s}");
    cx.facts.nodes.push(Node {
        id: id.clone(),
        label: label(name),
        kind: NodeKind::Def,
        file_type: "concept".into(),
        source_file: cx.rel.into(),
        loc: Loc::Line(at),
        attrs: vec![origin()],
    });
    Some(id)
}

/// The decoded text of a JSON string node; the raw text between the quotes if it does not decode.
fn decode(src: &[u8], string: TsNode) -> String {
    let raw = std::str::from_utf8(&src[string.byte_range()]).unwrap_or("");
    serde_json::from_str::<String>(raw).unwrap_or_else(|_| {
        raw.strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or(raw)
            .to_string()
    })
}

/// Labels are the decoded text re-escaped as a JSON string body (`esc\"aped`, `été`).
fn label(text: &str) -> String {
    let quoted = serde_json::to_string(text).unwrap_or_default();
    quoted[1..quoted.len() - 1].to_string()
}

#[cfg(test)]
mod tests {
    use crate::extract::extract_file;
    use crate::facts::FileFacts;
    use crate::lang::Lang;

    fn ids(f: &FileFacts) -> Vec<&str> {
        f.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn non_manifest_json_yields_nothing() {
        let f = extract_file(Lang::Json, "locales/en.json", br#"{"labels": {"a": 1}}"#);
        assert!(f.nodes.is_empty());
        let f = extract_file(Lang::Json, "package.json", b"[1, 2]");
        assert!(f.nodes.is_empty());
        // A trigger key makes any name a manifest; the basename match is case-insensitive.
        let f = extract_file(Lang::Json, "x.json", br#"{"$schema": "s"}"#);
        assert_eq!(ids(&f), ["x", "x_schema"]);
        let f = extract_file(Lang::Json, "PACKAGE.JSON", b"{}");
        assert_eq!(ids(&f), ["package"]);
        // JSON Schema documents are skipped even with trigger keys.
        let src = br#"{"$schema": "s", "definitions": {}, "dependencies": {"a": "1"}}"#;
        assert!(extract_file(Lang::Json, "x.json", src).nodes.is_empty());
    }

    #[test]
    fn key_ids_use_only_the_immediate_parent() {
        let src = b"{\n \"deep\": {\"a\": {\"b\": {\"c\": 1}}},\n \"@/*\": {\"x\": 1},\n \"arr\": [{\"in\": 1}]\n}";
        let f = extract_file(Lang::Json, "package.json", src);
        assert_eq!(
            ids(&f),
            [
                "package",
                "package_deep",
                "package_deep_a",
                "package_a_b",
                "package_b_c",
                "package_arr"
            ]
        );
        assert!(f
            .edges
            .iter()
            .any(|e| e.source == "package_a_b" && e.target == "package_b_c"));
    }

    #[test]
    fn at_most_500_keys_per_file_in_pre_order() {
        let body: Vec<String> = (0..200)
            .map(|i| format!("\"o{i:03}\": {{\"a\": 1, \"b\": 2}}"))
            .collect();
        let src = format!(
            "{{\"dependencies\": {{\"r\": \"1\"}}, {}}}",
            body.join(", ")
        );
        let f = extract_file(Lang::Json, "package.json", src.as_bytes());
        let keys = f.nodes.iter().filter(|n| n.file_type == "code").count();
        assert_eq!(keys, 501); // file node + 500 keys; the ref does not count
        assert_eq!(f.nodes.last().unwrap().id, "package_o165_b");
    }

    #[test]
    fn dependency_and_extends_refs() {
        let src = br#"{
  // jsonc comment
  "extends": ["./a.json", 3],
  "dependencies": {"@types/node": "1", "obj": {"version": "1"},},
  "workspaces": {"devDependencies": {"nested": "1"}},
  "config": {"extends": "./c.json"}
}"#;
        let f = extract_file(Lang::Json, "tsconfig.json", src);
        let refs: Vec<_> = f
            .nodes
            .iter()
            .filter(|n| n.file_type == "concept")
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect();
        assert_eq!(
            refs,
            [
                ("ref_a_json", "./a.json"),
                ("ref_types_node", "@types/node"),
                ("ref_nested", "nested"),
                ("ref_c_json", "./c.json"),
            ]
        );
        let rel: Vec<_> = f
            .edges
            .iter()
            .filter(|e| e.relation != "contains")
            .map(|e| (e.source.as_str(), e.relation.as_str(), e.target.as_str()))
            .collect();
        assert_eq!(
            rel,
            [
                ("tsconfig_extends", "extends", "ref_a_json"),
                ("tsconfig", "imports", "ref_types_node"),
                ("tsconfig", "imports", "ref_nested"),
                ("tsconfig", "extends", "ref_c_json"),
            ]
        );
    }

    #[test]
    fn labels_are_reescaped_and_lines_follow_keys() {
        let src =
            "{\n \"a\": \"name\",\n \"esc\\\"aped\": 1,\n \"\\u00e9t\\u00e9\": 2,\n \"name\": 3\n}";
        let f = extract_file(Lang::Json, "package.json", src.as_bytes());
        let got: Vec<_> = f
            .nodes
            .iter()
            .map(|n| (n.label.as_str(), n.loc))
            .skip(1)
            .collect();
        use crate::facts::Loc::Line;
        assert_eq!(
            got,
            [
                ("a", Line(2)),
                ("esc\\\"aped", Line(3)),
                ("été", Line(4)),
                ("name", Line(5))
            ]
        );
    }
}
