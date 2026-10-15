//! Reply validation and repair (§6): the first JSON object of a reply (code fences and prose
//! around it accepted), salvage of a truncated reply, conversion to `Reply` and the schema
//! warnings.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::types::{Reply, ReplyEdge, ReplyHyperedge, ReplyNode};

pub const FILE_TYPES: [&str; 6] = ["code", "document", "paper", "image", "concept", "rationale"];
pub const CONFIDENCES: [&str; 3] = ["EXTRACTED", "INFERRED", "AMBIGUOUS"];

/// Why a reply has no usable object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unusable {
    Empty,
    /// No JSON object that `accept` takes; holds the first 200 characters of the reply.
    Invalid(String),
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unusable::Empty => f.write_str("empty reply"),
            Unusable::Invalid(head) => write!(f, "invalid JSON (first 200 chars: {head})"),
        }
    }
}

/// The first JSON object in `text` that `accept` takes. A code fence or prose before and after
/// the object is accepted, and of two concatenated objects the first is used. Single quotes,
/// trailing commas and a bare array are not repaired.
pub fn first_object(
    text: &str,
    accept: impl Fn(&Map<String, Value>) -> bool,
) -> Result<Map<String, Value>, Unusable> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Unusable::Empty);
    }
    let invalid = || Unusable::Invalid(trimmed.chars().take(200).collect());
    // A bare array (optionally fenced) is not a reply.
    let unfenced = trimmed
        .strip_prefix("```")
        .map(|rest| rest.split_once('\n').map_or("", |(_, body)| body))
        .unwrap_or(trimmed);
    if unfenced.trim_start().starts_with('[') {
        return Err(invalid());
    }
    for (tried, (i, _)) in text.match_indices('{').enumerate() {
        if tried >= 256 {
            break;
        }
        let mut stream = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>();
        if let Some(Ok(Value::Object(map))) = stream.next() {
            if accept(&map) {
                return Ok(map);
            }
        }
    }
    Err(invalid())
}

/// Whether `map` looks like an extraction reply.
pub fn is_extraction(map: &Map<String, Value>) -> bool {
    ["nodes", "edges", "hyperedges"]
        .iter()
        .any(|k| map.contains_key(*k))
}

/// The complete part of a reply cut off at the output limit: everything up to the last array
/// element that closed, with the open containers closed after it.
pub fn salvage(text: &str) -> Option<Map<String, Value>> {
    let start = text.find('{')?;
    let body = &text[start..];
    let mut stack: Vec<u8> = Vec::new();
    let mut checkpoints: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut in_str = false;
    let mut escaped = false;
    for (i, b) in body.bytes().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => stack.push(b),
            b'}' | b']' => {
                stack.pop();
                if stack.is_empty() {
                    break;
                }
                if stack.last() == Some(&b'[') {
                    checkpoints.push((i + 1, stack.clone()));
                }
            }
            _ => {}
        }
    }
    for (end, open) in checkpoints.iter().rev().take(64) {
        let mut candidate = body[..*end].to_string();
        for c in open.iter().rev() {
            candidate.push(if *c == b'[' { ']' } else { '}' });
        }
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&candidate) {
            if is_extraction(&map) {
                return Some(map);
            }
        }
    }
    None
}

/// `map`'s `nodes`, `edges` and `hyperedges` as a `Reply`. Values of the wrong JSON type are
/// coerced (numbers and booleans to strings) or dropped; items that are not objects are skipped.
pub fn to_reply(map: &Map<String, Value>) -> Reply {
    Reply {
        nodes: items(map, "nodes")
            .filter_map(|v| typed::<ReplyNode>(v, NODE_STR, &[]))
            .collect(),
        edges: items(map, "edges")
            .filter_map(|v| typed::<ReplyEdge>(v, EDGE_STR, EDGE_NUM))
            .collect(),
        hyperedges: items(map, "hyperedges")
            .filter_map(|v| {
                let mut v = v.clone();
                if let Some(Value::Array(members)) =
                    v.as_object_mut().and_then(|o| o.get_mut("nodes"))
                {
                    members.retain(Value::is_string);
                } else if let Some(o) = v.as_object_mut() {
                    o.remove("nodes");
                }
                typed::<ReplyHyperedge>(&v, HYPER_STR, HYPER_NUM)
            })
            .collect(),
    }
}

