//! Shell scripts (§14.3): a file node, an entry node for top-level code, `defines` edges to
//! functions and to top-level declared variables, and `calls` between known functions.
//!
//! Calls are counted where a command runs as a statement: plain commands in any control-flow
//! position (conditions, bodies, case arms, pipelines, lists, subshells, groups) and command
//! substitutions in assignment values. Command arguments, test expressions, `for … in` lists, case
//! subjects, here-documents and redirections are not searched. A function's body is walked only
//! when it is a `{ … }` group; a call to the enclosing function itself is not an edge.
//!
//! tree-sitter error nodes are walked like blocks, but their contents are never "top level": a
//! damaged script still yields its functions and calls, not its variables.
//!
//! `source` paths may start with a variable (`"${ROOT}/lib/util.sh"`). The variable's last
//! top-level plain assignment before the `source` is evaluated when it is a literal path or the
//! script-directory idiom `$(dirname "${BASH_SOURCE[0]}")` (or `"$0"`) plus a literal suffix; an
//! assignment of any other shape makes the path unknown. A variable never assigned at top level
//! stands for the script's directory, except before a `..` segment.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node as TsNode;

use super::{line, parse, Call, DataFacts, Grammar, Sourced};
use crate::extract::{slug, Ctx};
use crate::facts::{Attr, LangFacts};

