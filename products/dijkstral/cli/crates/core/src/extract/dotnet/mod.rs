//! .NET project and markup files in a code-only build (`docs/graphify-format.md` §43): MSBuild
//! projects (`.csproj`, `.fsproj`, `.vbproj`), solutions (`.sln`, `.slnx`), Razor (`.razor`,
//! `.cshtml`) and XAML (`.xaml`).
//!
//! Project files are read with the strict XML reader in `crate::extract::xml` (a file that is not
//! well-formed yields nothing), solutions and Razor with line scanners. A project another file
//! names (a solution entry, a `ProjectReference`) is emitted as a node by the naming file with
//! the project's own path as `source_file`, so every naming of one project merges into a single
//! node; `crate::resolve::dotnet` then gives that node the shortest of its names. What Razor and
//! XAML refer to in C# is recorded in [`DotnetFacts`] and bound there too.

mod msbuild;
pub mod razor;
mod sln;
pub mod xaml;

use serde::{Deserialize, Serialize};

pub use razor::RazorFacts;
pub use xaml::XamlFacts;

use super::{origin, slug, Ctx};
use crate::facts::{Attr, Edge, Loc, Node, NodeKind};
use crate::ids::file_id;
use crate::lang::Lang;

/// Facts the cross-file pass (`crate::resolve::dotnet`) needs from one file. Ids are local
/// (before collision renaming); translate them with `View::id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DotnetFacts {
    /// Ids of the project nodes this file emitted for other paths, in emission order.
    pub projects: Vec<String>,
    pub razor: Option<RazorFacts>,
    pub xaml: Option<XamlFacts>,
}

pub fn extract(cx: &mut Ctx) {
    match cx.lang {
        Lang::MsBuild => msbuild::extract(cx),
        Lang::Sln => sln::extract_sln(cx),
        Lang::Slnx => sln::extract_slnx(cx),
        Lang::Razor => razor::extract(cx),
        Lang::Xaml => xaml::extract(cx),
        _ => {}
    }
}

/// Where a project path written in `referrer` points, relative to the scan root.
pub(crate) struct ProjectPath {
    pub id: String,
    /// The basename (after the last `/`).
    pub label: String,
    pub source_file: String,
}

/// Join `written` (backslashes read as `/`) to the directory of `referrer` and normalise it
/// lexically (Python's `os.path.normpath`). Inside the root the id is the file id of that path;
/// a path that leaves the root keeps its `../` form and gets `ext_<slug(path)>`; an absolute path
/// is reduced to its basename.
pub(crate) fn project_path(referrer: &str, written: &str) -> ProjectPath {
    let written = written.replace('\\', "/");
    let path = if written.starts_with('/') {
        let base = written.rsplit('/').next().unwrap_or("").to_string();
        return outside(base);
    } else {
        let dir = referrer.rsplit_once('/').map_or("", |(d, _)| d);
        normpath(&if dir.is_empty() {
            written
        } else {
            format!("{dir}/{written}")
        })
    };
    if path == ".." || path.starts_with("../") {
        return outside(path);
    }
    ProjectPath {
        id: file_id(&path),
        label: basename(&path).to_string(),
        source_file: path,
    }
}

fn outside(path: String) -> ProjectPath {
    ProjectPath {
        id: format!("ext_{}", slug(&path)),
        label: basename(&path).to_string(),
        source_file: path,
    }
}

pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `os.path.normpath` for relative POSIX paths: drop empty and `.` segments, fold `x/..`.
pub(crate) fn normpath(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|s| *s != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    if out.is_empty() {
        ".".into()
    } else {
        out.join("/")
    }
}

/// A node of this file with `source_location: null` (project files locate nothing).
pub(crate) fn null_node(cx: &Ctx, id: String, label: String, file_type: &str) -> Node {
    Node {
        id,
        label,
        kind: NodeKind::Def,
        file_type: file_type.into(),
        source_file: cx.rel.into(),
        loc: Loc::Null,
        attrs: vec![origin()],
    }
}

/// The file node with `source_location: null` (solutions, projects, Razor).
pub(crate) fn add_null_file_node(cx: &mut Ctx) {
    cx.add_file_node(vec![]);
    if let Some(n) = cx.facts.nodes.last_mut() {
        n.loc = Loc::Null;
    }
}

/// A project node for a path this file names; recorded so the cross-file pass can pick its label.
pub(crate) fn add_project(cx: &mut Ctx, projects: &mut Vec<String>, p: &ProjectPath, label: &str) {
    if projects.contains(&p.id) {
        return;
    }
    projects.push(p.id.clone());
    cx.facts.nodes.push(Node {
        id: p.id.clone(),
        label: label.to_string(),
        kind: NodeKind::Def,
        file_type: "code".into(),
        source_file: p.source_file.clone(),
        loc: Loc::Null,
        attrs: vec![origin()],
    });
}

/// An EXTRACTED edge with no `source_location` (project files) and an optional context.
pub(crate) fn unlocated_edge(cx: &Ctx, source: &str, target: &str, relation: &str) -> Edge {
    cx.edge(source, target, relation, Loc::Absent, None)
}

/// `metadata` value from pre-serialized key/value JSON pairs, in order.
pub(crate) fn metadata(pairs: &[(&str, String)]) -> (String, Attr) {
    let body: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}:{v}", json(k)))
        .collect();
    (
        "metadata".into(),
        Attr::Json(format!("{{{}}}", body.join(","))),
    )
}

/// A JSON string literal.
pub(crate) fn json(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_paths_resolve_like_graphify() {
        let p = project_path("a/src/Core/Core.csproj", "..\\Old\\Old.csproj");
        assert_eq!(
            (p.id.as_str(), p.label.as_str(), p.source_file.as_str()),
            ("a_src_old_old", "Old.csproj", "a/src/Old/Old.csproj")
        );
        let p = project_path("sample.csproj", "..\\Domain\\Domain.csproj");
        assert_eq!(
            (p.id.as_str(), p.source_file.as_str()),
            ("ext_domain_domain_csproj", "../Domain/Domain.csproj")
        );
        let p = project_path("a/N.csproj", "/abs/Abs.csproj");
        assert_eq!(
            (p.id.as_str(), p.source_file.as_str()),
            ("ext_abs_csproj", "Abs.csproj")
        );
        assert_eq!(normpath("sub/http://x/./y/../W"), "sub/http:/x/W");
    }
}
