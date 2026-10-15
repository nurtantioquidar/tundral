//! Records every reply of a backend as `req-NNNN.json` in the shim's transcript format, so a run
//! can be replayed (`replay`) without the model. Only the request fingerprint, sizes and the
//! reply are stored, never the prompt or the documents.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;

use crate::types::{Backend, Request, Response};

use super::replay::fingerprint;

pub struct Recorder {
    inner: Box<dyn Backend>,
    dir: PathBuf,
    count: Mutex<usize>,
}

impl Recorder {
    pub fn new(inner: Box<dyn Backend>, dir: PathBuf) -> Result<Recorder> {
        std::fs::create_dir_all(&dir)?;
        let existing = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("req-"))
            .count();
        Ok(Recorder {
            inner,
            dir,
            count: Mutex::new(existing),
        })
    }
}

impl Backend for Recorder {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    fn complete(&self, req: &Request) -> Result<Response> {
        let started = Instant::now();
        let result = self.inner.complete(req);
        let latency = started.elapsed().as_secs_f64();
        let n = {
            let mut c = self.count.lock().expect("recorder lock");
            *c += 1;
            *c
        };
        let (status, response) = match &result {
            Ok(r) => (
                200,
                json!({
                    "ok": true,
                    "text": r.text,
                    "usage": {"input_tokens": r.input_tokens, "output_tokens": r.output_tokens},
                    "latency_s": (latency * 100.0).round() / 100.0,
                    "finish_reason": if r.truncated { "length" } else { "stop" },
                }),
            ),
            Err(e) => (
                502,
                json!({"ok": false, "text": "", "note": e.to_string(),
                       "latency_s": (latency * 100.0).round() / 100.0}),
            ),
        };
        let rec = json!({
            "index": n,
            "fingerprint": fingerprint(req),
            "responder": self.inner.name(),
            "codex_model": self.inner.model(),
            "request": {
                "system_chars": req.system.chars().count(),
                "user_chars": req.user_text.chars().count(),
                "images": req.images.len(),
                "max_output_tokens": req.max_output_tokens,
            },
            "status": status,
            "response": response,
        });
        std::fs::write(
            self.dir.join(format!("req-{n:04}.json")),
            serde_json::to_string_pretty(&rec)? + "\n",
        )?;
        result
    }
}
