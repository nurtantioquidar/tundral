//! Solutions: `.sln` (line format) and `.slnx` (XML) (§43.2).
//!
//! Every project entry becomes the project's node, labelled with its solution name, and a
//! `contains` edge from the solution; `ProjectDependencies` (`.sln`) and `BuildDependency`
//! (`.slnx`) become `imports` between projects. An `.sln` entry whose path equals its name is a
//! solution folder: its id and `source_file` are the name itself, not joined to the solution's
//! directory.

use super::{
    add_null_file_node, add_project, project_path, unlocated_edge, DotnetFacts, ProjectPath,
};
use crate::extract::xml;
use crate::extract::Ctx;
use crate::facts::LangFacts;
use crate::ids::file_id;

/// `Project("{type}") = "name", "path", "{guid}"`, any spacing around the punctuation.
fn project_line(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.trim_start().strip_prefix("Project(\"{")?;
    let (_, rest) = rest.split_once("}\")")?;
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let (name, rest) = quoted(rest)?;
    let rest = rest.trim_start().strip_prefix(',')?.trim_start();
    let (path, rest) = quoted(rest)?;
    let rest = rest.trim_start().strip_prefix(',')?.trim_start();
    let (guid, _) = quoted(rest)?;
    let guid = guid.strip_prefix('{')?.strip_suffix('}')?;
    Some((name, path, guid))
}

/// `"text"` at the start of `s`: the text and what follows the closing quote.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let s = s.strip_prefix('"')?;
    let end = s.find('"')?;
    Some((&s[..end], &s[end + 1..]))
}

/// `{GUID} = {GUID}` inside a `ProjectDependencies` section.
fn dependency_line(line: &str) -> Option<&str> {
    let (left, _) = line.trim().split_once('=')?;
    left.trim().strip_prefix('{')?.strip_suffix('}')
}

pub fn extract_sln(cx: &mut Ctx) {
    add_null_file_node(cx);
    let file = cx.file_id.clone();
    let text = String::from_utf8_lossy(cx.src).into_owned();
    let mut projects = Vec::new();
    let mut by_guid: Vec<(String, String)> = Vec::new();
    // (project id, dependency guids) in file order.
    let mut deps: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<String> = None;
    let mut in_deps = false;
    for line in text.lines() {
        if let Some((name, path, guid)) = project_line(line) {
            let p = if path == name {
                ProjectPath {
                    id: file_id(path),
                    label: name.to_string(),
                    source_file: path.to_string(),
                }
            } else {
                project_path(cx.rel, path)
            };
            add_project(cx, &mut projects, &p, name);
            let e = unlocated_edge(cx, &file, &p.id, "contains");
            cx.facts.edges.push(e);
            by_guid.push((guid.to_ascii_uppercase(), p.id.clone()));
            current = Some(p.id);
            continue;
        }
        let t = line.trim();
        if t.starts_with("EndProject") && !t.starts_with("EndProjectSection") {
            current = None;
            in_deps = false;
        } else if t.starts_with("ProjectSection(ProjectDependencies)") {
            in_deps = current.is_some();
        } else if t.starts_with("EndProjectSection") {
            in_deps = false;
        } else if in_deps {
            if let (Some(cur), Some(g)) = (&current, dependency_line(t)) {
                match deps.last_mut() {
                    Some((id, list)) if id == cur => list.push(g.to_ascii_uppercase()),
                    _ => deps.push((cur.clone(), vec![g.to_ascii_uppercase()])),
                }
            }
        }
    }
    for (from, guids) in deps {
        for g in guids {
            if let Some((_, to)) = by_guid.iter().find(|(x, _)| *x == g) {
                let e = unlocated_edge(cx, &from, to, "imports");
                cx.facts.edges.push(e);
            }
        }
    }
    cx.facts.lang = LangFacts::Dotnet(DotnetFacts {
        projects,
        ..Default::default()
    });
}

pub fn extract_slnx(cx: &mut Ctx) {
    let Some(root) = xml::parse(cx.src) else {
        return;
    };
    add_null_file_node(cx);
    let file = cx.file_id.clone();
    let mut projects = Vec::new();
    let mut imports: Vec<(String, String)> = Vec::new();
    for e in root.descendants() {
        if e.name != "Project" {
            continue;
        }
        let Some(path) = e.attr("Path") else {
            continue;
        };
        let p = project_path(cx.rel, path);
        // The label is the path's last `/` segment without its extension; backslashes stay.
        let base = path.rsplit('/').next().unwrap_or(path);
        let label = match base.rfind('.') {
            Some(i) if i > 0 => &base[..i],
            _ => base,
        };
        add_project(cx, &mut projects, &p, label);
        let edge = unlocated_edge(cx, &file, &p.id, "contains");
        cx.facts.edges.push(edge);
        for d in e.children.iter().filter(|c| c.name == "BuildDependency") {
            if let Some(dep) = d.attr("Project") {
                imports.push((p.id.clone(), project_path(cx.rel, dep).id));
            }
        }
    }
    for (from, to) in imports {
        let e = unlocated_edge(cx, &from, &to, "imports");
        cx.facts.edges.push(e);
    }
    cx.facts.lang = LangFacts::Dotnet(DotnetFacts {
        projects,
        ..Default::default()
    });
}
