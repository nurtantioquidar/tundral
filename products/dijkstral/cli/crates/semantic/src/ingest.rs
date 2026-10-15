//! Which files go to the model, office and PDF text, image bytes, token estimates and chunking
//! (`docs/graphify-semantic.md` §2, §4.1–4.2).
//!
//! Inputs are the walked non-code files of core discovery (`Discovery::documents`), so the ignore
//! rules, skipped directories and the sensitive-file filter are core's, applied once.
//!
//! PDF text comes from `pdf-extract` (pure Rust), where Graphify uses pypdf. On the fixture paper
//! (one page, one `Tj` per line) both give the same lines, but `pdf-extract` lays text out by
//! position: the page starts with two newlines and lines whose baselines are far apart are
//! separated by a blank line. [`pdf_text`] drops blank lines and trailing whitespace, so the
//! fixture's text equals pypdf's byte for byte and costs the same 177 tokens. On other PDFs the two
//! libraries can differ in word spacing (each infers spaces from glyph gaps its own way), in
//! hyphenation, in the order of multi-column text and in blank lines pypdf keeps; such differences
//! move the estimate by a few tokens per page and the chunk boundaries only when a chunk is within
//! that margin of the budget. A PDF whose text cannot be extracted is still sent, with empty text,
//! as Graphify does without pypdf.

use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};

use crate::types::{Chunk, DocKind, Part, SourceDoc};

/// `--token-budget` default (§4.2).
pub const DEFAULT_TOKEN_BUDGET: usize = 60_000;
/// Added to every text file's token count (§4.2).
pub const TEXT_OVERHEAD_TOKENS: usize = 40;
/// The estimate of every raster image, whatever its size (§4.2).
pub const IMAGE_TOKENS: usize = 1600;
/// Oversize files are sent in pieces of this many characters (§4.2).
pub const PIECE_CHARS: usize = 20_000;

/// How a walked file takes part in the semantic pass (§2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// `.md .mdx .rst .txt .html .yaml .yml`, sent as text.
    Document,
    /// `.docx .xlsx`, converted to Markdown first (§2.2).
    Office,
    /// `.pdf`, sent as its extracted text.
    Paper,
    /// `.png .jpg .jpeg .gif .webp`, sent as an image part.
    Raster,
    /// `.svg`, sent as its XML text.
    Vector,
}

/// The class of a relative path, or `None` for files the pass never sends: audio and video
/// (silently, without the `video` extra), Google Workspace shortcuts, `.markdown`, `.htm`, `.csv`,
/// `.ipynb` and everything not listed in §2.1.
pub fn classify(rel: &str) -> Option<Class> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let ext = match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => return None,
    };
    Some(match ext.as_str() {
        "md" | "mdx" | "rst" | "txt" | "html" | "yaml" | "yml" => Class::Document,
        "docx" | "xlsx" => Class::Office,
        "pdf" => Class::Paper,
        "png" | "jpg" | "jpeg" | "gif" | "webp" => Class::Raster,
        "svg" => Class::Vector,
        _ => return None,
    })
}

/// Whether a text document reads as a paper (probed on `update`, which gives such Markdown no
/// page): at least three of these signals, each counted once, case-insensitive: `arxiv`, an arXiv
/// id (`1706.03762`), `doi`, `abstract`, a numbered citation `[1]` (not an index such as
/// `argv[0]`: Graphify's own `CHANGELOG.md` and skill files, with `arxiv`, `abstract` and
/// `scored[0]`, stay pages), `\cite{`, `journal`, `preprint`, `proceedings`. `et al.`,
/// `References`, `ISBN`, `Theorem`, `Figure 1`, `vol.` and a second DOI do not count. Exact word
/// boundaries were not probed.
pub fn looks_like_paper(text: &str) -> bool {
    static SIGNALS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let signals = SIGNALS.get_or_init(|| {
        [
            r"(?i)arxiv",
            r"\b\d{4}\.\d{4,5}\b",
            r"(?i)\bdoi\b",
            r"(?i)\babstract\b",
            r"(?:^|[^\w\]])\[\d+\]",
            r"\\cite\{",
            r"(?i)\bjournal\b",
            r"(?i)\bpreprint\b",
            r"(?i)\bproceedings\b",
        ]
        .iter()
        .map(|p| regex::Regex::new(p).expect("valid pattern"))
        .collect()
    });
    signals.iter().filter(|r| r.is_match(text)).count() >= 3
}

