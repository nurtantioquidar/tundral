//! Community naming with a model (§10): batches of up to 100 communities per request, a
//! `{"<cid>": "<name>"}` reply, and halving on failure down to single communities. The caller
//! (`dijkstral label`, `cluster-only`) chooses each community's member labels and keeps the
//! structural fallback name for the communities left unnamed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use crate::types::Backend;

use super::prompt::label_request;
use super::repair::first_object;
use super::usage::{UsageLog, UsageReport};

/// One community to name.
#[derive(Debug, Clone, PartialEq)]
pub struct Community {
    /// The community id as written in the graph (`"0"`, `"12"`).
    pub id: String,
    /// Member labels, most representative first.
    pub labels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LabelOptions {
    /// `--batch-size` (default 100).
    pub batch_size: usize,
    /// `--max-concurrency` (default 4).
    pub max_concurrency: usize,
    /// Member labels shown per community (default 12).
    pub max_labels: usize,
}

impl Default for LabelOptions {
    fn default() -> Self {
        LabelOptions {
            batch_size: 100,
            max_concurrency: 4,
            max_labels: 12,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LabelOutcome {
    /// Names by community id.
    pub names: BTreeMap<String, String>,
    /// Communities still unnamed (they keep their structural names), in input order.
    pub unnamed: Vec<String>,
    pub warnings: Vec<String>,
    pub usage: UsageReport,
}

impl LabelOutcome {
    /// `labeled N of M communities; K kept structural fallback names`.
    pub fn summary(&self) -> String {
        let total = self.names.len() + self.unnamed.len();
        let mut s = format!("labeled {} of {total} communities", self.names.len());
        if !self.unnamed.is_empty() {
            s.push_str(&format!(
                "; {} kept structural fallback names",
                self.unnamed.len()
            ));
        }
        s
    }
}

const MAX_NAME_CHARS: usize = 80;
const MAX_LABEL_CHARS: usize = 60;

/// Names `communities` through `backend`.
pub fn name_communities(
    communities: &[Community],
    backend: &dyn Backend,
    opts: &LabelOptions,
) -> LabelOutcome {
    let shown: Vec<(String, Vec<String>)> = communities
        .iter()
        .map(|c| {
            let labels = c
                .labels
                .iter()
                .map(|l| {
                    let l = l.strip_suffix("()").unwrap_or(l);
                    l.chars().take(MAX_LABEL_CHARS).collect::<String>()
                })
                .filter(|l| !l.is_empty())
                .take(opts.max_labels)
                .collect();
            (c.id.clone(), labels)
        })
        .collect();
    let batches: Vec<&[(String, Vec<String>)]> = shown.chunks(opts.batch_size.max(1)).collect();
    let usage = UsageLog::new();
    let names: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
    let warnings: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    let requests = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let workers = opts.max_concurrency.max(1).min(batches.len());
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let b = next.fetch_add(1, Ordering::SeqCst);
                if b >= batches.len() {
                    break;
                }
                let ctx = Ctx {
                    backend,
                    usage: &usage,
                    names: &names,
                    warnings: &warnings,
                    requests: &requests,
                    batch: b,
                };
                ctx.name(batches[b]);
            });
        }
    });
    let names = names.into_inner().expect("names lock");
    let unnamed = communities
        .iter()
        .filter(|c| !names.contains_key(&c.id))
        .map(|c| c.id.clone())
        .collect();
    let mut warnings = warnings.into_inner().expect("warnings lock");
    warnings.sort_by_key(|(b, _)| *b);
    LabelOutcome {
        names,
        unnamed,
        warnings: warnings.into_iter().map(|(_, w)| w).collect(),
        usage: usage.report(backend.name(), backend.model()),
    }
}

struct Ctx<'a> {
    backend: &'a dyn Backend,
    usage: &'a UsageLog,
    names: &'a Mutex<BTreeMap<String, String>>,
    warnings: &'a Mutex<Vec<(usize, String)>>,
    requests: &'a AtomicUsize,
    batch: usize,
}

