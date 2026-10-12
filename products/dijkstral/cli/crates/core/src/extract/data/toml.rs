//! `Cargo.toml` and `pyproject.toml` (§14.2, §11.6): one package node, no file node.

use toml::{Table, Value};

use super::{DataFacts, Package};
use crate::extract::{origin, slug, Ctx};
use crate::facts::{Attr, LangFacts, Loc, Node, NodeKind};

pub fn extract(cx: &mut Ctx) {
    let Some(table) = std::str::from_utf8(cx.src)
        .ok()
        .and_then(|s| s.parse::<Table>().ok())
    else {
        return;
    };
    let cargo = cx.rel.rsplit('/').next() == Some("Cargo.toml");
    let found = if cargo {
        cargo_package(&table)
    } else {
        python_package(&table)
    };
    let Some((name, version, deps)) = found else {
        return;
    };
    let key = slug(name);
    if key.is_empty() {
        return;
    }
    let id = format!("pkg_{key}");
    let mut attrs = vec![
        ("type".to_string(), Attr::Str("package".into())),
        (
            "ecosystem".to_string(),
            Attr::Str(if cargo { "cargo" } else { "python" }.into()),
        ),
    ];
    if let Some(v) = version {
        attrs.push(("version".into(), Attr::Str(v.into())));
    }
    attrs.push(origin());
    cx.facts.nodes.push(Node {
        id: id.clone(),
        label: name.to_string(),
        kind: NodeKind::Def,
        file_type: "code".into(),
        source_file: cx.rel.into(),
        loc: Loc::Line(1),
        attrs,
    });
    cx.facts.lang = LangFacts::Data(DataFacts {
        package: Some(Package {
            id,
            key,
            deps: deps
                .iter()
                .map(|d| slug(d))
                .filter(|d| !d.is_empty())
                .collect(),
        }),
        ..Default::default()
    });
}

type Found<'a> = (&'a str, Option<&'a str>, Vec<&'a str>);

fn get<'a>(t: &'a Table, path: &[&str]) -> Option<&'a Value> {
    let (last, init) = path.split_last()?;
    let mut cur = t;
    for k in init {
        cur = cur.get(*k)?.as_table()?;
    }
    cur.get(*last)
}

fn str_at<'a>(t: &'a Table, path: &[&str]) -> Option<&'a str> {
    get(t, path)?.as_str()
}

fn keys(v: Option<&Value>) -> impl Iterator<Item = &str> {
    v.and_then(Value::as_table)
        .into_iter()
        .flat_map(|t| t.keys().map(String::as_str))
}

/// `[package] name`; `depends_on` candidates come from `[dependencies]` and
/// `[target.<cfg>.dependencies]` only (not dev, build or workspace dependencies).
fn cargo_package(t: &Table) -> Option<Found<'_>> {
    let name = str_at(t, &["package", "name"])?;
    let version = str_at(t, &["package", "version"]);
    let mut deps: Vec<&str> = keys(t.get("dependencies")).collect();
    if let Some(targets) = t.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            deps.extend(keys(target.get("dependencies")));
        }
    }
    Some((name, version, deps))
}

/// `[project]` wins over `[tool.poetry]`, field by field. Dependencies are PEP 508 names from
/// `[project] dependencies` plus `[tool.poetry.dependencies]` keys.
fn python_package(t: &Table) -> Option<Found<'_>> {
    let name =
        str_at(t, &["project", "name"]).or_else(|| str_at(t, &["tool", "poetry", "name"]))?;
    let version =
        str_at(t, &["project", "version"]).or_else(|| str_at(t, &["tool", "poetry", "version"]));
    let mut deps: Vec<&str> = get(t, &["project", "dependencies"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(requirement_name)
        .collect();
    deps.extend(keys(get(t, &["tool", "poetry", "dependencies"])));
    Some((name, version, deps))
}

/// The distribution name at the start of a PEP 508 requirement (`q-pkg[extra] ; …` → `q-pkg`).
fn requirement_name(req: &str) -> &str {
    let req = req.trim_start();
    let end = req
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
        .unwrap_or(req.len());
    &req[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract_file;
    use crate::lang::Lang;

    fn facts(rel: &str, src: &str) -> (Vec<Node>, Option<Package>) {
        let f = extract_file(Lang::Toml, rel, src.as_bytes());
        let pkg = match f.lang {
            LangFacts::Data(d) => d.package,
            _ => None,
        };
        (f.nodes, pkg)
    }

    #[test]
    fn cargo_package_and_dependencies() {
        let src = r#"
[package]
name = "a-crate"
version.workspace = true

[dependencies]
b_crate = { path = "../b" }
renamed = { package = "c", version = "1" }

[dependencies.d-crate]
path = "../d"

[target.'cfg(unix)'.dependencies]
e-crate = "1"

[dev-dependencies]
f-crate = "1"
"#;
        let (nodes, pkg) = facts("a/Cargo.toml", src);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, "pkg_a_crate");
        assert_eq!(nodes[0].label, "a-crate");
        assert!(nodes[0].attr("version").is_none());
        let mut deps = pkg.unwrap().deps;
        deps.sort();
        assert_eq!(deps, ["b_crate", "d_crate", "e_crate", "renamed"]);
    }

    #[test]
    fn workspace_roots_and_invalid_toml_yield_nothing() {
        assert!(facts("Cargo.toml", "[workspace]\nmembers = []\n")
            .0
            .is_empty());
        assert!(facts("Cargo.toml", "[package]\nname = \"x\"\n[package]\n")
            .0
            .is_empty());
        assert!(facts("pyproject.toml", "[tool.ruff]\nx = 1\n").0.is_empty());
    }

    #[test]
    fn pyproject_prefers_project_then_poetry() {
        let src = r#"
[tool.poetry]
name = "s-poetry"
version = "1"
[tool.poetry.dependencies]
python = "^3.10"
[project]
name = "s-project"
dependencies = ["a-crate>=1", "q-pkg[extra] ; python_version>'3'", "Poetry.Pkg==1"]
"#;
        let (nodes, pkg) = facts("pyproject.toml", src);
        assert_eq!(nodes[0].id, "pkg_s_project");
        assert_eq!(nodes[0].attr("version"), Some(&Attr::Str("1".into())));
        assert_eq!(
            pkg.unwrap().deps,
            ["a_crate", "q_pkg", "poetry_pkg", "python"]
        );
    }
}
