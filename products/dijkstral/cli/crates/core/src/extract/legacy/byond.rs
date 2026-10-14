//! BYOND interface files: `.dmf` (skin), `.dmi` (icon), `.dmm` (map) (§44.6).
//!
//! `.dmf`: `window "w"` blocks become `window "w"` nodes, their named `elem "e"` children
//! `elem "e" [TYPE]` nodes (`type = …` inside the element). `.dmi`: a PNG whose `tEXt`
//! `Description` chunk holds the DMI header; each `state = "s"` becomes a `"s"` node, located
//! by its line in that text. `zTXt` (compressed) headers are not read (dijkstral has no inflate;
//! §44.7). `.dmm`: the file node only.

use super::{def, file_node};
use crate::extract::Ctx;

pub fn extract(cx: &mut Ctx) {
    file_node(cx, vec![]);
    let ext = cx.rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "dmf" => skin(cx),
        "dmi" => icon(cx),
        _ => {}
    }
}

/// `keyword "name"` → the name.
fn quoted<'l>(t: &'l str, keyword: &str) -> Option<&'l str> {
    let rest = t.strip_prefix(keyword)?.trim_start();
    let rest = rest.strip_prefix('"')?;
    rest.split_once('"').map(|(n, _)| n)
}

fn skin(cx: &mut Ctx) {
    let file = cx.file_id.clone();
    let text = String::from_utf8_lossy(cx.src).into_owned();
    let mut window: Option<(String, String)> = None; // (id, name)
                                                     // The element being read: (id, label head, line, type).
    let mut elem: Option<(String, String, u32, Option<String>)> = None;
    let flush = |cx: &mut Ctx,
                 elem: &mut Option<(String, String, u32, Option<String>)>,
                 window: &Option<(String, String)>| {
        if let (Some((id, head, line, ty)), Some((wid, _))) = (elem.take(), window) {
            let label = match ty {
                Some(t) => format!("{head} [{t}]"),
                None => head,
            };
            def(cx, &id, &label, line, vec![]);
            cx.add_edge(wid, &id, "contains", line, None);
        }
    };
    for (i, raw) in text.split('\n').enumerate() {
        let line = i as u32 + 1;
        let indented = raw.starts_with([' ', '\t']);
        let t = raw.trim();
        if !indented && !t.is_empty() {
            flush(cx, &mut elem, &window);
            window = None;
            if let Some(name) = quoted(t, "window") {
                let id = Ctx::child_id(&file, &format!("window_{name}"));
                def(cx, &id, &format!("window \"{name}\""), line, vec![]);
                cx.add_edge(&file, &id, "contains", line, None);
                window = Some((id, name.to_string()));
            }
            continue;
        }
        if let Some(name) = quoted(t, "elem") {
            flush(cx, &mut elem, &window);
            if let Some((_, wname)) = &window {
                let id = Ctx::child_id(&file, &format!("elem_{wname}_{name}"));
                elem = Some((id, format!("elem \"{name}\""), line, None));
            }
        } else if t.starts_with("elem") {
            flush(cx, &mut elem, &window);
        } else if let Some(v) = t.strip_prefix("type") {
            if let (Some(v), Some(e)) = (v.trim_start().strip_prefix('='), elem.as_mut()) {
                if e.3.is_none() {
                    e.3 = Some(v.trim().to_string());
                }
            }
        }
    }
    flush(cx, &mut elem, &window);
}

/// The `tEXt` chunk with keyword `Description` of a PNG.
fn description(src: &[u8]) -> Option<String> {
    let mut at = src.strip_prefix(b"\x89PNG\r\n\x1a\n").map(|_| 8)?;
    while at + 8 <= src.len() {
        let len = u32::from_be_bytes(src[at..at + 4].try_into().ok()?) as usize;
        let kind = &src[at + 4..at + 8];
        let data = src.get(at + 8..at + 8 + len)?;
        if kind == b"tEXt" {
            if let Some(rest) = data.strip_prefix(b"Description\0") {
                return Some(String::from_utf8_lossy(rest).into_owned());
            }
        }
        at += 12 + len;
    }
    None
}

fn icon(cx: &mut Ctx) {
    let file = cx.file_id.clone();
    let Some(text) = description(cx.src) else {
        return;
    };
    for (i, raw) in text.split('\n').enumerate() {
        let t = raw.trim();
        let Some(v) = t.strip_prefix("state").map(str::trim_start) else {
            continue;
        };
        let Some(v) = v.strip_prefix('=') else {
            continue;
        };
        let v = v.trim();
        let Some(name) = v
            .strip_prefix('"')
            .and_then(|r| r.split_once('"'))
            .map(|(n, _)| n)
        else {
            continue;
        };
        let line = i as u32 + 1;
        let id = Ctx::child_id(&file, &format!("state_{name}"));
        def(cx, &id, &format!("\"{name}\""), line, vec![]);
        cx.add_edge(&file, &id, "contains", line, None);
    }
}