const NODE_STR: &[&str] = &[
    "id",
    "label",
    "file_type",
    "source_file",
    "source_url",
    "captured_at",
    "author",
    "contributor",
    "rationale",
];
const EDGE_STR: &[&str] = &["source", "target", "relation", "confidence", "source_file"];
const EDGE_NUM: &[&str] = &["confidence_score", "weight"];
const HYPER_STR: &[&str] = &["id", "label", "relation", "confidence", "source_file"];
const HYPER_NUM: &[&str] = &["confidence_score"];

pub fn items<'a>(map: &'a Map<String, Value>, key: &str) -> impl Iterator<Item = &'a Value> {
    map.get(key).and_then(Value::as_array).into_iter().flatten()
}

fn typed<T: serde::de::DeserializeOwned>(v: &Value, strs: &[&str], nums: &[&str]) -> Option<T> {
    let mut o = v.as_object()?.clone();
    for k in strs {
        match o.get(*k) {
            Some(Value::Number(n)) => {
                let s = n.to_string();
                o.insert((*k).to_string(), Value::String(s));
            }
            Some(Value::Bool(b)) => {
                let s = b.to_string();
                o.insert((*k).to_string(), Value::String(s));
            }
            Some(Value::Array(_) | Value::Object(_)) => {
                o.remove(*k);
            }
            _ => {}
        }
    }
    for k in nums {
        if !matches!(o.get(*k), None | Some(Value::Null | Value::Number(_))) {
            o.remove(*k);
        }
    }
    serde_json::from_value(Value::Object(o)).ok()
}

/// A reply with no nodes and no edges (§6).
pub fn is_hollow(map: &Map<String, Value>) -> bool {
    items(map, "nodes").next().is_none() && items(map, "edges").next().is_none()
}

/// Schema warnings, counted by kind: `missing required field 'X'` and `other` with the first
/// example (§6). Warnings never drop an item.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchemaIssues {
    pub missing: BTreeMap<String, usize>,
    pub other: usize,
    pub other_example: Option<String>,
}

impl SchemaIssues {
    pub fn total(&self) -> usize {
        self.missing.values().sum::<usize>() + self.other
    }

    pub fn add(&mut self, other: &SchemaIssues) {
        for (k, n) in &other.missing {
            *self.missing.entry(k.clone()).or_default() += n;
        }
        self.other += other.other;
        if self.other_example.is_none() {
            self.other_example.clone_from(&other.other_example);
        }
    }

    fn note(&mut self, example: String) {
        self.other += 1;
        self.other_example.get_or_insert(example);
    }

    /// `Extraction warning (N issues): 12x missing required field 'source_file'; …`, or `None`.
    pub fn summary(&self) -> Option<String> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        let mut missing: Vec<(&String, &usize)> = self.missing.iter().collect();
        missing.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut parts: Vec<String> = missing
            .into_iter()
            .map(|(k, n)| format!("{n}x missing required field '{k}'"))
            .collect();
        if self.other > 0 {
            parts.push(format!(
                "{}x other schema issue (e.g. {})",
                self.other,
                self.other_example.as_deref().unwrap_or("")
            ));
        }
        Some(format!(
            "Extraction warning ({total} issues): {}",
            parts.join("; ")
        ))
    }
}

