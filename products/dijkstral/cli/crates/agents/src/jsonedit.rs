//! Byte-preserving edits of hook settings files (`.claude/settings.json`, `.codex/hooks.json`).
//!
//! Graphify re-serialises the whole file (§12.1); dijkstral splices text instead, so every byte
//! outside the hook entries it owns is kept: the user's formatting, key order, other hooks and
//! settings. Inserted text follows the file's own indentation. Each edit is checked by parsing the
//! result and comparing it structurally with the input before it is returned.

use anyhow::{bail, Result};
use serde_json::{Map, Value};

/// A JSON value with the byte span it occupies in the source text.
#[derive(Debug)]
enum Node {
    Object {
        start: usize,
        end: usize,
        /// Key, byte offset of the key's opening quote, value.
        members: Vec<(String, usize, Node)>,
    },
    Array {
        start: usize,
        end: usize,
        items: Vec<Node>,
    },
    Scalar {
        start: usize,
        end: usize,
    },
}

impl Node {
    fn span(&self) -> (usize, usize) {
        match *self {
            Node::Object { start, end, .. }
            | Node::Array { start, end, .. }
            | Node::Scalar { start, end } => (start, end),
        }
    }

    fn member(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Object { members, .. } => members
                .iter()
                .rev()
                .find(|(k, _, _)| k == key)
                .map(|(_, _, v)| v),
            _ => None,
        }
    }
}

/// Span parser for text that `serde_json` has already accepted.
struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
}

impl Scanner<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn string_end(&mut self) -> Option<usize> {
        // At the opening quote.
        self.i += 1;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'\\' => self.i += 2,
                b'"' => {
                    self.i += 1;
                    return Some(self.i);
                }
                _ => self.i += 1,
            }
        }
        None
    }

    fn value(&mut self) -> Option<Node> {
        self.ws();
        let start = self.i;
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut members = Vec::new();
                loop {
                    self.ws();
                    match *self.s.get(self.i)? {
                        b'}' => {
                            self.i += 1;
                            return Some(Node::Object {
                                start,
                                end: self.i,
                                members,
                            });
                        }
                        b',' => self.i += 1,
                        b'"' => {
                            let key_start = self.i;
                            let key_end = self.string_end()?;
                            let raw = std::str::from_utf8(&self.s[key_start..key_end]).ok()?;
                            let key: String = serde_json::from_str(raw).ok()?;
                            self.ws();
                            if self.s.get(self.i) != Some(&b':') {
                                return None;
                            }
                            self.i += 1;
                            let value = self.value()?;
                            members.push((key, key_start, value));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    match *self.s.get(self.i)? {
                        b']' => {
                            self.i += 1;
                            return Some(Node::Array {
                                start,
                                end: self.i,
                                items,
                            });
                        }
                        b',' => self.i += 1,
                        _ => items.push(self.value()?),
                    }
                }
            }
            b'"' => {
                let end = self.string_end()?;
                Some(Node::Scalar { start, end })
            }
            _ => {
                while self.i < self.s.len()
                    && !matches!(
                        self.s[self.i],
                        b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r'
                    )
                {
                    self.i += 1;
                }
                Some(Node::Scalar { start, end: self.i })
            }
        }
    }
}

fn scan(text: &str) -> Option<Node> {
    Scanner {
        s: text.as_bytes(),
        i: 0,
    }
    .value()
}

/// The parsed document, required to be an object whose `hooks` (if present) is an object whose
/// `event` (if present) is an array.
fn validate(text: &str, event: &str) -> Result<Value> {
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        bail!("not valid JSON");
    };
    let Some(root) = doc.as_object() else {
        bail!("not a JSON object");
    };
    if let Some(hooks) = root.get("hooks") {
        let Some(hooks) = hooks.as_object() else {
            bail!("\"hooks\" is not an object");
        };
        if hooks.get(event).is_some_and(|v| !v.is_array()) {
            bail!("\"hooks\".\"{event}\" is not an array");
        }
    }
    Ok(doc)
}

