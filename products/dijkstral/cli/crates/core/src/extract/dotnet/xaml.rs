//! XAML views: `.xaml` (§43.4).
//!
//! Read with the strict XML reader (a file that is not well-formed yields nothing). The root
//! element becomes `<file>_<tag>`; every element named by `Name` or `x:Name` becomes
//! `<file>_<name>`, contained by the root and referencing its tag as the concept `xaml_<tag>`.
//! `{Binding}` paths and `<Binding Path=…>` become `binding_<path>` concepts, converters
//! `binding_converter_<name>`. Edges from an unnamed element start at the root.
//!
//! What needs other files is bound by `crate::resolve::dotnet` through virtual edge targets: the
//! `x:Class` (the class of the `.xaml.cs` beside the file, else the file's own `<file>_<class>`
//! node), event handlers (methods of that class taking an `…Args` parameter), view models (an
//! explicit `DataContext`, else `<View>ViewModel` by naming convention, looked up among the C#
//! classes of the nearest `.csproj`'s directory) and bindings to the CommunityToolkit members
//! those view models generate.

use serde::{Deserialize, Serialize};

use super::DotnetFacts;
use crate::extract::xml::{self, local, Element, XmlAttr};
use crate::extract::{origin, slug, Ctx};
use crate::facts::{Confidence, LangFacts, Loc, Node, NodeKind};

/// Prefix of the virtual edge targets the cross-file pass redirects.
pub const VIRTUAL: &str = "@xaml:";
const XAML_NS: &str = "http://schemas.microsoft.com/winfx/2006/xaml";

/// Attributes whose value is never read as an event handler name.
const NOT_EVENTS: &[&str] = &[
    "Name",
    "Header",
    "ToolTip",
    "DataContext",
    "Source",
    "Uid",
    "Key",
    "Class",
    "Text",
    "Tag",
    "Content",
    "Title",
    "Style",
];