fn metadata(kind: &str) -> Vec<(String, Attr)> {
    vec![(
        "metadata".into(),
        Attr::Json(format!(r#"{{"language":"bash","kind":"{kind}"}}"#)),
    )]
}

pub fn extract(cx: &mut Ctx) {
    cx.add_file_node(metadata("file"));
    let entry = format!("{}__entry", slug(cx.rel));
    let base = cx.rel.rsplit('/').next().unwrap_or(cx.rel);
    cx.add_def(
        entry.clone(),
        format!("{base} script"),
        1,
        metadata("bash_entrypoint"),
    );
    let file_id = cx.file_id.clone();
    cx.add_edge(&file_id, &entry, "contains", 1, None);

    let Some(tree) = parse(Grammar::Bash, cx.src) else {
        return;
    };
    let root = tree.root_node();
    let mut functions = Vec::new();
    collect_functions(cx, root, &mut functions);
    let mut w = Walker {
        cx,
        local: functions.iter().cloned().collect(),
        callers: vec![(entry, Vec::new())],
        sources: Vec::new(),
        vars: HashMap::new(),
        scripts: Vec::new(),
        seen_scripts: HashSet::new(),
        substitution: 0,
    };
    w.statements(root, 0, root.kind() == "program");

    let Walker {
        cx,
        local,
        callers,
        sources,
        scripts,
        ..
    } = w;
    let mut calls = Vec::new();
    for (caller, sites) in &callers {
        let mut seen = HashSet::new();
        for (name, at) in sites {
            if !seen.insert(name.as_str()) {
                continue;
            }
            match local.get(name) {
                Some(target) if target == caller => {}
                Some(target) => cx.add_edge(caller, target, "calls", *at, Some("call")),
                None => calls.push(Call {
                    caller: caller.clone(),
                    name: name.clone(),
                    line: *at,
                }),
            }
        }
    }
    cx.facts.lang = LangFacts::Data(DataFacts {
        sources,
        functions,
        calls,
        scripts,
        ..Default::default()
    });
}

/// Every function definition in the file, `(name, id)`, in pre-order.
fn collect_functions(cx: &Ctx, node: TsNode, out: &mut Vec<(String, String)>) {
    if node.kind() == "function_definition" {
        if let Some(name) = node.child_by_field_name("name").map(|n| cx.text(n)) {
            if !slug(name).is_empty() {
                out.push((name.to_string(), Ctx::child_id(&cx.file_id, name)));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_functions(cx, child, out);
    }
}

struct Walker<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    /// Function name → id; with duplicate definitions the first id wins (they are equal anyway).
    local: HashMap<String, String>,
    /// Callers in order (the entry, then functions in pre-order) with their call sites.
    callers: Vec<(String, Vec<(String, u32)>)>,
    sources: Vec<Sourced>,
    /// Top-level plain assignments seen so far: the evaluated path, or `None` when the value
    /// has another shape.
    vars: HashMap<String, Option<String>>,
    scripts: Vec<Call>,
    seen_scripts: HashSet<(String, String)>,
    /// Depth of command substitutions being walked; scripts run there are not invocations.
    substitution: usize,
}

impl Walker<'_, '_> {
    /// Walk every child statement of `node`; `top` is true for the program's direct children.
    fn statements(&mut self, node: TsNode, caller: usize, top: bool) {
        let mut cursor = node.walk();
        let children: Vec<TsNode> = node.named_children(&mut cursor).collect();
        for child in children {
            self.statement(child, caller, top);
        }
    }

    fn statement(&mut self, node: TsNode, caller: usize, top: bool) {
        match node.kind() {
            "function_definition" => self.function(node, caller),
            "command" => self.command(node, caller),
            "declaration_command" => {
                let mut cursor = node.walk();
                let children: Vec<TsNode> = node.named_children(&mut cursor).collect();
                for child in children
                    .into_iter()
                    .filter(|c| c.kind() == "variable_assignment")
                {
                    if top {
                        self.variable(child);
                    }
                    self.value(child, caller);
                }
            }
            "variable_assignment" => {
                if top {
                    self.assign(node);
                }
                self.value(node, caller)
            }
            "variable_assignments" => self.statements(node, caller, top),
            "ERROR" => self.statements(node, caller, false),
            "for_statement" | "c_style_for_statement" => {
                if let Some(body) = node.child_by_field_name("body") {
                    self.statements(body, caller, false);
                }
            }
            "case_statement" => {
                let mut cursor = node.walk();
                let items: Vec<TsNode> = node
                    .named_children(&mut cursor)
                    .filter(|c| c.kind() == "case_item")
                    .collect();
                for item in items {
                    let mut c2 = item.walk();
                    let body: Vec<TsNode> = item
                        .named_children(&mut c2)
                        .filter(|c| !is_case_pattern(item, *c))
                        .collect();
                    for stmt in body {
                        self.statement(stmt, caller, false);
                    }
                }
            }
            "redirected_statement" => {
                if let Some(body) = node.child_by_field_name("body") {
                    self.statement(body, caller, false);
                }
            }
            "if_statement" | "elif_clause" | "else_clause" | "while_statement" | "do_group"
            | "compound_statement" | "subshell" | "pipeline" | "list" | "negated_command" => {
                self.statements(node, caller, false)
            }
            _ => {}
        }
    }

    fn function(&mut self, node: TsNode, caller: usize) {
        let Some(name) = node.child_by_field_name("name").map(|n| self.cx.text(n)) else {
            return;
        };
        if slug(name).is_empty() {
            return;
        }
        let id = Ctx::child_id(&self.cx.file_id, name);
        let at = line(node);
        self.cx.add_def(
            id.clone(),
            format!("{name}()"),
            at,
            metadata("bash_function"),
        );
        let parent = if caller == 0 {
            self.cx.file_id.clone()
        } else {
            self.callers[caller].0.clone()
        };
        self.cx.add_edge(&parent, &id, "defines", at, None);
        self.callers.push((id, Vec::new()));
        let me = self.callers.len() - 1;
        if let Some(body) = node.child_by_field_name("body") {
            if body.kind() == "compound_statement" {
                self.statements(body, me, false);
            }
        }
    }

    fn command(&mut self, node: TsNode, caller: usize) {
        let mut cursor = node.walk();
        let children: Vec<TsNode> = node.named_children(&mut cursor).collect();
        for child in children
            .iter()
            .filter(|c| c.kind() == "variable_assignment")
        {
            self.value(*child, caller);
        }
        let Some(head) = node
            .child_by_field_name("name")
            .and_then(|n| n.named_child(0))
        else {
            return;
        };
        let at = line(node);
        if head.kind() != "word" {
            if let Some(path) = script_path(self.cx, head) {
                self.script(caller, path, at);
            }
            return;
        }
        let name = self.cx.text(head);
        if matches!(name, "bash" | "sh" | "zsh") {
            if let Some(path) = node
                .child_by_field_name("argument")
                .and_then(|a| script_path(self.cx, a))
            {
                self.script(caller, path, at);
            }
            return;
        }
        if name.contains('/') || name.ends_with(".sh") || name.ends_with(".bash") {
            self.script(caller, name.to_string(), at);
            return;
        }
        if name == "source" || name == "." {
            if let Some(arg) = node.child_by_field_name("argument") {
                if let Some(path) = literal(self.cx, arg) {
                    self.sources.push(Sourced {
                        path,
                        line: at,
                        variable: false,
                    });
                } else if let Some(path) = self.expand(arg) {
                    self.sources.push(Sourced {
                        path,
                        line: at,
                        variable: true,
                    });
                }
            }
            return;
        }
        self.callers[caller].1.push((name.to_string(), at));
    }

    /// Command substitutions anywhere in an assignment's value run as statements, including
    /// substitutions nested inside them (`${N:-$(($(count) / 2))}`).
    fn value(&mut self, node: TsNode, caller: usize) {
        let mut cursor = node.walk();
        let children: Vec<TsNode> = node.named_children(&mut cursor).collect();
        for child in children {
            if child.kind() == "command_substitution" {
                self.substitution += 1;
                self.statements(child, caller, false);
                self.value(child, caller);
                self.substitution -= 1;
            } else {
                self.value(child, caller);
            }
        }
    }

    /// A script run as a command, once per (caller, path).
    fn script(&mut self, caller: usize, path: String, at: u32) {
        if self.substitution > 0 {
            return;
        }
        let caller = self.callers[caller].0.clone();
        if self.seen_scripts.insert((caller.clone(), path.clone())) {
            self.scripts.push(Call {
                caller,
                name: path,
                line: at,
            });
        }
    }

    /// Record a top-level plain assignment for `source` path expansion.
    fn assign(&mut self, node: TsNode) {
        let Some(name) = node
            .child_by_field_name("name")
            .filter(|n| n.kind() == "variable_name")
        else {
            return;
        };
        let value = node
            .child_by_field_name("value")
            .and_then(|v| eval_value(self.cx.text(v)));
        self.vars.insert(self.cx.text(name).to_string(), value);
    }

    /// A `source` argument of the form `"${VAR}/rest"`, `"$VAR/rest"` or `$VAR/rest`, expanded
    /// as described in the module docs.
    fn expand(&self, arg: TsNode) -> Option<String> {
        let mut cursor = arg.walk();
        let parts: Vec<TsNode> = arg.named_children(&mut cursor).collect();
        let [var, rest] = parts.as_slice() else {
            return None;
        };
        let rest_kind = match arg.kind() {
            "string" => "string_content",
            "concatenation" => "word",
            _ => return None,
        };
        if !matches!(var.kind(), "expansion" | "simple_expansion") || rest.kind() != rest_kind {
            return None;
        }
        let rest = self.cx.text(*rest);
        if !rest.starts_with('/') || rest.contains(['$', '`']) {
            return None;
        }
        let name = var.named_child(0).and_then(|n| match n.kind() {
            "variable_name" => Some(self.cx.text(n)),
            "subscript" => n.child_by_field_name("name").map(|v| self.cx.text(v)),
            _ => None,
        });
        match name.and_then(|n| self.vars.get(n)) {
            Some(Some(base)) => Some(format!("{base}{rest}")),
            Some(None) => None,
            None if rest.split('/').any(|s| s == "..") => None,
            None => Some(format!(".{rest}")),
        }
    }

    /// A top-level declared variable (`export`, `declare`, `readonly`, `local`, `typeset`).
    fn variable(&mut self, assignment: TsNode) {
        let Some(name) = assignment
            .child_by_field_name("name")
            .filter(|n| n.kind() == "variable_name")
            .map(|n| self.cx.text(n))
        else {
            return;
        };
        if slug(name).is_empty() {
            return;
        }
        let id = Ctx::child_id(&self.cx.file_id, name);
        let at = line(assignment);
        self.cx
            .add_def(id.clone(), name.to_string(), at, metadata("code"));
        let file_id = self.cx.file_id.clone();
        self.cx.add_edge(&file_id, &id, "defines", at, None);
    }
}

/// The end of the first `$(dirname "${BASH_SOURCE[0]}")` (also `${BASH_SOURCE}`, `$0`, quoted or
/// not) in `text`.
fn script_dir_idiom(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find("$(dirname") {
        let start = from + i + "$(dirname".len();
        from = start;
        let s = &text[start..];
        let t = s.trim_start();
        if t.len() == s.len() {
            continue;
        }
        let t = t.strip_prefix('"').unwrap_or(t);
        let Some(t) = t.strip_prefix('$') else {
            continue;
        };
        let braced = t.starts_with('{');
        let t = t.strip_prefix('{').unwrap_or(t);
        let t = if let Some(r) = t.strip_prefix("BASH_SOURCE") {
            r.strip_prefix("[0]").unwrap_or(r)
        } else if let Some(r) = t.strip_prefix('0') {
            r
        } else {
            continue;
        };
        let t = if braced {
            match t.strip_prefix('}') {
                Some(r) => r,
                None => continue,
            }
        } else {
            t
        };
        let t = t.strip_prefix('"').unwrap_or(t).trim_start();
        if let Some(r) = t.strip_prefix(')') {
            return Some(text.len() - r.len());
        }
    }
    None
}

/// Evaluate an assignment value to a path (relative to the script directory unless absolute).
/// A value mentioning the script-directory idiom evaluates to that directory plus the literal
/// `/…` run right after the idiom, whatever surrounds it (`$(cd "$(dirname "$0")/.." && pwd)`,
/// `${ROOT:-$(dirname "$0")/..}`); a value without `$`, quotes or `~` is a literal path; any
/// other value is unknown.
fn eval_value(text: &str) -> Option<String> {
    if let Some(end) = script_dir_idiom(text) {
        let suffix: String = text[end..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | ')' | '}' | '$' | '`'))
            .collect();
        return Some(if suffix.starts_with('/') {
            format!(".{suffix}")
        } else {
            ".".into()
        });
    }
    let t = text.trim();
    let t = t
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(t);
    (!t.is_empty() && !t.contains(['$', '`', '"', '\'', '~', ' '])).then(|| t.to_string())
}

/// The path of a script run as a command, relative to the running script's directory: a literal
/// word or string, or `${VAR}/rest` / `$VAR/rest` where the variable stands for that directory.
fn script_path(cx: &Ctx, node: TsNode) -> Option<String> {
    if let Some(text) = literal(cx, node) {
        return Some(text);
    }
    let mut cursor = node.walk();
    let parts: Vec<TsNode> = node.named_children(&mut cursor).collect();
    let [var, rest] = parts.as_slice() else {
        return None;
    };
    let rest_kind = match node.kind() {
        "string" => "string_content",
        "concatenation" => "word",
        _ => return None,
    };
    if !matches!(var.kind(), "expansion" | "simple_expansion") || rest.kind() != rest_kind {
        return None;
    }
    let rest = cx.text(*rest).strip_prefix('/')?;
    (!rest.is_empty() && !rest.contains(['$', '`'])).then(|| rest.to_string())
}

/// Whether `child` of a `case_item` is one of its patterns (the `value` field).
fn is_case_pattern(item: TsNode, child: TsNode) -> bool {
    let mut cursor = item.walk();
    let found = item
        .children_by_field_name("value", &mut cursor)
        .any(|v| v.id() == child.id());
    found
}

/// The literal text of a `source` argument: an unquoted word or a double-quoted string without
/// expansions. Single-quoted and dynamic paths are not followed.
fn literal(cx: &Ctx, arg: TsNode) -> Option<String> {
    let text = match arg.kind() {
        "word" => cx.text(arg),
        "string" => {
            let mut cursor = arg.walk();
            if arg
                .named_children(&mut cursor)
                .any(|c| c.kind() != "string_content")
            {
                return None;
            }
            let t = cx.text(arg);
            t.strip_prefix('"')?.strip_suffix('"')?
        }
        _ => return None,
    };
    (!text.is_empty() && !text.contains(['$', '`'])).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use crate::extract::data::DataFacts;
    use crate::extract::extract_file;
    use crate::facts::{FileFacts, LangFacts};
    use crate::lang::Lang;

    fn run(src: &str) -> (FileFacts, DataFacts) {
        let f = extract_file(Lang::Shell, "s/a.sh", src.as_bytes());
        let d = match &f.lang {
            LangFacts::Data(d) => d.clone(),
            _ => panic!("no facts"),
        };
        (f, d)
    }

    fn edges(f: &FileFacts, rel: &str) -> Vec<String> {
        f.edges
            .iter()
            .filter(|e| e.relation == rel)
            .map(|e| format!("{}->{}", e.source, e.target))
            .collect()
    }

    #[test]
    fn entry_functions_and_variables() {
        let src = "#!/bin/bash\nexport A=1 B\ndeclare -x D=\"$(fa)\"\nF=1\nfa() {\n  inner() { fb; }\n}\nfunction fb { :; }\nif x; then export Z=1; fi\n";
        let (f, _) = run(src);
        let ids: Vec<_> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "s_a",
                "s_a_sh__entry",
                "s_a_a",
                "s_a_d",
                "s_a_fa",
                "s_a_inner",
                "s_a_fb"
            ]
        );
        assert_eq!(
            edges(&f, "defines"),
            [
                "s_a->s_a_a",
                "s_a->s_a_d",
                "s_a->s_a_fa",
                "s_a_fa->s_a_inner",
                "s_a->s_a_fb"
            ]
        );
        assert_eq!(
            edges(&f, "calls"),
            ["s_a_sh__entry->s_a_fa", "s_a_inner->s_a_fb"]
        );
    }

    #[test]
    fn which_positions_count_as_calls() {
        let src = r#"
f() {
  case "$(g1)" in x) g2 ;; esac
  for i in $(g3); do g4; done
  [[ $(g5) ]] && echo "$(g6)" `g7`
  x=$(g8) cmd
  local v="${V:-$(g9)}"
  ( g10 ) | { g11; }
  trap g12 EXIT
  f
  g4
  cat <<X
g13
X
}
g1() { :; }; g2() { :; }; g3() { :; }; g4() { :; }; g5() { :; }; g6() { :; }
g7() { :; }; g8() { :; }; g9() { :; }; g10() { :; }; g11() { :; }; g12() { :; }
g13() { :; }
s() ( g1 )
"#;
        let (f, _) = run(src);
        let got: Vec<_> = f
            .edges
            .iter()
            .filter(|e| e.relation == "calls")
            .map(|e| format!("{}@{:?}", e.target.trim_start_matches("s_a_"), e.loc))
            .collect();
        assert_eq!(
            got,
            [
                "g2@Line(3)",
                "g4@Line(4)",
                "g8@Line(6)",
                "g9@Line(7)",
                "g10@Line(8)",
                "g11@Line(8)"
            ]
        );
    }

    #[test]
    fn sources_and_unresolved_calls_are_recorded() {
        let src = "source lib.sh\n. \"./sub/x.sh\"\nsource '$q.sh'\nsource \"$D/y.sh\"\nf() { log hi; log again; }\nlog2\n";
        let (_, d) = run(src);
        let paths: Vec<_> = d
            .sources
            .iter()
            .map(|s| (s.path.as_str(), s.line, s.variable))
            .collect();
        assert_eq!(
            paths,
            [
                ("lib.sh", 1, false),
                ("./sub/x.sh", 2, false),
                ("./y.sh", 4, true)
            ]
        );
        let calls: Vec<_> = d
            .calls
            .iter()
            .map(|c| (c.caller.as_str(), c.name.as_str(), c.line))
            .collect();
        assert_eq!(calls, [("s_a_sh__entry", "log2", 6), ("s_a_f", "log", 5)]);
    }

    #[test]
    fn variable_source_paths() {
        let src = r#"
R=$(dirname "${BASH_SOURCE[0]}")/..
source "${R}/a.sh"
C="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
source "${C}/b.sh"
K=${K:-$(dirname "$0")/..}
source "$K/c.sh"
Q=$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)
source "${Q}/d.sh"
G=$(git rev-parse --show-toplevel)
source "${G}/e.sh"
A=/opt/lib
source "${A}/f.sh"
source "${U}/g.sh"
source "${U}/../h.sh"
source "${U}"/i.sh
export X=$(dirname "$0")/..
source "${X}/j.sh"
"#;
        let (_, d) = run(src);
        let paths: Vec<_> = d.sources.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "./../a.sh",
                "./../../b.sh",
                "./../c.sh",
                "./d.sh",
                "/opt/lib/f.sh",
                "./g.sh",
                "./j.sh"
            ]
        );
    }

    #[test]
    fn script_invocations() {
        let src = "SR=$(dirname \"$0\")\n\"${SR}/a.sh\" x &\n./b.sh\nbash c/d.sh\nf() { $U/e.sh; }\nx=$(./g.sh)\necho h.sh\n";
        let (_, d) = run(src);
        let got: Vec<_> = d
            .scripts
            .iter()
            .map(|c| (c.caller.as_str(), c.name.as_str(), c.line))
            .collect();
        assert_eq!(
            got,
            [
                ("s_a_sh__entry", "a.sh", 2),
                ("s_a_sh__entry", "./b.sh", 3),
                ("s_a_sh__entry", "c/d.sh", 4),
                ("s_a_f", "e.sh", 5)
            ]
        );
    }

    #[test]
    fn error_nodes_keep_functions_but_not_variables() {
        // tree-sitter-bash cannot parse a backquoted comment; the rest of the file is an error.
        let src = "f() {\n  git ls \\\n    ':x' `# c` \\\n    \"$@\"\n}\nexport V=1\ng() { h; }\nh() { :; }\n";
        let (f, _) = run(src);
        let ids: Vec<_> = f.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"s_a_g") && ids.contains(&"s_a_h"), "{ids:?}");
        assert!(!ids.contains(&"s_a_v"), "{ids:?}");
        assert_eq!(edges(&f, "calls"), ["s_a_g->s_a_h"]);
    }
}
