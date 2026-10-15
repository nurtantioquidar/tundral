//! `extract --google-workspace`: Google Docs, Sheets and Slides shortcuts (`.gdoc`, `.gsheet`,
//! `.gslides`) exported through the `gws` CLI and sent like converted office files
//! (`docs/graphify-inputs.md` §4).
//!
//! A shortcut is a small JSON file naming the Drive file (`doc_id`, else the `/d/<id>/` of its
//! `url`). Docs export as Markdown, Slides as plain text and Sheets as `.xlsx` (then converted
//! like an office spreadsheet); the result is written to `dijkstral-out/converted/<stem>_<h8>.md`
//! behind a front-matter block naming its origin. `gws` is run without a shell, once per shortcut,
//! and only when the flag is given; the account e-mail a shortcut may carry is kept only as a
//! truncated hash.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::ingest::{sha256_hex, xlsx_markdown};

/// Runs an export: Drive file `file_id` as `mime` into `dest`.
pub trait Exporter: Sync + std::fmt::Debug {
    fn export(&self, file_id: &str, mime: &str, dest: &Path) -> Result<()>;
}

/// The `gws` command-line tool.
#[derive(Debug)]
pub struct GwsCli {
    bin: PathBuf,
}

impl GwsCli {
    /// `DIJKSTRAL_GWS`, else `gws` on `PATH`.
    pub fn find() -> Option<GwsCli> {
        if let Some(p) = std::env::var_os("DIJKSTRAL_GWS").filter(|p| !p.is_empty()) {
            return Some(GwsCli { bin: p.into() });
        }
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|d| d.join("gws"))
            .find(|p| p.is_file())
            .map(|bin| GwsCli { bin })
    }
}

impl Exporter for GwsCli {
    /// `gws drive files export --params {"fileId": …, "mimeType": …} -o <name>`, run in the
    /// destination's directory (rule G3).
    fn export(&self, file_id: &str, mime: &str, dest: &Path) -> Result<()> {
        let params = format!(
            "{{\"fileId\": {}, \"mimeType\": {}}}",
            Value::from(file_id),
            Value::from(mime)
        );
        let dir = dest
            .parent()
            .context("export destination has no directory")?;
        let name = dest.file_name().context("export destination has no name")?;
        let out = Command::new(&self.bin)
            .args(["drive", "files", "export", "--params", &params, "-o"])
            .arg(name)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("running {}", self.bin.display()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let first = err.lines().next().unwrap_or("").trim();
            bail!("gws exited with {}: {first}", out.status);
        }
        Ok(())
    }
}

/// The three shortcut kinds and how each is exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Doc,
    Sheet,
    Slides,
}

impl Kind {
    pub fn of(rel: &str) -> Option<Kind> {
        let lower = rel.to_ascii_lowercase();
        if lower.ends_with(".gdoc") {
            Some(Kind::Doc)
        } else if lower.ends_with(".gsheet") {
            Some(Kind::Sheet)
        } else if lower.ends_with(".gslides") {
            Some(Kind::Slides)
        } else {
            None
        }
    }

    /// Export MIME type and the extension of the exported file (rule G2).
    pub fn export_as(self) -> (&'static str, &'static str) {
        match self {
            Kind::Doc => ("text/markdown", "md"),
            Kind::Sheet => (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "xlsx",
            ),
            Kind::Slides => ("text/plain", "txt"),
        }
    }
}

/// What a shortcut file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub file_id: String,
    pub url: Option<String>,
    pub email: Option<String>,
}

/// Parse a shortcut; `None` when it is not JSON or names no Drive file (rule G1).
pub fn parse(bytes: &[u8]) -> Option<Shortcut> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let url = s("url").map(str::to_string);
    let file_id = s("doc_id")
        .map(str::to_string)
        .or_else(|| url.as_deref().and_then(id_from_url))?;
    // Ids are passed to gws as JSON; anything but Drive's alphabet is refused.
    if !file_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(Shortcut {
        file_id,
        url,
        email: s("email").map(str::to_string),
    })
}