/// The MIME type of a raster image, from its extension only (§2.1: `.jpg` is `image/jpeg` even
/// for PNG bytes).
pub fn image_mime(rel: &str) -> &'static str {
    let lower = rel.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or("") {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

#[derive(Debug, Clone)]
pub struct IngestOptions<'a> {
    /// Relative path the converted office files are dispatched, cached and expected back under
    /// (Graphify: `graphify-out/converted`).
    pub converted_prefix: &'a str,
    /// Directory the converted files are also written to; `None` keeps them in memory.
    pub converted_dir: Option<&'a Path>,
    /// Exports Google Workspace shortcuts (`--google-workspace`); `None` leaves them out.
    pub workspace: Option<&'a dyn crate::gws::Exporter>,
}

/// What [`ingest`] produced.
#[derive(Debug, Default)]
pub struct Ingested {
    /// Inputs in dispatch order (§4.1).
    pub docs: Vec<SourceDoc>,
    /// Files left out because they could not be read or converted: `(rel, reason)`.
    pub skipped: Vec<(String, String)>,
    /// Non-fatal problems (a PDF without extractable text): `(rel, message)`.
    pub warnings: Vec<(String, String)>,
}

/// Load the semantic-pass inputs among `documents` (core discovery's walked non-code files,
/// relative to `root`) in dispatch order (§4.1): documents and SVGs sorted by path, then converted
/// office files (sorted by their converted path), then raster images, then papers by path
/// (PDFs, and text documents that [`looks_like_paper`], sent as their text).
pub fn ingest(root: &Path, documents: &[String], opts: &IngestOptions) -> Ingested {
    let mut out = Ingested::default();
    let mut text = Vec::new();
    let mut office = Vec::new();
    let mut raster = Vec::new();
    let mut papers = Vec::new();
    let mut sorted: Vec<&String> = documents.iter().collect();
    sorted.sort();
    for rel in sorted {
        if let (Some(exporter), Some(_)) = (opts.workspace, crate::gws::Kind::of(rel)) {
            // Exported, then sent with the converted office files (`graphify-inputs.md` §4).
            let dir = opts
                .converted_dir
                .map_or_else(std::env::temp_dir, Path::to_path_buf);
            match crate::gws::convert(root, rel, exporter, &dir) {
                Ok(md) => {
                    let path = format!("{}/{}", opts.converted_prefix, converted_name(rel));
                    if let Some(dir) = opts.converted_dir {
                        if let Err(e) = std::fs::write(dir.join(converted_name(rel)), &md) {
                            out.warnings
                                .push((rel.clone(), format!("writing {path}: {e}")));
                        }
                    }
                    let mut doc = text_doc(rel, DocKind::Document, md.as_bytes());
                    doc.converted = Some(path);
                    office.push(doc);
                }
                Err(e) => out.skipped.push((rel.clone(), format!("{e:#}"))),
            }
            continue;
        }
        let Some(class) = classify(rel) else {
            continue;
        };
        let bytes = match std::fs::read(root.join(rel)) {
            Ok(b) => b,
            Err(e) => {
                out.skipped.push((rel.clone(), e.to_string()));
                continue;
            }
        };
        match class {
            Class::Vector => text.push(text_doc(rel, DocKind::VectorImage, &bytes)),
            Class::Document => {
                let doc = text_doc(rel, DocKind::Document, &bytes);
                if looks_like_paper(&doc.text) {
                    papers.push(SourceDoc {
                        kind: DocKind::Paper,
                        ..doc
                    });
                } else {
                    text.push(doc);
                }
            }
            Class::Office => match convert_office(rel, &bytes) {
                Ok(md) => {
                    let path = format!("{}/{}", opts.converted_prefix, converted_name(rel));
                    if let Some(dir) = opts.converted_dir {
                        let written = std::fs::create_dir_all(dir)
                            .and_then(|()| std::fs::write(dir.join(converted_name(rel)), &md));
                        if let Err(e) = written {
                            out.warnings
                                .push((rel.clone(), format!("writing {path}: {e}")));
                        }
                    }
                    let mut doc = text_doc(rel, DocKind::Document, md.as_bytes());
                    doc.converted = Some(path);
                    office.push(doc);
                }
                Err(e) => out.skipped.push((rel.clone(), format!("{e:#}"))),
            },
            Class::Raster => raster.push(SourceDoc {
                rel: rel.clone(),
                kind: DocKind::RasterImage,
                text: String::new(),
                image: Some((image_mime(rel).to_string(), bytes.clone())),
                sha256: sha256_hex(&bytes),
                tokens: IMAGE_TOKENS,
                converted: None,
            }),
            Class::Paper => {
                let text = match pdf_text(&bytes) {
                    Ok(t) => t,
                    Err(e) => {
                        out.warnings
                            .push((rel.clone(), format!("PDF text extraction failed: {e:#}")));
                        String::new()
                    }
                };
                papers.push(SourceDoc {
                    rel: rel.clone(),
                    kind: DocKind::Paper,
                    tokens: count_tokens(&text) + TEXT_OVERHEAD_TOKENS,
                    text,
                    image: None,
                    sha256: sha256_hex(&bytes),
                    converted: None,
                });
            }
        }
    }
    office.sort_by(|a, b| a.converted.cmp(&b.converted));
    out.docs = text
        .into_iter()
        .chain(office)
        .chain(raster)
        .chain(papers)
        .collect();
    out
}