/// View class suffixes the naming convention strips before adding `ViewModel`, longest first.
pub const VIEW_SUFFIXES: &[&str] = &["UserControl", "Control", "Window", "View", "Page"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct XamlFacts {
    /// `x:Class` as written, the file's own node for it and the virtual target of its edge.
    pub class: Option<(String, String, String)>,
    /// The name the view-model convention starts from (`x:Class`'s last segment, else the file
    /// stem).
    pub view: String,
    /// Edges to bind, in emission order.
    pub refs: Vec<XamlRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XamlRef {
    pub target: String,
    pub kind: XamlKind,
    /// Handler method name, view-model class name, or binding path.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XamlKind {
    Event,
    /// An explicit `DataContext` type.
    ViewModel,
    /// The naming-convention view model (INFERRED).
    Inferred,
    /// A binding path, bound to a generated view-model member (INFERRED).
    Member,
}

/// Prefix → namespace URI scopes while walking the tree.
struct Scopes(Vec<Vec<(String, String)>>);

impl Scopes {
    fn push(&mut self, e: &Element) {
        self.0.push(
            e.attrs
                .iter()
                .filter_map(|a| {
                    a.name
                        .strip_prefix("xmlns:")
                        .map(|p| (p.to_string(), a.value.clone()))
                })
                .collect(),
        );
    }

    fn uri(&self, prefix: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .flat_map(|s| s.iter())
            .find(|(p, _)| p == prefix)
            .map(|(_, u)| u.as_str())
    }

    /// The attribute is `x:<local>` (the XAML language namespace).
    fn is_x(&self, a: &XmlAttr, want: &str) -> bool {
        match a.name.split_once(':') {
            Some((p, l)) => l == want && self.uri(p) == Some(XAML_NS),
            None => false,
        }
    }
}

struct Emit<'a, 'c> {
    cx: &'a mut Ctx<'c>,
    root: String,
    facts: XamlFacts,
    serial: usize,
}

impl Emit<'_, '_> {
    fn has(&self, id: &str) -> bool {
        self.cx.facts.nodes.iter().any(|n| n.id == id)
    }

    fn node(&mut self, id: &str, label: &str, line: u32, file_type: &str) {
        if self.has(id) {
            return;
        }
        self.cx.facts.nodes.push(Node {
            id: id.into(),
            label: label.into(),
            kind: NodeKind::Def,
            file_type: file_type.into(),
            source_file: self.cx.rel.into(),
            loc: Loc::Line(line),
            attrs: vec![origin()],
        });
    }

    fn edge(&mut self, source: &str, target: &str, line: u32, context: Option<&str>, rel: &str) {
        self.cx.add_edge(source, target, rel, line, context);
    }

    /// An edge whose target the cross-file pass binds (or drops).
    fn bound(&mut self, source: &str, kind: XamlKind, name: &str, line: u32, context: &str) {
        self.serial += 1;
        let target = format!("{VIRTUAL}{}|{}", self.cx.rel, self.serial);
        self.facts.refs.push(XamlRef {
            target: target.clone(),
            kind,
            name: name.into(),
        });
        let mut e = self.cx.edge(
            source,
            &target,
            "references",
            Loc::Line(line),
            Some(context),
        );
        if matches!(kind, XamlKind::Inferred | XamlKind::Member) {
            e.confidence = Confidence::Inferred;
        }
        self.cx.facts.edges.push(e);
    }
}

pub fn extract(cx: &mut Ctx) {
    let Some(root) = xml::parse(cx.src) else {
        return;
    };
    cx.add_file_node(vec![]);
    let file = cx.file_id.clone();
    let root_id = Ctx::child_id(&file, root.local());
    let stem = cx
        .rel
        .rsplit('/')
        .next()
        .map(|b| b.rsplit_once('.').map_or(b, |(s, _)| s))
        .unwrap_or("")
        .to_string();
    let mut em = Emit {
        cx,
        root: root_id.clone(),
        facts: XamlFacts::default(),
        serial: 0,
    };
    em.node(&root_id, root.local(), root.line, "code");
    em.edge(&file, &root_id, root.line, None, "contains");

    let mut scopes = Scopes(Vec::new());
    scopes.push(&root);
    let class = root
        .attrs
        .iter()
        .find(|a| scopes.is_x(a, "Class"))
        .map(|a| a.value.clone());
    em.facts.view = match &class {
        Some(c) => c.rsplit('.').next().unwrap_or(c).to_string(),
        None => stem,
    };
    if let Some(fqn) = class {
        let short = fqn.rsplit('.').next().unwrap_or(&fqn).to_string();
        let own = Ctx::child_id(&file, &short);
        em.node(&own, &short, root.line, "code");
        em.serial += 1;
        let target = format!("{VIRTUAL}{}|{}", em.cx.rel, em.serial);
        em.facts.class = Some((fqn, own, target.clone()));
        em.edge(&root_id, &target, root.line, Some("x_class"), "references");
    }

    // View models: an explicit DataContext anywhere, else the naming convention.
    let mut explicit = Vec::new();
    view_models(&root, &mut scopes, &mut explicit);
    if explicit.is_empty() {
        let view = em.facts.view.clone();
        em.bound(&root_id, XamlKind::Inferred, &view, root.line, "view_model");
    }
    for (name, line) in explicit {
        em.bound(&root_id, XamlKind::ViewModel, &name, line, "view_model");
    }

    walk(&mut em, &root, &mut scopes, true);
    let facts = em.facts;
    em.cx.facts.lang = LangFacts::Dotnet(DotnetFacts {
        xaml: Some(facts),
        ..Default::default()
    });
}

/// Explicit view-model types in document order: `DataContext="{d:DesignInstance Type=…}"`
/// attributes and the elements inside a `*.DataContext` property element.
fn view_models(e: &Element, scopes: &mut Scopes, out: &mut Vec<(String, u32)>) {
    for a in &e.attrs {
        if local(&a.name) == "DataContext" {
            if let Some(t) = design_instance_type(&a.value) {
                out.push((t, a.line));
            }
        }
    }
    let is_context = e.local().ends_with(".DataContext");
    for c in &e.children {
        scopes.push(c);
        if is_context {
            out.push((c.local().to_string(), c.line));
        }
        view_models(c, scopes, out);
        scopes.0.pop();
    }
}

/// The `Type=` of a `{d:DesignInstance …}` (`Type=vm:X` or `Type={x:Type vm:X}`), prefix removed.
fn design_instance_type(value: &str) -> Option<String> {
    if !value.contains("DesignInstance") {
        return None;
    }
    let at = value.find("Type=")?;
    let rest = value[at + 5..].trim_start();
    let rest = match rest.strip_prefix('{') {
        Some(inner) => inner.trim_start().strip_prefix("x:Type")?.trim_start(),
        None => rest,
    };
    let token: String = rest
        .chars()
        .take_while(|c| !matches!(c, ',' | '}' | ' ' | '\t' | '\n'))
        .collect();
    let name = local(&token);
    (!name.is_empty()).then(|| name.to_string())
}

/// The element's name: the first `Name` or `x:Name` attribute.
fn element_name<'e>(e: &'e Element, scopes: &Scopes) -> Option<&'e str> {
    e.attrs
        .iter()
        .find(|a| a.name == "Name" || scopes.is_x(a, "Name"))
        .map(|a| a.value.as_str())
}

