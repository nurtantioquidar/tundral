//! Per-language structural extraction via tree-sitter queries. Deterministic,
//! local, no LLM: definitions, name references (call sites), and imports.

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Java,
}

impl Lang {
    pub fn from_path(path: &std::path::Path) -> Option<Lang> {
        match path.extension()?.to_str()? {
            "rs" => Some(Lang::Rust),
            "py" => Some(Lang::Python),
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" => Some(Lang::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
            "java" => Some(Lang::Java),
            _ => None,
        }
    }

    fn language(self) -> Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
        }
    }

    fn query_source(self) -> &'static str {
        match self {
            Lang::Rust => RUST_QUERY,
            Lang::Python => PYTHON_QUERY,
            Lang::TypeScript | Lang::Tsx => TYPESCRIPT_QUERY,
            Lang::JavaScript => JAVASCRIPT_QUERY,
            Lang::Java => JAVA_QUERY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Struct,
    Enum,
    Trait,
    Class,
    Interface,
    TypeAlias,
    Const,
    Module,
    Field,
}

impl DefKind {
    pub fn label(self) -> &'static str {
        match self {
            DefKind::Function => "fn",
            DefKind::Struct => "struct",
            DefKind::Enum => "enum",
            DefKind::Trait => "trait",
            DefKind::Class => "class",
            DefKind::Interface => "interface",
            DefKind::TypeAlias => "type",
            DefKind::Const => "const",
            DefKind::Module => "mod",
            DefKind::Field => "field",
        }
    }

    fn from_capture(name: &str) -> Option<DefKind> {
        Some(match name {
            "fn" => DefKind::Function,
            "struct" => DefKind::Struct,
            "enum" => DefKind::Enum,
            "trait" => DefKind::Trait,
            "class" => DefKind::Class,
            "interface" => DefKind::Interface,
            "type" => DefKind::TypeAlias,
            "const" => DefKind::Const,
            "module" => DefKind::Module,
            "field" => DefKind::Field,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub name: String,
    pub kind: DefKind,
    /// Enclosing type/class/module name, when syntactically evident.
    pub container: Option<String>,
    /// 1-based line of the definition.
    pub line: usize,
    /// 1-based last line of the definition's node (its body included), so a
    /// caller can read exactly the definition's range. Equals `line` when the
    /// grammar gives no enclosing node.
    pub end_line: usize,
    /// The definition's first source line, trimmed and capped.
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
    pub imports: Vec<String>,
}

const RUST_QUERY: &str = r#"
(function_item name: (identifier) @fn)
(struct_item name: (type_identifier) @struct)
(enum_item name: (type_identifier) @enum)
(trait_item name: (type_identifier) @trait)
(const_item name: (identifier) @const)
(static_item name: (identifier) @const)
(mod_item name: (identifier) @module)
(call_expression function: (identifier) @ref)
(call_expression function: (field_expression field: (field_identifier) @ref))
(call_expression function: (scoped_identifier name: (identifier) @ref))
(macro_invocation macro: (identifier) @ref)
(use_declaration argument: (_) @import)
"#;

const PYTHON_QUERY: &str = r#"
(function_definition name: (identifier) @fn)
(class_definition name: (identifier) @class)
(call function: (identifier) @ref)
(call function: (attribute attribute: (identifier) @ref))
(import_statement name: (dotted_name) @import)
(import_from_statement module_name: (dotted_name) @import)
"#;

const TYPESCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @fn)
(method_definition name: (property_identifier) @fn)
(class_declaration name: (type_identifier) @class)
(interface_declaration name: (type_identifier) @interface)
(type_alias_declaration name: (type_identifier) @type)
(enum_declaration name: (identifier) @enum)
(variable_declarator name: (identifier) @fn value: (arrow_function))
(call_expression function: (identifier) @ref)
(call_expression function: (member_expression property: (property_identifier) @ref))
(new_expression constructor: (identifier) @ref)
(import_statement source: (string (string_fragment) @import))
"#;

const JAVA_QUERY: &str = r#"
(class_declaration name: (identifier) @class)
(interface_declaration name: (identifier) @interface)
(enum_declaration name: (identifier) @enum)
(record_declaration name: (identifier) @class)
(method_declaration name: (identifier) @fn)
(constructor_declaration name: (identifier) @fn)
(field_declaration declarator: (variable_declarator name: (identifier) @field))
(method_invocation name: (identifier) @ref)
(object_creation_expression type: (type_identifier) @ref)
(import_declaration (scoped_identifier) @import)
"#;

const JAVASCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @fn)
(method_definition name: (property_identifier) @fn)
(class_declaration name: (identifier) @class)
(variable_declarator name: (identifier) @fn value: (arrow_function))
(call_expression function: (identifier) @ref)
(call_expression function: (member_expression property: (property_identifier) @ref))
(new_expression constructor: (identifier) @ref)
(import_statement source: (string (string_fragment) @import))
"#;