fn text_doc(rel: &str, kind: DocKind, bytes: &[u8]) -> SourceDoc {
    let text = String::from_utf8_lossy(bytes).into_owned();
    SourceDoc {
        rel: rel.to_string(),
        kind,
        tokens: count_tokens(&text) + TEXT_OVERHEAD_TOKENS,
        text,
        image: None,
        sha256: sha256_hex(bytes),
        converted: None,
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// `len(cl100k_base.encode(text))`, special-token text counted as ordinary text.
pub fn count_tokens(text: &str) -> usize {
    static BPE: OnceLock<&'static tiktoken_rs::CoreBPE> = OnceLock::new();
    BPE.get_or_init(tiktoken_rs::cl100k_base_singleton)
        .encode_ordinary(text)
        .len()
}

/// The semantic cache key of a dispatched file (§9.1): `sha256(body + b"\0" + lower(path))`, where
/// `body` is the file's bytes, or for markdown with front matter the bytes after the closing
/// `---` (from the newline that follows it).
pub fn cache_key(dispatched: &str, bytes: &[u8]) -> String {
    let body = if crate::markdown::is_markdown(dispatched) {
        after_front_matter(bytes)
    } else {
        bytes
    };
    let mut h = Sha256::new();
    h.update(body);
    h.update([0]);
    h.update(dispatched.to_lowercase().as_bytes());
    format!("{:x}", h.finalize())
}

fn after_front_matter(bytes: &[u8]) -> &[u8] {
    let Some(rest) = bytes.strip_prefix(b"---\n") else {
        return bytes;
    };
    let mut at = 4;
    for line in rest.split_inclusive(|&b| b == b'\n') {
        let bare = line.strip_suffix(b"\n").unwrap_or(line);
        let bare = bare.strip_suffix(b"\r").unwrap_or(bare);
        if bare == b"---" {
            return &bytes[at + 3..];
        }
        at += line.len();
    }
    bytes
}

/// `<stem>_<first 8 hex of sha256(relative path)>.md` (§2.2).
pub fn converted_name(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let stem = name
        .rfind('.')
        .filter(|&i| i > 0)
        .map_or(name, |i| &name[..i]);
    format!("{stem}_{}.md", &sha256_hex(rel.as_bytes())[..8])
}

/// An office file as Graphify's converted Markdown (§2.2).
pub fn convert_office(rel: &str, bytes: &[u8]) -> Result<String> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let body = match classify(rel) {
        Some(Class::Office) if name.to_ascii_lowercase().ends_with(".docx") => docx_text(bytes)?,
        Some(Class::Office) => xlsx_markdown(bytes)?,
        _ => bail!("not an office file: {rel}"),
    };
    Ok(format!("<!-- converted from {name} -->\n\n{body}"))
}

fn zip_part(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, path: &str) -> Result<String> {
    let mut file = archive
        .by_name(path)
        .with_context(|| format!("missing part {path}"))?;
    let mut s = String::new();
    file.read_to_string(&mut s)
        .with_context(|| format!("reading part {path}"))?;
    Ok(s)
}

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<std::io::Cursor<&[u8]>>> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("not a zip archive")
}

