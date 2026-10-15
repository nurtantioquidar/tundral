//! The contract between the semantic pass's stages (`docs/graphify-semantic.md`):
//!
//! ```text
//! ingest  ──► Vec<SourceDoc> ──► chunk ──► Vec<Chunk>
//! run     ──► (Chunk, Backend) ──► Reply per chunk (cached by file)
//! merge   ──► (Graph, Vec<Reply>) ──► Graph with document/paper/image/concept nodes
//! ```
//!
//! Each stage owns its module; only these types are shared.

use serde::{Deserialize, Serialize};

/// How a file reaches the model (§2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocKind {
    /// Markdown, reStructuredText, text, HTML, YAML, and office files converted to Markdown.
    Document,
    /// SVG, sent as text.
    VectorImage,
    /// PNG/JPEG/GIF/WebP, sent as an image part.
    RasterImage,
    /// PDF, sent as its extracted text.
    Paper,
}

/// One input file, ready to be framed into a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDoc {
    /// Repo-relative path with `/` separators (for converted office files: the original path).
    pub rel: String,
    pub kind: DocKind,
    /// Text sent to the model (documents, SVG, extracted PDF text); empty for raster images.
    pub text: String,
    /// Raw bytes and MIME type for raster images.
    pub image: Option<(String, Vec<u8>)>,
    /// sha256 hex of the file's full content (front matter included), as framed in the request.
    pub sha256: String,
    /// Estimated cost in tokens (§4.2).
    pub tokens: usize,
}

/// A piece of a request: a whole document, or one oversize piece of it (§4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Index into the `SourceDoc` list.
    pub doc: usize,
    /// Character range of the document's text for an oversize piece; `None` when whole.
    pub piece: Option<(usize, usize)>,
}

/// One request's worth of input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub index: usize,
    pub parts: Vec<Part>,
}

/// What a backend is asked (built by `run` from a chunk; backends never see `SourceDoc`s).
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub system: String,
    pub user_text: String,
    /// `(mime, bytes, rel path)` per raster image, in order.
    pub images: Vec<(String, Vec<u8>, String)>,
    pub max_output_tokens: u32,
}

/// What a backend returns: the raw text (validated and repaired by `run`) and usage if known.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Response {
    pub text: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The backend's own name for the model that answered, for the cache key and reports.
    pub model: String,
}

/// A model backend (Codex CLI, OpenAI-compatible HTTP, Anthropic, Gemini, replay, …).
pub trait Backend: Send + Sync {
    /// Short stable name used in cache keys and logs (`codex`, `openai`, `replay`, …).
    fn name(&self) -> &str;
    /// The model identifier that will answer (part of the cache key).
    fn model(&self) -> &str;
    fn complete(&self, req: &Request) -> anyhow::Result<Response>;
}

/// The reply schema (§5). Unknown keys are ignored; required ones are checked by `run`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub nodes: Vec<ReplyNode>,
    #[serde(default)]
    pub edges: Vec<ReplyEdge>,
    #[serde(default)]
    pub hyperedges: Vec<ReplyHyperedge>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyNode {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub file_type: Option<String>,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub source_location: Option<serde_json::Value>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub captured_at: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub contributor: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyEdge {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
    #[serde(default)]
    pub confidence_score: Option<f64>,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub source_location: Option<serde_json::Value>,
    #[serde(default)]
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyHyperedge {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub confidence: Option<String>,
    #[serde(default)]
    pub confidence_score: Option<f64>,
    #[serde(default)]
    pub source_file: Option<String>,
}
