//! dijkstral's own prompts (written from the reply schema of §5, never from Graphify's text) and
//! the framing of a chunk's documents and images into a request.

use std::fmt::Write;

use crate::types::{Chunk, DocKind, Part, Request, SourceDoc};

use super::Mode;

/// Bumped whenever a prompt or the framing changes in a way that can change replies. Part of the
/// cache scope (§9), so a new version re-asks the model.
pub const PROMPT_VERSION: &str = "dijkstral-semantic-1";

/// The output limit of an extraction request (§4.3).
pub const MAX_OUTPUT_TOKENS: u32 = 16384;

const SYSTEM: &str = "\
You turn documents, papers and images into knowledge-graph fragments for dijkstral, a code \
knowledge graph that AI coding agents query. Code is indexed separately; your job is everything \
the code cannot say: what the documents describe, decide and connect.

Reply with exactly one JSON object and nothing else (no prose, no code fence):
{\"nodes\": [...], \"edges\": [...], \"hyperedges\": [...]}

Each node:
- \"id\": lowercase snake_case, unique in the reply. The node for a whole file uses the file's \
path without extension, every non-alphanumeric character replaced by \"_\" (docs/guide.md -> \
\"docs_guide\"). A thing inside a file uses that prefix plus its own snake_case name \
(\"docs_guide_retry_policy\").
- \"label\": a short human-readable name (under 80 characters).
- \"file_type\": one of \"document\", \"paper\", \"image\", \"concept\", \"rationale\", \"code\". \
Use the file's own kind for its file node, \"concept\" for ideas, components and terms, \
\"rationale\" for a decision and its reason, \"code\" only for a code symbol the text names.
- \"source_file\": the path exactly as written in the FILE header it came from.
- \"source_location\": the heading, section, sheet or page it came from, or null.
- \"source_url\", \"captured_at\", \"author\", \"contributor\": when the file states them \
(front matter, a byline), else null.
- \"rationale\": for a decision, why it was made and under which conditions to revisit it; \
else null.

Each edge:
- \"source\", \"target\": node ids from this reply. To point at code the documents name, use \
the code file's path without extension, non-alphanumerics as \"_\", then \"_\" and the symbol in \
lowercase (src/app/util.py, slugify -> \"src_app_util_slugify\").
- \"relation\": a snake_case verb phrase such as \"references\", \"describes\", \"cites\", \
\"implements\", \"configures\", \"calls\", \"depends_on\", \"conceptually_related_to\", \
\"shares_data_with\", \"rationale_for\".
- \"confidence\": \"EXTRACTED\" when the text states it, \"INFERRED\" for a reasonable \
inference, \"AMBIGUOUS\" when unsure and worth a human look.
- \"confidence_score\": a number from 0 to 1.
- \"source_file\", \"source_location\": where the evidence is.
- \"weight\": 1.0 unless the link is clearly weaker.

Each hyperedge groups three or more nodes that act together (a flow, a pipeline, a protocol):
{\"id\", \"label\", \"nodes\": [node ids], \"relation\", \"confidence\", \"confidence_score\", \
\"source_file\"}.

Rules:
- Give every file in the request a file node, and connect each other node to its file or to \
another node.
- Images: describe what the picture shows; boxes become nodes and arrows become edges.
- Only state what the files support. Do not invent files, people, links or numbers.
- Keep ids stable and predictable: the same thing gets the same id wherever it appears.
- Use empty lists when there is nothing to report.";

const DEEP: &str = "\n\nDeep mode: go finer. Also extract every defined term, parameter and its value, constraint, \
requirement, decision and cross-reference; add INFERRED edges between related concepts across \
the files of this request; give every decision a rationale node.";

/// The system prompt for `mode`.
pub fn system_prompt(mode: Mode) -> String {
    match mode {
        Mode::Normal => SYSTEM.to_string(),
        Mode::Deep => format!("{SYSTEM}{DEEP}"),
    }
}

/// Frames `parts` of `docs` (in order) into one extraction request.
pub fn extraction_request(docs: &[SourceDoc], parts: &[Part], mode: Mode) -> Request {
    let mut text = String::new();
    let mut files: Vec<&str> = Vec::new();
    for p in parts {
        let rel = docs[p.doc].rel.as_str();
        if !files.contains(&rel) {
            files.push(rel);
        }
    }
    let _ = writeln!(
        text,
        "Extract the knowledge graph of {} file(s). source_file must be one of: {}.",
        files.len(),
        files.join(", ")
    );
    let mut images = Vec::new();
    for p in parts {
        let d = &docs[p.doc];
        match d.kind {
            DocKind::RasterImage => {
                if let Some((mime, bytes)) = &d.image {
                    images.push((mime.clone(), bytes.clone(), d.rel.clone()));
                }
            }
            DocKind::Paper => {
                let _ = write!(
                    text,
                    "\n=== FILE: {} (paper, extracted text) ===\n{}\n=== END FILE: {} ===\n",
                    d.rel,
                    piece(&d.text, p.piece),
                    d.rel
                );
            }
            DocKind::Document | DocKind::VectorImage => {
                let what = if d.kind == DocKind::VectorImage {
                    "image, SVG source"
                } else {
                    "document"
                };
                match p.piece {
                    None => {
                        let _ = write!(
                            text,
                            "\n=== FILE: {} ({what}, sha256 {}) ===\n{}\n=== END FILE: {} ===\n",
                            d.rel, d.sha256, d.text, d.rel
                        );
                    }
                    Some((a, b)) => {
                        let total = d.text.chars().count();
                        let _ = write!(
                            text,
                            "\n=== FILE: {} ({what}, characters {a}-{b} of {total}) ===\n{}\n\
                             === END FILE: {} ===\n",
                            d.rel,
                            piece(&d.text, p.piece),
                            d.rel
                        );
                    }
                }
            }
        }
    }
    if !images.is_empty() {
        text.push_str("\nAttached images, in order (each is a file of this request):\n");
        for (i, (_, _, rel)) in images.iter().enumerate() {
            let _ = writeln!(text, "{}. {rel}", i + 1);
        }
    }
    Request {
        system: system_prompt(mode),
        user_text: text,
        images,
        max_output_tokens: MAX_OUTPUT_TOKENS,
    }
}

/// The request for `chunk` (all its parts).
pub fn chunk_request(docs: &[SourceDoc], chunk: &Chunk, mode: Mode) -> Request {
    extraction_request(docs, &chunk.parts, mode)
}

/// The characters `range` of `text` (a character range, §4.2), or all of it.
fn piece(text: &str, range: Option<(usize, usize)>) -> &str {
    let Some((a, b)) = range else {
        return text;
    };
    let start = text.char_indices().nth(a).map_or(text.len(), |(i, _)| i);
    let end = text.char_indices().nth(b).map_or(text.len(), |(i, _)| i);
    &text[start..end.max(start)]
}

/// Label prompt: one user message, no system message (§10).
pub fn label_request(batch: &[(String, Vec<String>)]) -> Request {
    let mut text = String::from(
        "Name each community of a code knowledge graph. A community is a group of closely \
         connected code symbols, documents and concepts; its members are listed after its id. \
         Give each a specific name of two to five words in Title Case that says what the group \
         is about (not \"Misc\" or \"Utilities\" unless nothing better fits).\n\
         Reply with exactly one JSON object mapping every community id (as a string) to its \
         name, for example {\"0\": \"Job Queue Caching\"}, and nothing else.\n\n",
    );
    for (cid, labels) in batch {
        let _ = writeln!(text, "{cid}: {}", labels.join(", "));
    }
    Request {
        system: String::new(),
        user_text: text,
        images: Vec::new(),
        max_output_tokens: 256 + 48 * batch.len() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(rel: &str, kind: DocKind, text: &str) -> SourceDoc {
        SourceDoc {
            rel: rel.into(),
            kind,
            text: text.into(),
            image: None,
            sha256: "ab".repeat(32),
            tokens: 10,
        }
    }

    #[test]
    fn system_prompt_names_every_schema_key() {
        let p = system_prompt(Mode::Normal);
        for key in [
            "nodes",
            "edges",
            "hyperedges",
            "id",
            "label",
            "file_type",
            "source_file",
            "source_location",
            "source_url",
            "captured_at",
            "author",
            "contributor",
            "rationale",
            "source",
            "target",
            "relation",
            "confidence",
            "confidence_score",
            "weight",
        ] {
            assert!(p.contains(&format!("\"{key}\"")), "{key}");
        }
        assert!(system_prompt(Mode::Deep).starts_with(&p));
        assert!(system_prompt(Mode::Deep).len() > p.len());
    }

    #[test]
    fn frames_documents_pieces_and_images() {
        let mut img = doc("images/a.png", DocKind::RasterImage, "");
        img.image = Some(("image/png".into(), vec![1, 2, 3]));
        let docs = vec![
            doc("README.md", DocKind::Document, "# Hi\nbody"),
            img,
            doc("p.pdf", DocKind::Paper, "abcdefghij"),
        ];
        let parts = vec![
            Part {
                doc: 0,
                piece: None,
            },
            Part {
                doc: 1,
                piece: None,
            },
            Part {
                doc: 2,
                piece: Some((2, 5)),
            },
        ];
        let r = extraction_request(&docs, &parts, Mode::Normal);
        assert!(r.user_text.starts_with(
            "Extract the knowledge graph of 3 file(s). source_file must be one of: README.md, images/a.png, p.pdf."
        ));
        assert!(r
            .user_text
            .contains("=== FILE: README.md (document, sha256 abab"));
        assert!(r
            .user_text
            .contains("# Hi\nbody\n=== END FILE: README.md ==="));
        assert!(r.user_text.contains("\ncde\n"));
        assert!(r.user_text.ends_with("1. images/a.png\n"));
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].2, "images/a.png");
    }

    #[test]
    fn label_request_budget() {
        let b = vec![("0".to_string(), vec!["a".to_string()]); 5];
        assert_eq!(label_request(&b).max_output_tokens, 496);
        assert!(label_request(&b).system.is_empty());
    }
}