/// The text a general reference (`&amp;`, `&#x27;`) stands for.
fn reference(r: &quick_xml::events::BytesRef) -> Result<String> {
    if let Some(c) = r.resolve_char_ref()? {
        return Ok(c.to_string());
    }
    Ok(match r.decode()?.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        other => return Ok(format!("&{other};")),
    }
    .to_string())
}

/// A `.docx` body: one line per top-level paragraph with text (heading styles are not marked);
/// tables are not read.
fn docx_text(bytes: &[u8]) -> Result<String> {
    let mut archive = open_zip(bytes)?;
    let xml = zip_part(&mut archive, "word/document.xml")?;
    let mut reader = Reader::from_str(&xml);
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    // The depth of the enclosing paragraph that sits directly in the body (not in a table cell).
    let body_p = |path: &[Vec<u8>]| {
        path.windows(2)
            .position(|w| w[0] == b"body" && w[1] == b"p")
            .map(|i| i + 1)
    };
    let in_text =
        |path: &[Vec<u8>]| path.last().is_some_and(|p| p == b"t") && body_p(path).is_some();
    loop {
        match reader.read_event()? {
            Event::Start(e) => path.push(e.local_name().as_ref().to_vec()),
            Event::End(e) => {
                if e.local_name().as_ref() == b"p" && body_p(&path) == Some(path.len() - 1) {
                    let p = std::mem::take(&mut current);
                    if !p.trim().is_empty() {
                        paragraphs.push(p);
                    }
                }
                path.pop();
            }
            Event::Empty(e) => {
                if body_p(&path).is_some() {
                    match e.local_name().as_ref() {
                        b"tab" => current.push('\t'),
                        b"br" | b"cr" => current.push('\n'),
                        _ => {}
                    }
                }
            }
            Event::Text(t) if in_text(&path) => current.push_str(&t.xml_content()?),
            Event::GeneralRef(r) if in_text(&path) => current.push_str(&reference(&r)?),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(paragraphs.join("\n"))
}

/// A `.xlsx` workbook: `## Sheet: <name>` and a Markdown table per sheet, first row as header.
pub(crate) fn xlsx_markdown(bytes: &[u8]) -> Result<String> {
    let mut archive = open_zip(bytes)?;
    let workbook = zip_part(&mut archive, "xl/workbook.xml")?;
    let rels = zip_part(&mut archive, "xl/_rels/workbook.xml.rels").unwrap_or_default();
    let shared = match zip_part(&mut archive, "xl/sharedStrings.xml") {
        Ok(xml) => shared_strings(&xml)?,
        Err(_) => Vec::new(),
    };
    let targets: Vec<(String, String)> = elements(&rels, b"Relationship")?
        .into_iter()
        .filter_map(|a| Some((attr(&a, "Id")?, attr(&a, "Target")?)))
        .collect();
    let mut sections = Vec::new();
    for sheet in elements(&workbook, b"sheet")? {
        let name = attr(&sheet, "name").unwrap_or_default();
        let id = attr(&sheet, "id").ok_or_else(|| anyhow!("sheet {name} has no r:id"))?;
        let target = targets
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, t)| t.as_str())
            .ok_or_else(|| anyhow!("sheet {name}: no relationship {id}"))?;
        let part = match target.strip_prefix('/') {
            Some(abs) => abs.to_string(),
            None => format!("xl/{target}"),
        };
        let rows = sheet_rows(&zip_part(&mut archive, &part)?, &shared)?;
        let mut lines = vec![format!("## Sheet: {name}")];
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        for (i, row) in rows.iter().enumerate() {
            let mut cells = row.clone();
            cells.resize(width, String::new());
            lines.push(format!("| {} |", cells.join(" | ")));
            if i == 0 {
                lines.push(format!("|{}|", vec![" --- "; width].join("|")));
            }
        }
        sections.push(lines.join("\n"));
    }
    Ok(sections.join("\n\n"))
}