/// The schema warnings of one reply object.
pub fn schema_issues(map: &Map<String, Value>) -> SchemaIssues {
    let mut out = SchemaIssues::default();
    for key in ["nodes", "edges", "hyperedges"] {
        if let Some(v) = map.get(key) {
            if !v.is_array() {
                out.note(format!("'{key}' is not a list"));
            }
        }
    }
    let present = |o: &Map<String, Value>, k: &str| {
        o.get(k)
            .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
    };
    for (i, v) in items(map, "nodes").enumerate() {
        let Some(o) = v.as_object() else {
            out.note(format!("Node {i} is not an object"));
            continue;
        };
        for k in ["id", "label", "source_file"] {
            if !present(o, k) {
                *out.missing.entry(k.to_string()).or_default() += 1;
            }
        }
        if let Some(t) = o.get("file_type").and_then(Value::as_str) {
            if !FILE_TYPES.contains(&t) {
                out.note(format!("Node {i} has invalid file_type '{t}'"));
            }
        }
    }
    for (i, v) in items(map, "edges").enumerate() {
        let Some(o) = v.as_object() else {
            out.note(format!("Edge {i} is not an object"));
            continue;
        };
        for k in ["source", "target", "relation", "confidence", "source_file"] {
            if !present(o, k) {
                *out.missing.entry(k.to_string()).or_default() += 1;
            }
        }
        if let Some(c) = o.get("confidence").and_then(Value::as_str) {
            if !CONFIDENCES.contains(&c) {
                out.note(format!("Edge {i} has invalid confidence '{c}'"));
            }
        }
    }
    for (i, v) in items(map, "hyperedges").enumerate() {
        if !v.get("nodes").is_some_and(Value::is_array) {
            out.note(format!("Hyperedge {i} has no node list"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: &str = r#"{"nodes":[{"id":"a","label":"A","source_file":"x.md"}],"edges":[]}"#;

    #[test]
    fn accepts_fences_and_prose() {
        for text in [
            R.to_string(),
            format!("```json\n{R}\n```"),
            format!("Here is the graph:\n{R}\nHope it helps {{:)}}"),
            format!("{R}{{\"nodes\":[]}}"),
            format!("prose {{not json}} then {R}"),
        ] {
            let m = first_object(&text, is_extraction).unwrap();
            assert_eq!(to_reply(&m).nodes[0].id.as_deref(), Some("a"), "{text}");
        }
    }

    #[test]
    fn rejects_what_is_not_repaired() {
        assert_eq!(first_object("  ", is_extraction), Err(Unusable::Empty));
        for text in [
            "{'nodes': []}",
            r#"{"nodes": [{"id": "a"},], "edges": []}"#,
            r#"[{"nodes": []}]"#,
            "```json\n[{\"nodes\": []}]\n```",
            "no json here",
        ] {
            assert!(
                matches!(first_object(text, is_extraction), Err(Unusable::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn salvages_a_truncated_reply() {
        let text = r#"{"nodes":[{"id":"a","label":"A"},{"id":"b","label":"B"},{"id":"c","lab"#;
        assert!(first_object(text, is_extraction).is_err());
        let m = salvage(text).unwrap();
        assert_eq!(to_reply(&m).nodes.len(), 2);
        let text = r#"{"nodes":[{"id":"a"}],"edges":[{"source":"a","target":"b"},{"sou"#;
        let r = to_reply(&salvage(text).unwrap());
        assert_eq!((r.nodes.len(), r.edges.len()), (1, 1));
        assert!(salvage(r#"{"nodes":[{"id":"#).is_none());
    }

    #[test]
    fn coerces_wrong_types() {
        let m = first_object(
            r#"{"nodes":[{"id":5,"label":["x"],"source_file":"a"}, 3],
                "edges":[{"source":"a","target":"b","confidence_score":"high","weight":0.5}],
                "hyperedges":[{"id":"h","nodes":["a",1,"b"]}]}"#,
            is_extraction,
        )
        .unwrap();
        let r = to_reply(&m);
        assert_eq!(r.nodes.len(), 1);
        assert_eq!(r.nodes[0].id.as_deref(), Some("5"));
        assert_eq!(r.nodes[0].label, None);
        assert_eq!(r.edges[0].confidence_score, None);
        assert_eq!(r.edges[0].weight, Some(0.5));
        assert_eq!(r.hyperedges[0].nodes, vec!["a", "b"]);
    }

    #[test]
    fn hollow_and_schema_warnings() {
        assert!(is_hollow(
            &first_object(
                r#"{"nodes":[],"edges":[],"hyperedges":[{"id":"h"}]}"#,
                is_extraction
            )
            .unwrap()
        ));
        let m = first_object(
            r#"{"nodes":[{"id":"a"},{"id":"b","label":"B","file_type":"widget"}],
                "edges":[{"source":"a","target":"b","relation":"r","confidence":"maybe"}]}"#,
            is_extraction,
        )
        .unwrap();
        let s = schema_issues(&m);
        assert_eq!(s.missing["source_file"], 3);
        assert_eq!(s.missing["label"], 1);
        assert_eq!(s.other, 2);
        assert_eq!(
            s.summary().unwrap(),
            "Extraction warning (6 issues): 3x missing required field 'source_file'; \
             1x missing required field 'label'; 2x other schema issue (e.g. Node 1 has invalid \
             file_type 'widget')"
        );
    }
}
