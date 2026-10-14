//! Delphi / Lazarus form files: `.dfm`, `.lfm` (text form only) (§44.2).
//!
//! Every `object Name: TType` becomes a node for its *type* (`<file>_<ttype>`, one per type),
//! contained by the nearest enclosing `object` (else the file); `OnEvent = Handler` lines become
//! `Handler()` nodes the object `references` with context `event`. `inherited` and `inline`
//! blocks nest like objects but produce nothing themselves.

use super::{def, file_node};
use crate::extract::Ctx;

pub fn extract(cx: &mut Ctx) {
    file_node(cx, vec![]);
    let file = cx.file_id.clone();
    let text = String::from_utf8_lossy(cx.src).into_owned();
    // Enclosing blocks: the type node of an `object`, `None` for anything else that `end`s.
    let mut stack: Vec<Option<String>> = Vec::new();
    // Each edge once, at its first occurrence.
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut once = |cx: &mut Ctx, s: &str, t: &str, rel: &str, line: u32, ctx: Option<&str>| {
        if !edges.iter().any(|(a, b)| a == s && b == t) {
            edges.push((s.to_string(), t.to_string()));
            cx.add_edge(s, t, rel, line, ctx);
        }
    };
    for (i, raw) in text.split('\n').enumerate() {
        let line = i as u32 + 1;
        let t = raw.trim();
        let lower = t.to_ascii_lowercase();
        let kw = lower.split_whitespace().next().unwrap_or("");
        match kw {
            "object" | "inherited" | "inline" => {
                let ty = t
                    .split_once(':')
                    .map(|(_, ty)| ty.trim())
                    .and_then(|ty| ty.split_whitespace().next())
                    .unwrap_or("");
                if kw != "object" || ty.is_empty() {
                    stack.push(None);
                    continue;
                }
                let id = Ctx::child_id(&file, ty);
                def(cx, &id, ty, line, vec![]);
                let parent = stack
                    .iter()
                    .rev()
                    .flatten()
                    .next()
                    .cloned()
                    .unwrap_or(file.clone());
                once(cx, &parent, &id, "contains", line, None);
                stack.push(Some(id));
            }
            "item" => stack.push(None),
            _ if lower == "end" || lower.starts_with("end>") => {
                stack.pop();
            }
            _ => {
                let Some((key, value)) = t.split_once('=') else {
                    continue;
                };
                let (key, value) = (key.trim(), value.trim());
                let handler = value
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                    && value.chars().all(|c| c.is_alphanumeric() || c == '_');
                let Some(Some(owner)) = stack.last().cloned() else {
                    continue;
                };
                if key.starts_with("On") && handler {
                    let id = Ctx::child_id(&file, value);
                    def(cx, &id, &format!("{value}()"), line, vec![]);
                    once(cx, &owner, &id, "references", line, Some("event"));
                }
            }
        }
    }
}