/// The attributes of every `local`-named element, by local attribute name.
fn elements(xml: &str, local: &[u8]) -> Result<Vec<Vec<(String, String)>>> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == local => {
                let mut attrs = Vec::new();
                for a in e.attributes() {
                    let a = a?;
                    let key = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
                    attrs.push((key, a.unescape_value()?.into_owned()));
                }
                out.push(attrs);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn attr(attrs: &[(String, String)], key: &str) -> Option<String> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn shared_strings(xml: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut current = String::new();
    // Phonetic runs (`rPh`) are not part of the value.
    let in_text = |path: &[Vec<u8>]| {
        path.last().is_some_and(|p| p == b"t") && !path.iter().any(|p| p == b"rPh")
    };
    loop {
        match reader.read_event()? {
            Event::Start(e) => path.push(e.local_name().as_ref().to_vec()),
            Event::End(e) => {
                if e.local_name().as_ref() == b"si" {
                    out.push(std::mem::take(&mut current));
                }
                path.pop();
            }
            Event::Empty(e) if e.local_name().as_ref() == b"si" => out.push(String::new()),
            Event::Text(t) if in_text(&path) => current.push_str(&t.xml_content()?),
            Event::GeneralRef(r) if in_text(&path) => current.push_str(&reference(&r)?),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// A worksheet's rows as cell strings, columns placed by their reference (`C3`), missing cells
/// empty, rows from the first to the last one present.
fn sheet_rows(xml: &str, shared: &[String]) -> Result<Vec<Vec<String>>> {
    let mut reader = Reader::from_str(xml);
    let mut rows: Vec<(usize, Vec<String>)> = Vec::new();
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut cell: Option<(usize, String)> = None;
    let mut value = String::new();
    let mut next_col = 0;
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                match e.local_name().as_ref() {
                    b"row" => {
                        let n = attr_of(&e, b"r")?.and_then(|r| r.parse::<usize>().ok());
                        let n = n.unwrap_or_else(|| rows.last().map_or(1, |(r, _)| r + 1));
                        rows.push((n, Vec::new()));
                        next_col = 0;
                    }
                    b"c" => {
                        let col = attr_of(&e, b"r")?
                            .and_then(|r| column(&r))
                            .unwrap_or(next_col);
                        let t = attr_of(&e, b"t")?.unwrap_or_default();
                        cell = Some((col, t));
                        value.clear();
                    }
                    _ => {}
                }
                path.push(e.local_name().as_ref().to_vec());
            }
            Event::End(e) => {
                path.pop();
                if e.local_name().as_ref() == b"c" {
                    if let (Some((col, t)), Some((_, row))) = (cell.take(), rows.last_mut()) {
                        let text = cell_text(&t, &value, shared);
                        if row.len() <= col {
                            row.resize(col + 1, String::new());
                        }
                        row[col] = text;
                        next_col = col + 1;
                    }
                }
            }
            Event::Empty(e) if e.local_name().as_ref() == b"c" => {
                next_col = attr_of(&e, b"r")?
                    .and_then(|r| column(&r))
                    .map_or(next_col + 1, |c| c + 1);
            }
            Event::Text(t)
                if cell.is_some()
                    && matches!(path.last().map(Vec::as_slice), Some(b"v" | b"t")) =>
            {
                value.push_str(&t.xml_content()?)
            }
            Event::GeneralRef(r) if cell.is_some() => value.push_str(&reference(&r)?),
            Event::Eof => break,
            _ => {}
        }
    }
    rows.retain(|(_, cells)| !cells.is_empty());
    let Some(first) = rows.first().map(|(r, _)| *r) else {
        return Ok(Vec::new());
    };
    let last = rows.last().map_or(first, |(r, _)| *r);
    let mut out = vec![Vec::new(); last.saturating_sub(first) + 1];
    for (r, cells) in rows {
        if r >= first && r - first < out.len() {
            out[r - first] = cells;
        }
    }
    Ok(out)
}

fn attr_of(e: &quick_xml::events::BytesStart, key: &[u8]) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a?;
        if a.key.local_name().as_ref() == key {
            return Ok(Some(a.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

/// `C3` → 2.
fn column(reference: &str) -> Option<usize> {
    let letters: Vec<u8> = reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .collect();
    if letters.is_empty() {
        return None;
    }
    let n = letters.iter().fold(0usize, |n, c| {
        n * 26 + usize::from(c.to_ascii_uppercase() - b'A' + 1)
    });
    Some(n - 1)
}

/// A cell as Python prints openpyxl's value: shared and inline strings verbatim, booleans as
/// `True`/`False`, integers without and other numbers with a decimal point (`900`, `1.5`).
/// Date-formatted numbers are printed as numbers (openpyxl would print a `datetime`).
fn cell_text(t: &str, v: &str, shared: &[String]) -> String {
    match t {
        "s" => v
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| shared.get(i).cloned())
            .unwrap_or_default(),
        "b" => if v.trim() == "1" { "True" } else { "False" }.to_string(),
        "inlineStr" | "str" | "e" => v.to_string(),
        _ => {
            let v = v.trim();
            if v.contains(['.', 'e', 'E']) {
                match v.parse::<f64>() {
                    Ok(f) => python_float(f),
                    Err(_) => v.to_string(),
                }
            } else {
                match v.parse::<i64>() {
                    Ok(i) => i.to_string(),
                    Err(_) => v.to_string(),
                }
            }
        }
    }
}

/// Python's `repr(float)` for the common cases (`900.0`, `1.5`, `1e-05`, `1e+16`).
fn python_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 {
        return format!("{f:.1}");
    }
    let s = format!("{f:?}");
    match s.split_once('e') {
        Some((m, e)) => {
            let (sign, digits) = e.strip_prefix('-').map_or(("+", e), |d| ("-", d));
            format!("{m}e{sign}{digits:0>2}")
        }
        None => s,
    }
}

/// A PDF's text with `pdf-extract`, its layout whitespace removed so lines match pypdf's (see
/// the module notes): pages joined by a newline, blank lines and trailing spaces dropped.
pub fn pdf_text(bytes: &[u8]) -> Result<String> {
    let pages = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes))
        .map_err(|_| anyhow!("the PDF parser panicked"))?
        .map_err(|e| anyhow!("{e}"))?;
    let pages: Vec<String> = pages
        .iter()
        .map(|p| {
            p.lines()
                .map(str::trim_end)
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    Ok(pages.join("\n"))
}

/// Greedy chunking in dispatch order (§4.2): a file joins the current chunk while the running
/// estimate stays within `budget`; a file whose own estimate exceeds it is sent alone, a text one
/// in [`PIECE_CHARS`]-character pieces, one chunk each.
pub fn chunk(docs: &[SourceDoc], budget: usize) -> Vec<Chunk> {
    let mut chunks: Vec<Vec<Part>> = Vec::new();
    let mut current: Vec<Part> = Vec::new();
    let mut sum = 0;
    for (i, doc) in docs.iter().enumerate() {
        if doc.tokens > budget {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                sum = 0;
            }
            if doc.kind == DocKind::RasterImage {
                chunks.push(vec![Part {
                    doc: i,
                    piece: None,
                }]);
            } else {
                for piece in pieces(doc.text.chars().count()) {
                    chunks.push(vec![Part {
                        doc: i,
                        piece: Some(piece),
                    }]);
                }
            }
            continue;
        }
        if !current.is_empty() && sum + doc.tokens > budget {
            chunks.push(std::mem::take(&mut current));
            sum = 0;
        }
        current.push(Part {
            doc: i,
            piece: None,
        });
        sum += doc.tokens;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, parts)| Chunk { index, parts })
        .collect()
}

