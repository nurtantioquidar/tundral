//! MSBuild projects: `.csproj`, `.fsproj`, `.vbproj` (§43.1).
//!
//! Elements are matched by local name anywhere in the document, whatever their namespace:
//! `TargetFramework` then `TargetFrameworks` (split on `;`) become `framework_<tfm>` concept nodes,
//! `PackageReference Include=` becomes `nuget_<name>` labelled `Name (Version)` (the `Version`
//! attribute only), `ProjectReference Include=` the referenced project's node, and the root's
//! `Sdk` attribute an `sdk_<sdk>` concept node. Every node and edge is unlocated.

use super::{
    add_null_file_node, add_project, null_node, project_path, unlocated_edge, DotnetFacts,
};
use crate::extract::xml::{self, Element};
use crate::extract::{slug, Ctx};
use crate::facts::LangFacts;

pub fn extract(cx: &mut Ctx) {
    let Some(root) = xml::parse(cx.src) else {
        return;
    };
    add_null_file_node(cx);
    let file = cx.file_id.clone();
    let all = root.descendants();
    let named = |local: &str| -> Vec<&Element> {
        all.iter().copied().filter(|e| e.local() == local).collect()
    };

    let mut seen: Vec<String> = Vec::new();
    let mut frameworks: Vec<String> = Vec::new();
    for e in named("TargetFramework") {
        if let Some(t) = e.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            frameworks.push(t.to_string());
        }
    }
    for e in named("TargetFrameworks") {
        for t in e.text.as_deref().unwrap_or("").split(';') {
            let t = t.trim();
            if !t.is_empty() {
                frameworks.push(t.to_string());
            }
        }
    }
    for fw in frameworks {
        let id = format!("framework_{}", slug(&fw));
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        let n = null_node(cx, id.clone(), fw, "concept");
        cx.facts.nodes.push(n);
        let e = unlocated_edge(cx, &file, &id, "references");
        cx.facts.edges.push(e);
    }

    for e in named("PackageReference") {
        let Some(name) = e.attr("Include") else {
            continue;
        };
        let id = format!("nuget_{}", slug(name));
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        let label = match e.attr("Version").filter(|v| !v.is_empty()) {
            Some(v) => format!("{name} ({v})"),
            None => name.to_string(),
        };
        let n = null_node(cx, id.clone(), label, "code");
        cx.facts.nodes.push(n);
        let edge = unlocated_edge(cx, &file, &id, "imports");
        cx.facts.edges.push(edge);
    }

    let mut projects = Vec::new();
    for e in named("ProjectReference") {
        let Some(include) = e.attr("Include") else {
            continue;
        };
        let p = project_path(cx.rel, include);
        let label = p.label.clone();
        add_project(cx, &mut projects, &p, &label);
        let edge = unlocated_edge(cx, &file, &p.id, "imports");
        cx.facts.edges.push(edge);
    }

    if let Some(sdk) = root.attr("Sdk") {
        let id = format!("sdk_{}", slug(sdk));
        let n = null_node(cx, id.clone(), sdk.to_string(), "concept");
        cx.facts.nodes.push(n);
        let e = unlocated_edge(cx, &file, &id, "references");
        cx.facts.edges.push(e);
    }
    cx.facts.lang = LangFacts::Dotnet(DotnetFacts {
        projects,
        ..Default::default()
    });
}
