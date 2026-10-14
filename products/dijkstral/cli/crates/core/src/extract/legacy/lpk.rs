//! Lazarus packages: `.lpk` (XML) (§44.3).
//!
//! The package (`Package/Name Value=`, else the file stem) becomes `<file>_<name>`, contained by
//! the file; each `RequiredPkgs/*/PackageName Value=` an `imports` target `slug(name)` and each
//! `Files/*/UnitName Value=` a `contains` target `slug(unit)` (bound in the cross-file pass when
//! a Pascal file is named like the unit). Everything is at `L1`.

use super::{def, file_node, LegacyFacts};
use crate::extract::xml::{self, Element};
use crate::extract::{slug, Ctx};

fn value_of<'e>(e: &'e Element, child: &str) -> Option<&'e str> {
    e.children
        .iter()
        .find(|c| c.local() == child)
        .and_then(|c| c.attr("Value"))
}

pub fn extract(cx: &mut Ctx) -> LegacyFacts {
    let Some(root) = xml::parse(cx.src) else {
        return LegacyFacts::default();
    };
    file_node(cx, vec![]);
    let file = cx.file_id.clone();
    let all = root.descendants();
    let package = all.iter().find(|e| e.local() == "Package");
    let stem = cx
        .rel
        .rsplit('/')
        .next()
        .map(|b| b.rsplit_once('.').map_or(b, |(s, _)| s))
        .unwrap_or("")
        .to_string();
    let name = package
        .and_then(|p| value_of(p, "Name"))
        .map_or(stem, str::to_string);
    let pkg = Ctx::child_id(&file, &name);
    def(cx, &pkg, &name, 1, vec![]);
    let mut facts = LegacyFacts::default();
    let target = format!("{}{}|pkg", crate::extract::legacy::pascal::VIRTUAL, cx.rel);
    cx.add_edge(&file, &target, "contains", 1, None);
    facts.package = Some((pkg.clone(), target));
    let section = |name: &str| -> Vec<&Element> {
        package
            .and_then(|p| p.children.iter().find(|c| c.local() == name))
            .map(|s| s.children.iter().collect())
            .unwrap_or_default()
    };
    for item in section("RequiredPkgs") {
        if let Some(n) = value_of(item, "PackageName") {
            let id = slug(n);
            if !id.is_empty() {
                def(cx, &id, n, 1, vec![]);
                cx.add_edge(&pkg, &id, "imports", 1, Some("import"));
            }
        }
    }
    for item in section("Files") {
        if let Some(n) = value_of(item, "UnitName") {
            let id = slug(n);
            if id.is_empty() {
                continue;
            }
            def(cx, &id, n, 1, vec![]);
            let target = format!(
                "{}{}|{}",
                crate::extract::legacy::pascal::VIRTUAL,
                cx.rel,
                facts.units.len()
            );
            cx.add_edge(&pkg, &target, "contains", 1, None);
            facts.units.push((id, n.to_string(), target));
        }
    }
    facts
}
