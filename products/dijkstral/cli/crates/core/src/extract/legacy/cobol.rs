//! COBOL: `.cob`, `.cbl`, `.cpy` (fixed format) (§44.5).
//!
//! `PROGRAM-ID. X.` becomes `<file.ext>_program_<x>`; level-numbered data items become
//! `…_data_<name>` (kind `data_item`) of the program, or of the file in a copybook; area-A names
//! in the PROCEDURE DIVISION become paragraphs and sections (`…_paragraph_<name>`); `PERFORM X
//! [THRU Y]` calls the program's paragraphs. Every node carries `metadata {"language":"cobol",
//! "kind":…}`; programs, paragraphs and sections are `_callable`.

use super::{def, file_node};
use crate::extract::{flag, slug, Ctx};
use crate::facts::Attr;

fn meta(kind: &str) -> (String, Attr) {
    (
        "metadata".into(),
        Attr::Json(format!("{{\"language\":\"cobol\",\"kind\":\"{kind}\"}}")),
    )
}

struct Para {
    name: String,
    id: String,
}

/// Content of a fixed-format line (columns 8–72) and whether it starts in area A; `None` for
/// comment lines.
fn content(raw: &str) -> Option<(&str, bool)> {
    let chars: Vec<(usize, char)> = raw.char_indices().collect();
    if chars.len() > 6 && matches!(chars[6].1, '*' | '/') {
        return None;
    }
    let body = if chars.len() > 7 {
        &raw[chars[7].0..]
    } else {
        ""
    };
    let lead = body.len() - body.trim_start().len();
    Some((body.trim_end(), lead < 4))
}

pub fn extract(cx: &mut Ctx) {
    file_node(cx, vec![meta("file")]);
    let file = cx.file_id.clone();
    let prefix = slug(cx.rel);
    let text = String::from_utf8_lossy(cx.src).into_owned();
    let raw_lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();

    // Programs: (id, first line index, end line index).
    let mut programs: Vec<(Option<String>, usize)> = vec![(None, 0)];
    for (i, raw) in raw_lines.iter().enumerate() {
        let Some((body, _)) = content(raw) else {
            continue;
        };
        let upper = body.trim().to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("PROGRAM-ID") {
            let rest = rest.trim_start_matches(['.', ' ']);
            let name: String = body.trim()["PROGRAM-ID".len()..]
                .trim_start_matches(['.', ' '])
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !name.is_empty() && !rest.is_empty() {
                let id = format!("{prefix}_program_{}", slug(&name));
                let line = i as u32 + 1;
                def(
                    cx,
                    &id,
                    &name,
                    line,
                    vec![meta("program"), flag("_callable")],
                );
                cx.add_edge(&file, &id, "contains", line, None);
                programs.push((Some(id), i));
            }
        }
    }

    for (p, (program, start)) in programs.iter().enumerate() {
        let end = programs.get(p + 1).map_or(raw_lines.len(), |(_, s)| *s);
        let start = if p == 0 { 0 } else { *start };
        let owner = program.clone().unwrap_or(file.clone());
        let base = program.clone().unwrap_or(prefix.clone());
        let mut in_procedure = false;
        let mut paras: Vec<Para> = Vec::new();
        let mut owner_of_line: Vec<Option<usize>> = vec![None; end - start];
        for (k, raw) in raw_lines[start..end].iter().enumerate() {
            let i = start + k;
            let Some((body, area_a)) = content(raw) else {
                continue;
            };
            let t = body.trim();
            let upper = t.to_ascii_uppercase();
            if upper.contains(" DIVISION") {
                in_procedure = upper.starts_with("PROCEDURE DIVISION");
                continue;
            }
            let line = i as u32 + 1;
            if in_procedure {
                if area_a && t.ends_with('.') {
                    let head = t.trim_end_matches('.').trim();
                    let (name, kind) = match head.split_whitespace().collect::<Vec<_>>().as_slice()
                    {
                        [n] => (*n, "paragraph"),
                        [n, s] if s.eq_ignore_ascii_case("SECTION") => (*n, "section"),
                        _ => ("", ""),
                    };
                    if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '-') {
                        let id = format!("{base}_paragraph_{}", slug(name));
                        if def(cx, &id, name, line, vec![meta(kind), flag("_callable")]) {
                            cx.add_edge(&owner, &id, "contains", line, None);
                        }
                        paras.push(Para {
                            name: name.to_ascii_uppercase(),
                            id,
                        });
                        continue;
                    }
                }
                owner_of_line[k] = paras.len().checked_sub(1);
            } else if program.is_some() || p == 0 {
                // Data items: `NN NAME`.
                let mut it = t.split_whitespace();
                let (Some(level), Some(name)) = (it.next(), it.next()) else {
                    continue;
                };
                if level.len() <= 2
                    && level.chars().all(|c| c.is_ascii_digit())
                    && !name.eq_ignore_ascii_case("FILLER")
                {
                    let name = name.trim_end_matches('.');
                    if name.is_empty() {
                        continue;
                    }
                    let id = format!("{base}_data_{}", slug(name));
                    if def(cx, &id, name, line, vec![meta("data_item")]) {
                        cx.add_edge(&owner, &id, "contains", line, None);
                    }
                }
            }
        }
        // PERFORM targets, resolved within the program.
        for (k, raw) in raw_lines[start..end].iter().enumerate() {
            let Some(caller) = owner_of_line[k].map(|p| paras[p].id.clone()) else {
                continue;
            };
            let Some((body, _)) = content(raw) else {
                continue;
            };
            let words: Vec<String> = body
                .split_whitespace()
                .map(|w| w.trim_end_matches('.').to_ascii_uppercase())
                .collect();
            let line = (start + k) as u32 + 1;
            for (w, word) in words.iter().enumerate() {
                if word != "PERFORM" {
                    continue;
                }
                let mut targets = vec![words.get(w + 1)];
                if matches!(
                    words.get(w + 2).map(String::as_str),
                    Some("THRU" | "THROUGH")
                ) {
                    targets.push(words.get(w + 3));
                }
                for t in targets.into_iter().flatten() {
                    if let Some(p) = paras.iter().find(|p| p.name == *t) {
                        cx.add_edge(&caller, &p.id, "calls", line, None);
                    }
                }
            }
        }
    }
}