/// The document with every `hooks.<event>` entry for which `ours` holds removed, and an emptied
/// event array and `hooks` object dropped, for comparing edits structurally.
fn without_ours(mut doc: Value, event: &str, ours: &dyn Fn(&Value) -> bool) -> Value {
    let Some(root) = doc.as_object_mut() else {
        return doc;
    };
    let mut drop_hooks = false;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            list.retain(|e| !ours(e));
            if list.is_empty() {
                hooks.shift_remove(event);
            }
        }
        drop_hooks = hooks.is_empty();
    }
    if drop_hooks {
        root.shift_remove("hooks");
    }
    doc
}

/// Removes every `hooks.<event>` entry for which `ours` holds. When that empties the event's list,
/// the list goes too, and then an emptied `hooks` object, so removing what an install added
/// restores the file byte for byte. Returns the text unchanged when nothing is ours.
pub fn remove_entries(text: &str, event: &str, ours: &dyn Fn(&Value) -> bool) -> Result<String> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let before = validate(text, event)?;
    let mut text = text.to_string();
    let mut removed_any = false;
    loop {
        let root = scan(&text).ok_or_else(|| anyhow::anyhow!("not valid JSON"))?;
        let Some(list @ Node::Array { items, .. }) =
            root.member("hooks").and_then(|h| h.member(event))
        else {
            break;
        };
        let Some(i) = items.iter().position(|item| {
            let (s, e) = item.span();
            serde_json::from_str::<Value>(&text[s..e]).is_ok_and(|v| ours(&v))
        }) else {
            break;
        };
        let (s, e) = cut_child(list, i);
        text.replace_range(s..e, "");
        removed_any = true;
    }
    if removed_any {
        drop_empty_member(&mut text, Some("hooks"), event)?;
        drop_empty_member(&mut text, None, "hooks")?;
    }
    let after = validate(&text, event)?;
    if without_ours(before, event, ours) != without_ours(after, event, ours) {
        bail!("internal error: removing dijkstral hooks would change other settings");
    }
    Ok(text)
}

/// Removes member `key` of the root object (or of its `parent` member) when it is an empty array
/// or object.
fn drop_empty_member(text: &mut String, parent: Option<&str>, key: &str) -> Result<()> {
    let root = scan(text).ok_or_else(|| anyhow::anyhow!("not valid JSON"))?;
    let object = match parent {
        Some(name) => root.member(name),
        None => Some(&root),
    };
    let Some(object @ Node::Object { members, .. }) = object else {
        return Ok(());
    };
    let empty = |v: &Node| match v {
        Node::Array { items, .. } => items.is_empty(),
        Node::Object { members, .. } => members.is_empty(),
        Node::Scalar { .. } => false,
    };
    if let Some(i) = members.iter().position(|(k, _, v)| k == key && empty(v)) {
        let (s, e) = cut_child(object, i);
        text.replace_range(s..e, "");
    }
    Ok(())
}

/// The byte range to delete to remove child `i` of an object or array, together with the comma
/// and whitespace that joined it to a neighbour; an only child takes all inner whitespace.
fn cut_child(container: &Node, i: usize) -> (usize, usize) {
    let spans: Vec<(usize, usize)> = match container {
        Node::Object { members, .. } => members.iter().map(|(_, k, v)| (*k, v.span().1)).collect(),
        Node::Array { items, .. } => items.iter().map(Node::span).collect(),
        Node::Scalar { .. } => Vec::new(),
    };
    let (start, end) = container.span();
    if spans.len() <= 1 {
        (start + 1, end - 1)
    } else if i > 0 {
        (spans[i - 1].1, spans[i].1)
    } else {
        (spans[0].0, spans[1].0)
    }
}