fn walk(em: &mut Emit, e: &Element, scopes: &mut Scopes, is_root: bool) {
    let mut source = em.root.clone();
    if let Some(name) = element_name(e, scopes).filter(|n| !slug(n).is_empty()) {
        let file = em.cx.file_id.clone();
        let id = Ctx::child_id(&file, name);
        em.node(&id, name, e.line, "code");
        let root = em.root.clone();
        em.edge(&root, &id, e.line, None, "contains");
        let ty = Ctx::child_id("xaml", e.local());
        em.node(&ty, e.local(), e.line, "concept");
        em.edge(&id, &ty, e.line, Some("type"), "references");
        if !is_root {
            source = id;
        }
    }
    if e.local() == "Binding" {
        if let Some(a) = e.attrs.iter().find(|a| a.name == "Path") {
            binding_path(em, &source, a.value.trim(), a.line, "binding_path");
        }
        if let Some(a) = e.attrs.iter().find(|a| a.name == "Converter") {
            if let Some(c) = static_resource(&a.value) {
                converter(em, &source, &c, a.line);
            }
        }
    } else {
        for a in &e.attrs {
            attribute(em, &source, a, scopes);
        }
    }
    for c in &e.children {
        scopes.push(c);
        walk(em, c, scopes, false);
        scopes.0.pop();
    }
}

fn attribute(em: &mut Emit, source: &str, a: &XmlAttr, scopes: &Scopes) {
    let value = a.value.trim();
    if let Some(body) = markup(value, "Binding") {
        let args = split_args(body);
        let mut path = None;
        let mut conv = None;
        for (k, arg) in args.iter().enumerate() {
            match arg.split_once('=') {
                Some((key, v)) if !key.contains('{') => match key.trim() {
                    "Path" => path = Some(v.trim().to_string()),
                    "Converter" => conv = static_resource(v.trim()),
                    _ => {}
                },
                _ if k == 0 => path = Some(arg.trim().to_string()),
                _ => {}
            }
        }
        let context = if local(&a.name) == "Command" {
            "binding_command"
        } else {
            "binding_path"
        };
        if let Some(p) = path.filter(|p| !p.is_empty()) {
            binding_path(em, source, &p, a.line, context);
        }
        if let Some(c) = conv {
            converter(em, source, &c, a.line);
        }
        return;
    }
    // An identifier value on any attribute that is not a plain property may name a handler.
    let is_ident = value
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && value.chars().all(|c| c.is_alphanumeric() || c == '_');
    let ns_decl = a.name == "xmlns" || a.name.starts_with("xmlns:");
    let x_attr = a
        .name
        .split_once(':')
        .is_some_and(|(p, _)| scopes.uri(p) == Some(XAML_NS));
    if is_ident && !ns_decl && !x_attr && !NOT_EVENTS.contains(&local(&a.name)) {
        em.bound(source, XamlKind::Event, value, a.line, "event");
    }
}

fn binding_path(em: &mut Emit, source: &str, path: &str, line: u32, context: &str) {
    let id = Ctx::child_id("binding", path);
    if id == "binding" {
        return;
    }
    em.node(&id, path, line, "concept");
    em.edge(source, &id, line, Some(context), "references");
    em.bound(source, XamlKind::Member, path, line, context);
}

fn converter(em: &mut Emit, source: &str, name: &str, line: u32) {
    let id = Ctx::child_id("binding_converter", name);
    if id == "binding_converter" {
        return;
    }
    em.node(&id, name, line, "concept");
    em.edge(source, &id, line, Some("binding_converter"), "references");
}

/// The inside of `{Name …}` when `value` is that markup extension.
fn markup<'v>(value: &'v str, name: &str) -> Option<&'v str> {
    let inner = value.strip_prefix('{')?.strip_suffix('}')?;
    let rest = inner.strip_prefix(name)?;
    if rest.is_empty() || rest.starts_with([' ', '\t', '\n']) {
        Some(rest.trim())
    } else {
        None
    }
}

/// `{StaticResource Name}` → `Name`.
fn static_resource(value: &str) -> Option<String> {
    let body = markup(value.trim(), "StaticResource")?;
    let name = body.split([',', ' ']).next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Top-level comma-separated arguments of a markup extension body.
fn split_args(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < body.len() {
        out.push(&body[start..]);
    }
    out.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_design_instances_and_bindings() {
        assert_eq!(
            design_instance_type("{d:DesignInstance Type=vm:DesignViewModel}").as_deref(),
            Some("DesignViewModel")
        );
        assert_eq!(
            design_instance_type(
                "{d:DesignInstance Type={x:Type vm:B}, IsDesignTimeCreatable=True}"
            )
            .as_deref(),
            Some("B")
        );
        assert_eq!(design_instance_type("{d:DesignInstance vm:A}"), None);
        let body = markup(
            "{Binding Path=Order.Total, Converter={StaticResource Money}}",
            "Binding",
        );
        assert_eq!(
            split_args(body.unwrap()),
            ["Path=Order.Total", " Converter={StaticResource Money}"]
        );
        assert_eq!(markup("{Binding}", "Binding"), Some(""));
        assert_eq!(markup("{BindingX}", "Binding"), None);
    }
}