impl Ctx<'_> {
    fn name(&self, batch: &[(String, Vec<String>)]) {
        if batch.is_empty() {
            return;
        }
        let req = label_request(batch);
        let attempt = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        let started = Instant::now();
        let result = self.backend.complete(&req);
        let wall = started.elapsed();
        let (named, outcome, tokens) = match &result {
            Ok(r) => {
                let tokens = (r.input_tokens, r.output_tokens);
                match first_object(&r.text, |_| true) {
                    Ok(map) => {
                        let mut got = Vec::new();
                        for (cid, _) in batch {
                            let name = match map.get(cid) {
                                Some(Value::String(s)) => s.trim(),
                                _ => continue,
                            };
                            if !name.is_empty() {
                                got.push((
                                    cid.clone(),
                                    name.chars().take(MAX_NAME_CHARS).collect(),
                                ));
                            }
                        }
                        let outcome = if got.is_empty() { "invalid" } else { "ok" };
                        (got, outcome, tokens)
                    }
                    Err(_) => (Vec::new(), "invalid", tokens),
                }
            }
            Err(_) => (Vec::new(), "error", (None, None)),
        };
        self.usage.add(
            "label",
            self.batch + 1,
            attempt,
            batch.len(),
            tokens,
            wall,
            outcome,
        );
        if let Err(e) = &result {
            self.warn(format!("naming request failed: {e:#}"));
        }
        let got_any = !named.is_empty();
        let mut names = self.names.lock().expect("names lock");
        let missing: Vec<(String, Vec<String>)> = batch
            .iter()
            .filter(|(cid, _)| !named.iter().any(|(n, _)| n == cid))
            .cloned()
            .collect();
        names.extend(named);
        drop(names);
        if missing.is_empty() {
            return;
        }
        if got_any {
            // Ask again for the ones the reply skipped (a strictly smaller batch).
            self.name(&missing);
        } else if batch.len() > 1 {
            let mid = batch.len() / 2;
            self.warn(format!(
                "unusable naming reply for {} communities; retrying halves of {} and {}",
                batch.len(),
                mid,
                batch.len() - mid
            ));
            self.name(&batch[..mid]);
            self.name(&batch[mid..]);
        }
    }

    fn warn(&self, msg: String) {
        self.warnings
            .lock()
            .expect("warnings lock")
            .push((self.batch, msg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Request, Response};

    /// Names every community it is asked about unless the batch has more than `max` of them.
    struct Namer {
        max: usize,
        skip: Option<&'static str>,
        calls: Mutex<Vec<usize>>,
    }

    impl Backend for Namer {
        fn name(&self) -> &str {
            "fake"
        }
        fn model(&self) -> &str {
            "fake-1"
        }
        fn complete(&self, req: &Request) -> anyhow::Result<Response> {
            let ids: Vec<&str> = req
                .user_text
                .lines()
                .filter_map(|l| l.split_once(": "))
                .map(|(id, _)| id)
                .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
                .collect();
            self.calls.lock().unwrap().push(ids.len());
            assert_eq!(req.max_output_tokens, 256 + 48 * ids.len() as u32);
            if ids.len() > self.max {
                return Ok(Response {
                    text: "sorry, too many".into(),
                    ..Response::default()
                });
            }
            let obj: serde_json::Map<String, Value> = ids
                .iter()
                .filter(|id| Some(**id) != self.skip)
                .map(|id| (id.to_string(), Value::String(format!("Group {id}"))))
                .collect();
            Ok(Response {
                text: format!("```json\n{}\n```", Value::Object(obj)),
                input_tokens: Some(10),
                output_tokens: Some(5),
                ..Response::default()
            })
        }
    }

    fn communities(n: usize) -> Vec<Community> {
        (0..n)
            .map(|i| Community {
                id: i.to_string(),
                labels: vec![format!("run_{i}()"), "Thing".into()],
            })
            .collect()
    }

    #[test]
    fn batches_and_names() {
        let b = Namer {
            max: 100,
            skip: None,
            calls: Mutex::new(vec![]),
        };
        let opts = LabelOptions {
            batch_size: 2,
            max_concurrency: 1,
            ..LabelOptions::default()
        };
        let out = name_communities(&communities(5), &b, &opts);
        assert_eq!(*b.calls.lock().unwrap(), vec![2, 2, 1]);
        assert_eq!(out.names.len(), 5);
        assert_eq!(out.names["3"], "Group 3");
        assert_eq!(out.summary(), "labeled 5 of 5 communities");
        assert_eq!(out.usage.requests, 3);
        assert_eq!(out.usage.input_tokens, 30);
    }

    #[test]
    fn halves_on_failure_down_to_singles() {
        // Nothing usable at any size: 5 -> 2 + 3 -> 1 + 1 + 1 + 2 -> 1 + 1 = 9 requests.
        let b = Namer {
            max: 0,
            skip: None,
            calls: Mutex::new(vec![]),
        };
        let out = name_communities(&communities(5), &b, &LabelOptions::default());
        assert_eq!(b.calls.lock().unwrap().len(), 9);
        assert!(out.names.is_empty());
        assert_eq!(out.unnamed, vec!["0", "1", "2", "3", "4"]);
        assert_eq!(
            out.summary(),
            "labeled 0 of 5 communities; 5 kept structural fallback names"
        );
        // Usable at two or fewer: 5 -> 2 + 3 -> 1 + 2.
        let b = Namer {
            max: 2,
            skip: None,
            calls: Mutex::new(vec![]),
        };
        let out = name_communities(&communities(5), &b, &LabelOptions::default());
        assert_eq!(*b.calls.lock().unwrap(), vec![5, 2, 3, 1, 2]);
        assert_eq!(out.names.len(), 5);
    }

    #[test]
    fn asks_again_for_skipped_communities() {
        let b = Namer {
            max: 100,
            skip: Some("1"),
            calls: Mutex::new(vec![]),
        };
        let out = name_communities(&communities(3), &b, &LabelOptions::default());
        assert_eq!(*b.calls.lock().unwrap(), vec![3, 1]);
        assert_eq!(out.unnamed, vec!["1"]);
    }

    #[test]
    fn prompt_strips_call_parentheses() {
        let req = label_request(&[("7".into(), vec!["run_7".into(), "Thing".into()])]);
        assert!(req.user_text.ends_with("7: run_7, Thing\n"));
    }
}