/// Replaces our `hooks.<event>` entries with `entries`, appended at the end of the event's list
/// (created, with `hooks`, when missing). An empty or whitespace-only file becomes a fresh
/// document.
pub fn upsert_entries(
    text: &str,
    event: &str,
    entries: &[Value],
    ours: &dyn Fn(&Value) -> bool,
) -> Result<String> {
    if text.trim().is_empty() {
        let mut list = Map::new();
        list.insert(event.to_string(), Value::Array(entries.to_vec()));
        let mut root = Map::new();
        root.insert("hooks".into(), Value::Object(list));
        let mut out = render(&Value::Object(root), "  ", "", true);
        out.push('\n');
        return Ok(out);
    }
    let before = validate(text, event)?;
    // Layout comes from the file as given: removing our old entries may leave a bare `{}`.
    let style = Style::of(text);
    let mut text = remove_entries(text, event, ours)?;
    let root = scan(&text).ok_or_else(|| anyhow::anyhow!("not valid JSON"))?;
    let hooks = root.member("hooks");
    let list = hooks.and_then(|h| h.member(event));
    let (container, children): (&Node, Vec<String>) = match (hooks, list) {
        (Some(_), Some(list)) => {
            let indent = child_indent(&text, list, &style.unit);
            let items = entries.iter().map(|e| style.render(e, &indent)).collect();
            (list, items)
        }
        (Some(hooks), None) => {
            let indent = child_indent(&text, hooks, &style.unit);
            let value = Value::Array(entries.to_vec());
            let member = format!(
                "{}{}{}",
                Value::from(event),
                style.colon,
                style.render(&value, &indent)
            );
            (hooks, vec![member])
        }
        (None, _) => {
            let indent = child_indent(&text, &root, &style.unit);
            let mut list = Map::new();
            list.insert(event.to_string(), Value::Array(entries.to_vec()));
            let member = format!(
                "\"hooks\"{}{}",
                style.colon,
                style.render(&Value::Object(list), &indent)
            );
            (&root, vec![member])
        }
    };
    let (start, end, insert) = insert_children(&text, container, &children, &style);
    text.replace_range(start..end, &insert);

    let after = validate(&text, event)?;
    let ok = without_ours(before, event, ours) == without_ours(after.clone(), event, ours)
        && after["hooks"][event]
            .as_array()
            .is_some_and(|l| l.ends_with(entries));
    if !ok {
        bail!("internal error: the edited settings did not round-trip");
    }
    Ok(text)
}

/// How the file lays out JSON, so inserted text looks like the rest of it.
struct Style {
    /// Multi-line (pretty-printed) rather than single-line.
    pretty: bool,
    /// One indentation step.
    unit: String,
    /// What follows a comma between siblings on one line (compact files only).
    after_comma: &'static str,
    /// Separator between a key and its value.
    colon: &'static str,
}

impl Style {
    fn of(text: &str) -> Style {
        let pretty = text.trim().contains('\n');
        Style {
            pretty,
            unit: indent_unit(text),
            after_comma: if text.contains(", ") { " " } else { "" },
            colon: if pretty || text.contains("\": ") {
                ": "
            } else {
                ":"
            },
        }
    }

    fn render(&self, value: &Value, indent: &str) -> String {
        render(value, &self.unit, indent, self.pretty)
    }
}

/// The replacement `(start, end, text)` that appends `children` to an object or array.
fn insert_children(
    text: &str,
    container: &Node,
    children: &[String],
    style: &Style,
) -> (usize, usize, String) {
    let (start, end) = container.span();
    let indent = child_indent(text, container, &style.unit);
    let last_child_end = match container {
        Node::Object { members, .. } => members.last().map(|(_, _, v)| v.span().1),
        Node::Array { items, .. } => items.last().map(|v| v.span().1),
        Node::Scalar { .. } => None,
    };
    let sep = if style.pretty {
        format!(",\n{indent}")
    } else {
        format!(",{}", style.after_comma)
    };
    match last_child_end {
        Some(at) => {
            let mut s = String::new();
            for c in children {
                s.push_str(&sep);
                s.push_str(c);
            }
            (at, at, s)
        }
        None => {
            let body = if style.pretty {
                let closing = line_indent(text, start);
                format!("\n{indent}{}\n{closing}", children.join(&sep))
            } else {
                children.join(&sep)
            };
            (start + 1, end - 1, body)
        }
    }
}

/// Indentation for a child of `container`: that of its first child when that child starts a
/// line, otherwise the container's line indentation plus one unit.
fn child_indent(text: &str, container: &Node, unit: &str) -> String {
    let first = match container {
        Node::Object { members, .. } => members.first().map(|(_, key, _)| *key),
        Node::Array { items, .. } => items.first().map(|v| v.span().0),
        Node::Scalar { .. } => None,
    };
    if let Some(pos) = first {
        let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
        let lead = &text[line_start..pos];
        if line_start > 0 && lead.chars().all(|c| c == ' ' || c == '\t') {
            return lead.to_string();
        }
    }
    format!("{}{unit}", line_indent(text, container.span().0))
}