/// Ancestor node kinds that name a container, per language family.
fn container_of(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        let name_field = match ancestor.kind() {
            "impl_item" => ancestor.child_by_field_name("type"),
            "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "mod_item"
            | "trait_item" => ancestor.child_by_field_name("name"),
            _ => None,
        };
        if let Some(name_node) = name_field {
            // Only report a container that is not the node itself (a class's
            // own name has the class as ancestor).
            if name_node.byte_range() != node.byte_range() {
                return name_node.utf8_text(source).ok().map(str::to_string);
            }
        }
        current = ancestor.parent();
    }
    None
}

fn signature_line(source: &str, line: usize) -> String {
    let text = source.lines().nth(line.saturating_sub(1)).unwrap_or("");
    let mut trimmed = text.trim().to_string();
    if trimmed.len() > 120 {
        let mut end = 120;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed.truncate(end);
        trimmed.push('…');
    }
    trimmed
}

/// Parse one file's source and extract definitions, references, imports.
/// Errors (grammar/query issues) are programming bugs and surface loudly in
/// tests; at runtime an unparseable file simply yields an empty extraction.
pub fn extract(lang: Lang, source: &str) -> Extraction {
    let mut parser = Parser::new();
    if parser.set_language(&lang.language()).is_err() {
        return Extraction::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Extraction::default();
    };
    let query = match Query::new(&lang.language(), lang.query_source()) {
        Ok(query) => query,
        Err(_) => return Extraction::default(),
    };
    let bytes = source.as_bytes();
    let mut extraction = Extraction::default();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), bytes);
    while let Some(matched) = matches.next() {
        for capture in matched.captures {
            let capture_name = &query.capture_names()[capture.index as usize];
            let node = capture.node;
            let Ok(text) = node.utf8_text(bytes) else {
                continue;
            };
            let line = node.start_position().row + 1;
            if *capture_name == "ref" {
                extraction.references.push(Reference {
                    name: text.to_string(),
                    line,
                });
            } else if *capture_name == "import" {
                extraction.imports.push(text.to_string());
            } else if let Some(kind) = DefKind::from_capture(capture_name) {
                // The capture is the name; the enclosing node is the
                // declaration whose extent the outline needs.
                let end_line = node
                    .parent()
                    .map(|parent| parent.end_position().row + 1)
                    .unwrap_or(line)
                    .max(line);
                extraction.definitions.push(Definition {
                    name: text.to_string(),
                    kind,
                    container: container_of(node, bytes),
                    line,
                    end_line,
                    signature: signature_line(source, line),
                });
            }
        }
    }
    extraction
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(defs: &[Definition], kind: DefKind) -> Vec<&str> {
        defs.iter()
            .filter(|d| d.kind == kind)
            .map(|d| d.name.as_str())
            .collect()
    }

    #[test]
    fn extracts_rust_shapes() {
        let source = r#"
use crate::domain::Money;

pub struct Ledger { total: u64 }

pub enum Kind { A, B }

pub trait Priced { fn price(&self) -> u64; }

impl Ledger {
    pub fn add(&mut self, amount: u64) {
        record(amount);
        self.validate();
    }
}

fn record(amount: u64) { println!("{amount}"); }

const CAP: u64 = 10;
"#;
        let out = extract(Lang::Rust, source);
        assert_eq!(names(&out.definitions, DefKind::Struct), vec!["Ledger"]);
        assert_eq!(names(&out.definitions, DefKind::Enum), vec!["Kind"]);
        assert_eq!(names(&out.definitions, DefKind::Trait), vec!["Priced"]);
        assert!(names(&out.definitions, DefKind::Function).contains(&"add"));
        assert!(names(&out.definitions, DefKind::Function).contains(&"record"));
        assert_eq!(names(&out.definitions, DefKind::Const), vec!["CAP"]);
        let add = out
            .definitions
            .iter()
            .find(|d| d.name == "add")
            .expect("add def");
        assert_eq!(add.container.as_deref(), Some("Ledger"));
        assert!(add.signature.contains("pub fn add"));
        let ref_names: Vec<&str> = out.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"record"));
        assert!(ref_names.contains(&"validate"));
        assert!(out.imports.iter().any(|i| i.contains("crate::domain")));
    }

    #[test]
    fn extracts_python_shapes() {
        let source = r#"
import os
from collections import OrderedDict

class Store:
    def add(self, item):
        self.validate(item)
        persist(item)

def persist(item):
    pass
"#;
        let out = extract(Lang::Python, source);
        assert_eq!(names(&out.definitions, DefKind::Class), vec!["Store"]);
        let fns = names(&out.definitions, DefKind::Function);
        assert!(fns.contains(&"add") && fns.contains(&"persist"));
        let add = out.definitions.iter().find(|d| d.name == "add").unwrap();
        assert_eq!(add.container.as_deref(), Some("Store"));
        let refs: Vec<&str> = out.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"persist") && refs.contains(&"validate"));
        assert!(out.imports.iter().any(|i| i == "os"));
        assert!(out.imports.iter().any(|i| i == "collections"));
    }

    #[test]
    fn extracts_typescript_shapes() {
        let source = r#"
import { helper } from "./helper";

export interface Shape { area(): number }

export type Alias = Shape;

export class Circle {
    area(): number { return compute(this.r); }
}

export function main() {
    const c = new Circle();
    return c.area();
}

export const arrow = (x: number) => helper(x);
"#;
        let out = extract(Lang::TypeScript, source);
        assert_eq!(names(&out.definitions, DefKind::Interface), vec!["Shape"]);
        assert_eq!(names(&out.definitions, DefKind::TypeAlias), vec!["Alias"]);
        assert_eq!(names(&out.definitions, DefKind::Class), vec!["Circle"]);
        let fns = names(&out.definitions, DefKind::Function);
        assert!(fns.contains(&"main") && fns.contains(&"arrow") && fns.contains(&"area"));
        let area = out
            .definitions
            .iter()
            .find(|d| d.name == "area" && d.kind == DefKind::Function)
            .unwrap();
        assert_eq!(area.container.as_deref(), Some("Circle"));
        let refs: Vec<&str> = out.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"compute") && refs.contains(&"Circle") && refs.contains(&"area"));
        assert_eq!(out.imports, vec!["./helper"]);
    }

    #[test]
    fn extracts_javascript_shapes() {
        let source = r#"
import { load } from "./load.js";

class Repo {
    fetch() { return load(this.url); }
}

const run = () => new Repo().fetch();
"#;
        let out = extract(Lang::JavaScript, source);
        assert_eq!(names(&out.definitions, DefKind::Class), vec!["Repo"]);
        let fns = names(&out.definitions, DefKind::Function);
        assert!(fns.contains(&"fetch") && fns.contains(&"run"));
        let refs: Vec<&str> = out.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"load") && refs.contains(&"Repo"));
        assert_eq!(out.imports, vec!["./load.js"]);
    }

    #[test]
    fn extracts_java_shapes() {
        let source = r#"
package com.example;

import java.util.List;
import org.apache.flink.api.common.functions.MapFunction;

public class Enricher implements MapFunction<String, String> {
    private static final int CAP = 10;

    public Enricher(List<String> names) {
        validate(names);
    }

    @Override
    public String map(String value) {
        return new Helper().apply(value);
    }
}

interface Helper { String apply(String value); }

enum Mode { FAST, SLOW }

record Pair(int left, int right) {}
"#;
        let out = extract(Lang::Java, source);
        assert_eq!(
            names(&out.definitions, DefKind::Class),
            vec!["Enricher", "Pair"]
        );
        assert_eq!(names(&out.definitions, DefKind::Interface), vec!["Helper"]);
        assert_eq!(names(&out.definitions, DefKind::Enum), vec!["Mode"]);
        assert_eq!(names(&out.definitions, DefKind::Field), vec!["CAP"]);
        let fns = names(&out.definitions, DefKind::Function);
        assert!(fns.contains(&"Enricher") && fns.contains(&"map") && fns.contains(&"apply"));
        let map = out
            .definitions
            .iter()
            .find(|d| d.name == "map" && d.kind == DefKind::Function)
            .unwrap();
        assert_eq!(map.container.as_deref(), Some("Enricher"));
        assert_eq!(map.line, 15);
        assert_eq!(map.end_line, 17, "method extent covers its body");
        let class = out
            .definitions
            .iter()
            .find(|d| d.name == "Enricher")
            .unwrap();
        assert_eq!((class.line, class.end_line), (7, 18));
        let refs: Vec<&str> = out.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"validate") && refs.contains(&"apply") && refs.contains(&"Helper"));
        assert!(out.imports.iter().any(|i| i == "java.util.List"));
    }

    #[test]
    fn broken_source_yields_partial_not_panic() {
        let out = extract(Lang::Rust, "fn good() {} fn broken(");
        assert!(names(&out.definitions, DefKind::Function).contains(&"good"));
    }
}
