//! Per-file extraction results. Everything here is derived from one file's bytes and path alone,
//! so it can be cached by content hash; cross-file meaning is added later by `resolve`.
//!
//! Field semantics follow `docs/graphify-format.md` (§6, §7).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Confidence {
    Extracted,
    Inferred,
    Ambiguous,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Extracted => "EXTRACTED",
            Confidence::Inferred => "INFERRED",
            Confidence::Ambiguous => "AMBIGUOUS",
        }
    }

    /// The score the clustered build fills in when an edge carries none (§2.5).
    pub fn default_score(self) -> f64 {
        match self {
            Confidence::Extracted => 1.0,
            Confidence::Inferred => 0.55,
            Confidence::Ambiguous => 0.2,
        }
    }
}

/// An extra node or edge field, kept typed so facts stay cheap to (de)serialize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Attr {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
    /// A pre-serialized JSON value (objects such as `metadata`), spliced verbatim.
    Json(String),
}

/// `source_location` has four observable shapes (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Loc {
    /// `"L<n>"`, 1-based.
    Line(u32),
    /// `""` (unresolved-reference stubs).
    Empty,
    /// `null` (0-byte files).
    Null,
    /// Key absent (externals, dynamic-import phantoms, `dynamic_import` edges).
    Absent,
}

/// What a node is, for the build stage. Not serialized into `graph.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    /// The node of a source file itself.
    File,
    /// A definition inside a file (function, class, method, const, package, JSON key…).
    Def,
    /// An unresolved-reference stub (`source_file: ""`, `file_type: "code"`); per file.
    Stub,
    /// A shared external import node (`type: "external"`); exempt from collision renaming.
    External,
    /// A rationale node (docstring or tagged comment).
    Rationale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    /// `file_type`: code, rationale, concept…
    pub file_type: String,
    /// `source_file`; `""` for stubs and externals.
    pub source_file: String,
    pub loc: Loc,
    /// Every other field (`_origin`, `_callable`, `metadata`, `type`…), written sorted.
    pub attrs: Vec<(String, Attr)>,
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&Attr> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: Confidence,
    /// `confidence_score` when the extractor set one (cross-file pass); filled at write time.
    pub score: Option<f64>,
    pub context: Option<String>,
    /// `source_file` of the source construct.
    pub source_file: String,
    pub loc: Loc,
    /// `weight`; `None` when the key is absent (`dynamic_import`).
    pub weight: Option<f64>,
    /// Every other field (`_origin`, `type_only`, `deferred`, `metadata`…), written sorted.
    pub attrs: Vec<(String, Attr)>,
}

/// Language-specific facts that only the cross-file pass consumes. Each language owns its type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum LangFacts {
    #[default]
    None,
    Python(crate::extract::python::PyFacts),
    Script(crate::extract::script::ScriptFacts),
    Rust(crate::extract::rust::RustFacts),
    Go(crate::extract::go::GoFacts),
    Java(crate::extract::java::JavaFacts),
    Data(crate::extract::data::DataFacts),
    Php(crate::extract::php::PhpFacts),
    Ruby(crate::extract::ruby::RubyFacts),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileFacts {
    /// Nodes in raw emission order: file node, definitions in pre-order, stubs at first use,
    /// rationale; externals may appear too (deduplicated globally, first wins).
    pub nodes: Vec<Node>,
    /// In-file edges in raw emission order (undeduplicated: the build keeps the last duplicate).
    pub edges: Vec<Edge>,
    pub lang: LangFacts,
    /// Whitespace-separated words in the file (Python `len(text.split())`), for the report's
    /// corpus check.
    pub words: u64,
}
