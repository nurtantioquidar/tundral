//! The `.gitattributes` entry that routes `dijkstral-out/graph.json` to the merge driver.

use crate::{DRIVER, GRAPH_PATH};

/// The line `hook install` adds.
pub fn line() -> String {
    format!("{GRAPH_PATH} merge={DRIVER}")
}

fn attr() -> String {
    format!("merge={DRIVER}")
}

/// Whether a line gives the graph path the driver's `merge` attribute.
fn routes(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    tokens.next() == Some(GRAPH_PATH) && tokens.any(|t| t == attr())
}

pub fn has_entry(text: &str) -> bool {
    text.lines().any(routes)
}

/// `text` with the entry appended; `None` when it is already there.
pub fn add(text: &str) -> Option<String> {
    if has_entry(text) {
        return None;
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&line());
    out.push('\n');
    Some(out)
}

/// `text` without the driver's attribute: a line that only routed the graph goes, a line with
/// other attributes for the graph keeps them. `None` when there was nothing to remove.
pub fn remove(text: &str) -> Option<String> {
    if !has_entry(text) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if !routes(line) {
            out.push_str(line);
            continue;
        }
        let rest: Vec<&str> = line
            .split_whitespace()
            .skip(1)
            .filter(|t| *t != attr())
            .collect();
        if !rest.is_empty() {
            out.push_str(GRAPH_PATH);
            for t in rest {
                out.push(' ');
                out.push_str(t);
            }
            out.push('\n');
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_remove_round_trip() {
        assert_eq!(
            add("").as_deref(),
            Some("dijkstral-out/graph.json merge=dijkstral\n")
        );
        let added = add("*.png binary").unwrap();
        assert_eq!(
            added,
            "*.png binary\ndijkstral-out/graph.json merge=dijkstral\n"
        );
        assert!(add(&added).is_none());
        assert_eq!(remove(&added).as_deref(), Some("*.png binary\n"));
        assert!(remove("*.png binary\n").is_none());
    }

    #[test]
    fn foreign_attributes_and_lookalikes_survive() {
        let text = "  dijkstral-out/graph.json   merge=dijkstral -diff\n\
                    graph.json merge=dijkstral\n\
                    dijkstral-out/graph.json merge=dijkstralx\n";
        assert!(has_entry(text));
        assert_eq!(
            remove(text).unwrap(),
            "dijkstral-out/graph.json -diff\ngraph.json merge=dijkstral\n\
             dijkstral-out/graph.json merge=dijkstralx\n"
        );
        assert!(!has_entry("# dijkstral-out/graph.json merge=dijkstral\n"));
    }
}