fn id_from_url(url: &str) -> Option<String> {
    let rest = &url[url.find("/d/")? + 3..];
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!id.is_empty()).then_some(id)
}

/// Export the shortcut `rel` (under `root`) into `converted_dir` and return the converted
/// Markdown (rule G4). The caller writes it under its converted name.
pub fn convert(
    root: &Path,
    rel: &str,
    exporter: &dyn Exporter,
    converted_dir: &Path,
) -> Result<String> {
    let kind = Kind::of(rel).context("not a Google Workspace shortcut")?;
    let bytes = std::fs::read(root.join(rel)).with_context(|| format!("reading {rel}"))?;
    let shortcut = parse(&bytes).context("not a shortcut file (no doc_id or Drive URL)")?;
    let (mime, ext) = kind.export_as();
    std::fs::create_dir_all(converted_dir)?;
    let tmp = converted_dir.join(format!(".{}.export.{ext}", sha256_hex(rel.as_bytes())));
    let exported = exporter
        .export(&shortcut.file_id, mime, &tmp)
        .and_then(|()| std::fs::read(&tmp).context("gws wrote no file"));
    std::fs::remove_file(&tmp).ok();
    let exported = exported?;
    let body = match kind {
        Kind::Sheet => xlsx_markdown(&exported)?,
        Kind::Doc | Kind::Slides => String::from_utf8_lossy(&exported).into_owned(),
    };
    Ok(render(rel, &shortcut, mime, &body))
}

/// The converted file: front matter, a marker line, then the export.
pub fn render(rel: &str, s: &Shortcut, mime: &str, body: &str) -> String {
    let q = |v: &str| Value::from(v).to_string();
    let mut out = String::from("---\n");
    out.push_str(&format!("source_file: {}\n", q(rel)));
    out.push_str("source_type: \"google_workspace\"\n");
    out.push_str(&format!("google_file_id: {}\n", q(&s.file_id)));
    out.push_str(&format!("google_export_mime_type: {}\n", q(mime)));
    out.push_str(&format!(
        "source_url: {}\n",
        q(s.url.as_deref().unwrap_or(""))
    ));
    if let Some(email) = &s.email {
        out.push_str(&format!(
            "google_account_hash: {}\n",
            q(&sha256_hex(email.as_bytes())[..12])
        ));
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    out.push_str(&format!(
        "---\n\n<!-- converted from Google Workspace shortcut: {name} -->\n\n{body}"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_by_id_or_url() {
        let s = parse(
            br#"{"url": "https://docs.google.com/document/d/AbC-1_x/edit", "email": "a@b.c"}"#,
        )
        .unwrap();
        assert_eq!(s.file_id, "AbC-1_x");
        assert_eq!(s.email.as_deref(), Some("a@b.c"));
        assert_eq!(parse(br#"{"doc_id": "X1"}"#).unwrap().file_id, "X1");
        assert!(parse(br#"{"url": "https://example.com/not-google"}"#).is_none());
        assert!(parse(b"not json").is_none());
        assert!(parse(br#"{"doc_id": "a\"b"}"#).is_none());
        assert_eq!(Kind::of("x/Plan.GDOC"), Some(Kind::Doc));
        assert_eq!(Kind::of("a.txt"), None);
    }

    #[test]
    fn rendered_like_graphify() {
        let s = Shortcut {
            file_id: "DOCID123".into(),
            url: Some("https://docs.google.com/document/d/DOCID123/edit".into()),
            email: Some("someone@example.com".into()),
        };
        let md = render("plan.gdoc", &s, "text/markdown", "# Plan\n");
        assert_eq!(
            md,
            "---\nsource_file: \"plan.gdoc\"\nsource_type: \"google_workspace\"\n\
             google_file_id: \"DOCID123\"\ngoogle_export_mime_type: \"text/markdown\"\n\
             source_url: \"https://docs.google.com/document/d/DOCID123/edit\"\n\
             google_account_hash: \"72497f475e4f\"\n---\n\n\
             <!-- converted from Google Workspace shortcut: plan.gdoc -->\n\n# Plan\n"
        );
    }
}
