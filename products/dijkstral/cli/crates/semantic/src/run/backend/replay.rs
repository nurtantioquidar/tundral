//! Replays model replies from a transcript directory (`req-NNNN.json`, the format
//! `bench/llm_shim.py --transcript-dir` and `record::Recorder` write).
//!
//! A request is answered by the recorded reply whose fingerprint matches; the fingerprint is the
//! shim's (sha256 of Python's `json.dumps({"messages", "response_format"}, sort_keys=True)` over
//! the Chat Completions body), computed on the body the `openai` backend would send, so a
//! dijkstral run recorded through the shim replays here unchanged. A request with no match takes
//! the first unused successful record in order, like the shim. No model is called.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::types::{Backend, Request, Response};

use super::http::openai_messages;

pub struct Replay {
    records: Vec<Value>,
    used: Mutex<Vec<bool>>,
    model: String,
}

impl Replay {
    pub fn open(dir: &Path) -> Result<Replay> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("reading transcript directory {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("req-") && n.ends_with(".json"))
            })
            .collect();
        paths.sort();
        let mut records = Vec::new();
        for p in &paths {
            let v: Value = serde_json::from_slice(&std::fs::read(p)?)
                .with_context(|| format!("parsing {}", p.display()))?;
            records.push(v);
        }
        if records.is_empty() {
            bail!("no req-*.json records in {}", dir.display());
        }
        let model = records
            .iter()
            .find_map(|r| r["codex_model"].as_str())
            .unwrap_or("replay")
            .to_string();
        Ok(Replay {
            used: Mutex::new(vec![false; records.len()]),
            records,
            model,
        })
    }
}

impl Backend for Replay {
    fn name(&self) -> &str {
        "replay"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn complete(&self, req: &Request) -> Result<Response> {
        let fp = fingerprint(req);
        let ok = |r: &Value| r["status"] == 200 || r["status"].is_null();
        let mut used = self.used.lock().expect("replay lock");
        let pick = self
            .records
            .iter()
            .position(|r| ok(r) && r["fingerprint"] == fp.as_str())
            .or_else(|| {
                self.records
                    .iter()
                    .enumerate()
                    .position(|(i, r)| !used[i] && ok(r))
            });
        let Some(i) = pick else {
            bail!("replay: no recorded reply left");
        };
        used[i] = true;
        let resp = &self.records[i]["response"];
        let usage = &resp["usage"];
        Ok(Response {
            text: resp["text"].as_str().unwrap_or_default().to_string(),
            input_tokens: usage["input_tokens"].as_u64(),
            output_tokens: usage["output_tokens"].as_u64(),
            model: self.model.clone(),
            truncated: resp["finish_reason"] == "length",
        })
    }
}

/// The shim's request fingerprint for `req` sent as Chat Completions.
pub fn fingerprint(req: &Request) -> String {
    let key = json!({"messages": openai_messages(req), "response_format": null});
    let mut s = String::new();
    py_dumps_sorted(&key, &mut s);
    super::super::cache::sha256_hex(s.as_bytes())
}

/// `json.dumps(v, sort_keys=True)`: `", "` and `": "` separators, ASCII escapes.
fn py_dumps_sorted(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => dijkstral_core::pyjson::write_str(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                py_dumps_sorted(x, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dijkstral_core::pyjson::write_str(k, out);
                out.push_str(": ");
                py_dumps_sorted(&map[k], out);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_compatible_dump() {
        let mut s = String::new();
        py_dumps_sorted(
            &json!({"b": [1, null, true], "a": {"é": "x\n\u{1F600}"}}),
            &mut s,
        );
        assert_eq!(
            s,
            "{\"a\": {\"\\u00e9\": \"x\\n\\ud83d\\ude00\"}, \"b\": [1, null, true]}"
        );
    }

    #[test]
    fn replays_by_fingerprint_then_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let req = |t: &str| Request {
            system: "S".into(),
            user_text: t.into(),
            images: vec![],
            max_output_tokens: 1,
        };
        let rec = |n: usize, fp: &str, text: &str| {
            json!({"index": n, "fingerprint": fp, "status": 200, "codex_model": "gpt-x",
                   "response": {"text": text, "usage": {"input_tokens": n, "output_tokens": 1}}})
        };
        for (n, v) in [
            rec(1, "nope", "first"),
            rec(2, &fingerprint(&req("b")), "for b"),
        ]
        .iter()
        .enumerate()
        {
            std::fs::write(
                dir.path().join(format!("req-{:04}.json", n + 1)),
                v.to_string(),
            )
            .unwrap();
        }
        let r = Replay::open(dir.path()).unwrap();
        assert_eq!(r.model(), "gpt-x");
        // A fingerprint match is reusable; unmatched requests consume records in order.
        assert_eq!(r.complete(&req("b")).unwrap().text, "for b");
        assert_eq!(r.complete(&req("b")).unwrap().text, "for b");
        assert_eq!(r.complete(&req("zzz")).unwrap().text, "first");
        assert!(r.complete(&req("zzz")).is_err());
    }

    #[test]
    fn graphify_transcript_replays_in_order() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/semantic/mixed.transcript");
        let r = Replay::open(&dir).unwrap();
        let resp = r
            .complete(&Request {
                system: "x".into(),
                user_text: "y".into(),
                images: vec![],
                max_output_tokens: 1,
            })
            .unwrap();
        assert!(resp.text.starts_with("{\"nodes\":[{\"id\":\"readme\""));
        assert_eq!(resp.input_tokens, Some(16843));
    }
}