/// Leading whitespace of the line containing byte `pos`.
fn line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The file's indentation step: the leading whitespace of its first indented line, else 2 spaces.
fn indent_unit(text: &str) -> String {
    text.lines()
        .map(|l| {
            l.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        })
        .find(|lead| !lead.is_empty())
        .unwrap_or_else(|| "  ".to_string())
}

/// `value` as JSON text whose continuation lines start with `indent` (pretty), or compact.
fn render(value: &Value, unit: &str, indent: &str, pretty: bool) -> String {
    if !pretty {
        return value.to_string();
    }
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(value, &mut ser).expect("serialising a Value cannot fail");
    let text = String::from_utf8(buf).expect("serde_json writes UTF-8");
    text.replace('\n', &format!("\n{indent}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ours(v: &Value) -> bool {
        v["hooks"][0]["command"]
            .as_str()
            .is_some_and(|c| c.contains("dijkstral hook-guard"))
    }

    fn entries() -> Vec<Value> {
        vec![
            json!({"matcher": "Bash|Grep", "hooks": [{"type": "command", "command": "dijkstral hook-guard search", "timeout": 10}]}),
            json!({"matcher": "Read|Glob", "hooks": [{"type": "command", "command": "dijkstral hook-guard read", "timeout": 10}]}),
        ]
    }

    fn round_trip(original: &str) -> String {
        let installed = upsert_entries(original, "PreToolUse", &entries(), &ours).unwrap();
        let again = upsert_entries(&installed, "PreToolUse", &entries(), &ours).unwrap();
        assert_eq!(installed, again, "reinstall must be a no-op");
        let doc: Value = serde_json::from_str(&installed).unwrap();
        assert!(doc["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .ends_with(&entries()));
        let removed = remove_entries(&installed, "PreToolUse", &ours).unwrap();
        assert_eq!(
            remove_entries(&removed, "PreToolUse", &ours).unwrap(),
            removed
        );
        installed + "\u{0}" + &removed
    }

    #[test]
    fn existing_list_round_trips_byte_for_byte() {
        let original = "{\n    \"model\": \"opus\",\n    \"hooks\": {\n        \"PreToolUse\": [\n            {\"matcher\": \"Edit\", \"hooks\": [{\"type\": \"command\", \"command\": \"fmt\"}]}\n        ],\n        \"Stop\": []\n    },\n    \"z\": [1, 2]\n}\n";
        let out = round_trip(original);
        let (installed, removed) = out.split_once('\u{0}').unwrap();
        assert_eq!(removed, original);
        assert!(installed.starts_with(
            "{\n    \"model\": \"opus\",\n    \"hooks\": {\n        \"PreToolUse\": [\n            {\"matcher\": \"Edit\", \"hooks\": [{\"type\": \"command\", \"command\": \"fmt\"}]},\n            {\n                \"matcher\": \"Bash|Grep\",\n"
        ), "{installed}");
        assert!(
            installed.ends_with("        ],\n        \"Stop\": []\n    },\n    \"z\": [1, 2]\n}\n")
        );
    }

    #[test]
    fn our_entries_between_user_entries_are_removed_cleanly() {
        let original = "{\"hooks\": {\"PreToolUse\": [{\"matcher\": \"A\", \"hooks\": []}, {\"matcher\": \"Bash|Grep\", \"hooks\": [{\"type\": \"command\", \"command\": \"/old/dijkstral hook-guard search\"}]}, {\"matcher\": \"B\", \"hooks\": []}]}}";
        let removed = remove_entries(original, "PreToolUse", &ours).unwrap();
        assert_eq!(
            removed,
            "{\"hooks\": {\"PreToolUse\": [{\"matcher\": \"A\", \"hooks\": []}, {\"matcher\": \"B\", \"hooks\": []}]}}"
        );
        // Install moves ours to the end, in the file's compact style.
        let installed = upsert_entries(original, "PreToolUse", &entries(), &ours).unwrap();
        assert!(installed.starts_with(
            "{\"hooks\": {\"PreToolUse\": [{\"matcher\": \"A\", \"hooks\": []}, {\"matcher\": \"B\", \"hooks\": []}, {\"matcher\":\"Bash|Grep\""
        ));
    }

    #[test]
    fn missing_hooks_and_missing_event_are_created_in_place() {
        let original = "{\n  \"permissions\": {\"allow\": []}\n}";
        let out = round_trip(original);
        let (installed, removed) = out.split_once('\u{0}').unwrap();
        assert!(installed.starts_with("{\n  \"permissions\": {\"allow\": []},\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\n        \"matcher\": \"Bash|Grep\""));
        assert!(installed.ends_with("\n    ]\n  }\n}"));
        assert_eq!(removed, original);

        let original = "{\n  \"hooks\": {\n    \"Stop\": []\n  },\n  \"x\": 1\n}\n";
        let out = round_trip(original);
        let (installed, removed) = out.split_once('\u{0}').unwrap();
        assert!(installed
            .starts_with("{\n  \"hooks\": {\n    \"Stop\": [],\n    \"PreToolUse\": [\n      {\n"));
        assert!(installed.ends_with("    ]\n  },\n  \"x\": 1\n}\n"));
        assert_eq!(removed, original);

        let out = round_trip("{}");
        let (installed, removed) = out.split_once('\u{0}').unwrap();
        assert!(
            installed.starts_with("{\"hooks\":{\"PreToolUse\":[{"),
            "{installed}"
        );
        assert_eq!(removed, "{}");

        // A user's own empty list is left alone when nothing of ours is in it.
        let own = "{\"hooks\": {\"PreToolUse\": []}}";
        assert_eq!(remove_entries(own, "PreToolUse", &ours).unwrap(), own);

        let installed = upsert_entries("{\n}\n", "PreToolUse", &entries(), &ours).unwrap();
        assert!(
            installed.starts_with("{\n  \"hooks\": {\n    \"PreToolUse\": [\n"),
            "{installed}"
        );
        assert!(installed.ends_with("]\n  }\n}\n"));
    }

    #[test]
    fn empty_file_gets_a_fresh_document() {
        let installed = upsert_entries("", "PreToolUse", &entries(), &ours).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&installed).unwrap(),
            json!({"hooks": {"PreToolUse": entries()}})
        );
        assert!(installed
            .starts_with("{\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\n        \"matcher\""));
        // Reinstalling over our own fresh document changes nothing, although removing our
        // entries first leaves a single-line `{}`.
        assert_eq!(
            upsert_entries(&installed, "PreToolUse", &entries(), &ours).unwrap(),
            installed
        );
        let removed = remove_entries(&installed, "PreToolUse", &ours).unwrap();
        assert_eq!(removed, "{}\n");
    }

    #[test]
    fn tabs_are_followed() {
        let original = "{\n\t\"hooks\": {\n\t\t\"PreToolUse\": [\n\t\t\t{\"matcher\": \"X\", \"hooks\": []}\n\t\t]\n\t}\n}\n";
        let out = round_trip(original);
        let (installed, removed) = out.split_once('\u{0}').unwrap();
        assert_eq!(removed, original);
        assert!(installed.contains("},\n\t\t\t{\n\t\t\t\t\"matcher\": \"Bash|Grep\",\n\t\t\t\t\"hooks\": [\n\t\t\t\t\t{\n"), "{installed}");
    }

    #[test]
    fn invalid_documents_are_refused() {
        for bad in [
            "{bad",
            "[]",
            "{\"hooks\": 5}",
            "{\"hooks\": {\"PreToolUse\": {}}}",
            "null",
        ] {
            assert!(
                upsert_entries(bad, "PreToolUse", &entries(), &ours).is_err(),
                "{bad}"
            );
            assert!(remove_entries(bad, "PreToolUse", &ours).is_err(), "{bad}");
        }
    }

    #[test]
    fn strings_with_brackets_and_escapes_do_not_confuse_the_scanner() {
        let original = "{\"a\": \"]}\\\"{[\", \"hooks\": {\"PreToolUse\": [\"x]\"]}}";
        let out = round_trip(original);
        let (_, removed) = out.split_once('\u{0}').unwrap();
        assert_eq!(removed, original);
    }
}