/// `[0, 20000), [20000, 25000)` for 25,000 characters; one empty piece for an empty text.
fn pieces(chars: usize) -> Vec<(usize, usize)> {
    if chars == 0 {
        return vec![(0, 0)];
    }
    (0..chars)
        .step_by(PIECE_CHARS)
        .map(|start| (start, (start + PIECE_CHARS).min(chars)))
        .collect()
}

/// The text of a part: the whole document, or its character range.
pub fn part_text<'a>(doc: &'a SourceDoc, part: &Part) -> &'a str {
    match part.piece {
        None => &doc.text,
        Some((start, end)) => {
            let at = |n: usize| {
                doc.text
                    .char_indices()
                    .nth(n)
                    .map_or(doc.text.len(), |(i, _)| i)
            };
            &doc.text[at(start)..at(end)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_follows_the_spec() {
        for (rel, want) in [
            ("README.md", Some(Class::Document)),
            ("UPPER.MD", Some(Class::Document)),
            ("CMakeLists.txt", Some(Class::Document)),
            ("a/b.YML", Some(Class::Document)),
            ("x.markdown", None),
            ("x.htm", None),
            ("x.csv", None),
            ("x.ipynb", None),
            ("o/c.xlsx", Some(Class::Office)),
            ("p.pdf", Some(Class::Paper)),
            ("i.jpeg", Some(Class::Raster)),
            ("i.svg", Some(Class::Vector)),
            ("a.mp3", None),
            ("v.mov", None),
            ("d.gdoc", None),
            ("s.gsheet", None),
        ] {
            assert_eq!(classify(rel), want, "{rel}");
        }
        assert_eq!(image_mime("a.JPG"), "image/jpeg");
    }

    #[test]
    fn token_estimates_use_cl100k() {
        // `'word '*200` costs 241 (§4.2); a Japanese text pins cl100k over o200k.
        assert_eq!(
            count_tokens(&"word ".repeat(200)) + TEXT_OVERHEAD_TOKENS,
            241
        );
        assert_eq!(count_tokens("<|endoftext|>"), 7);
    }

    #[test]
    fn oversize_text_is_split_by_characters() {
        let doc = |tokens: usize, text: String| SourceDoc {
            rel: "a.md".into(),
            kind: DocKind::Document,
            text,
            image: None,
            sha256: String::new(),
            tokens,
            converted: None,
        };
        let docs = vec![
            doc(10, "a".into()),
            doc(500, "é".repeat(25_000)),
            doc(10, "b".into()),
        ];
        let chunks = chunk(&docs, 100);
        let shape: Vec<Vec<_>> = chunks
            .iter()
            .map(|c| c.parts.iter().map(|p| (p.doc, p.piece)).collect())
            .collect();
        assert_eq!(
            shape,
            [
                vec![(0, None)],
                vec![(1, Some((0, 20_000)))],
                vec![(1, Some((20_000, 25_000)))],
                vec![(2, None)],
            ]
        );
        assert_eq!(
            part_text(&docs[1], &chunks[2].parts[0]).chars().count(),
            5_000
        );
    }

    #[test]
    fn paper_signals_need_three() {
        let doc = |extra: &str| format!("# T\n\n## Abstract\n\ntext\ndoi: 10.1000/xyz\n{extra}\n");
        for extra in [
            "1706.03762",
            "arxiv",
            "[1] A. B. 2019.",
            "\\cite{x}",
            "Journal of X",
        ] {
            assert!(looks_like_paper(&doc(extra)), "{extra}");
        }
        for extra in [
            "",
            "Smith et al. (2019)",
            "## References",
            "doi: 10.1/2",
            "ISBN 1",
        ] {
            assert!(!looks_like_paper(&doc(extra)), "{extra}");
        }
        assert!(looks_like_paper(
            "Vaswani et al.\narXiv: 1706.03762\n## Abstract"
        ));
        assert!(!looks_like_paper("Vaswani et al.\narXiv: 1706.03762"));
    }

    #[test]
    fn python_float_repr() {
        assert_eq!(python_float(900.0), "900.0");
        assert_eq!(python_float(1.5), "1.5");
        assert_eq!(python_float(1e-5), "1e-05");
        assert_eq!(python_float(1e16), "1e+16");
        assert_eq!(cell_text("", "120", &[]), "120");
        assert_eq!(cell_text("b", "1", &[]), "True");
    }

    #[test]
    fn front_matter_is_left_out_of_cache_bodies() {
        assert_eq!(after_front_matter(b"---\na: 1\n---\nbody"), b"\nbody");
        assert_eq!(after_front_matter(b"---\na: 1\n"), b"---\na: 1\n");
        assert_eq!(after_front_matter(b"# x\n"), b"# x\n");
    }
}
