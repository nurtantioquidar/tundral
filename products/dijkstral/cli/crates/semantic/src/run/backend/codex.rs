//! The Codex CLI backend: each request is one non-interactive `codex exec` run on the user's
//! Codex (ChatGPT) login, with no API key.
//!
//! The run is isolated the way `bench/llm_shim.py` drives it: an ephemeral session, the user's
//! `~/.codex/config.toml`, rules and project docs ignored, a read-only sandbox, and an empty
//! temporary working directory, so the model sees nothing but the request. The model and the
//! reasoning effort are always passed explicitly. dijkstral never writes Codex's configuration.
//! Images are attached with `-i`; the final message and the token usage come from the JSON event
//! stream (`--json`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::types::{Backend, Request, Response};

pub const DEFAULT_MODEL: &str = "gpt-6.1-sol";
pub const DEFAULT_EFFORT: &str = "medium";

const PREAMBLE: &str = "\
You are answering one request from dijkstral, a non-interactive program. Do not run commands, \
read files or ask questions: everything you need is below. Follow the system instructions and \
reply with the final answer only.";

pub struct Codex {
    bin: PathBuf,
    model: String,
    effort: String,
    timeout: Duration,
}

impl Codex {
    pub fn new(bin: PathBuf, model: String, effort: String, timeout: Duration) -> Codex {
        Codex {
            bin,
            model,
            effort,
            timeout,
        }
    }

    /// The `codex exec` arguments for a run in `dir` with `images` attached.
    pub fn args(&self, dir: &Path, images: &[PathBuf]) -> Vec<String> {
        let mut args: Vec<String> = [
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "--ignore-user-config",
            "--ignore-rules",
            "-s",
            "read-only",
            "-C",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        args.push(dir.display().to_string());
        args.extend([
            "-m".into(),
            self.model.clone(),
            "-c".into(),
            format!("model_reasoning_effort=\"{}\"", self.effort),
            "-c".into(),
            "project_doc_max_bytes=0".into(),
            "--json".into(),
            "-o".into(),
            dir.join("last.txt").display().to_string(),
            "--color".into(),
            "never".into(),
        ]);
        for img in images {
            args.push("-i".into());
            args.push(img.display().to_string());
        }
        args.push("-".into());
        args
    }
}

/// The prompt Codex reads on stdin.
pub fn flatten(req: &Request) -> String {
    let mut s = format!("{PREAMBLE}\n\n");
    if !req.system.is_empty() {
        s.push_str("<<system>>\n");
        s.push_str(&req.system);
        s.push_str("\n\n");
    }
    s.push_str("<<user>>\n");
    s.push_str(&req.user_text);
    for (i, (_, _, rel)) in req.images.iter().enumerate() {
        s.push_str(&format!("\n[attached image {}: {rel}]", i + 1));
    }
    s.push_str("\n\n<<assistant>>\n");
    s
}

impl Backend for Codex {
    fn name(&self) -> &str {
        "codex"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn complete(&self, req: &Request) -> Result<Response> {
        let dir = TempDir::new()?;
        let mut images = Vec::new();
        for (i, (mime, bytes, _)) in req.images.iter().enumerate() {
            let ext = mime
                .rsplit('/')
                .next()
                .unwrap_or("png")
                .replace("jpeg", "jpg");
            let p = dir.0.join(format!("image{i}.{ext}"));
            std::fs::write(&p, bytes)?;
            images.push(p);
        }
        let out = run(
            &self.bin,
            &self.args(&dir.0, &images),
            &dir.0,
            flatten(req),
            self.timeout,
        )?;
        let (mut text, usage) = parse_events(&out.stdout);
        if text.is_empty() {
            text = std::fs::read_to_string(dir.0.join("last.txt")).unwrap_or_default();
        }
        if !out.success {
            let tail: String = out
                .stderr
                .chars()
                .rev()
                .take(300)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            bail!("codex exec failed: {}", tail.trim());
        }
        Ok(Response {
            text,
            input_tokens: usage.0,
            output_tokens: usage.1,
            model: self.model.clone(),
            truncated: false,
        })
    }
}

/// The last agent message and the turn's `(input, output)` tokens from `codex exec --json`.
pub fn parse_events(stdout: &str) -> (String, (Option<u64>, Option<u64>)) {
    let mut text = String::new();
    let mut usage = (None, None);
    for line in stdout.lines() {
        let Ok(ev) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match ev.get("type").and_then(Value::as_str) {
            Some("turn.completed") => {
                let u = &ev["usage"];
                usage = (u["input_tokens"].as_u64(), u["output_tokens"].as_u64());
            }
            Some("item.completed") if ev["item"]["type"] == "agent_message" => {
                if let Some(t) = ev["item"]["text"].as_str() {
                    text = t.to_string();
                }
            }
            _ => {}
        }
    }
    (text, usage)
}

struct Output {
    success: bool,
    stdout: String,
    stderr: String,
}

fn run(
    bin: &Path,
    args: &[String],
    cwd: &Path,
    stdin: String,
    timeout: Duration,
) -> Result<Output> {
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", bin.display()))?;
    let mut input = child.stdin.take().context("codex stdin")?;
    let writer = std::thread::spawn(move || {
        let _ = input.write_all(stdin.as_bytes());
    });
    let mut so = child.stdout.take().context("codex stdout")?;
    let mut se = child.stderr.take().context("codex stderr")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let ereader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("codex exec timed out after {} s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = writer.join();
    Ok(Output {
        success: status.success(),
        stdout: reader.join().unwrap_or_default(),
        stderr: ereader.join().unwrap_or_default(),
    })
}

/// `DIJKSTRAL_CODEX`, else `codex` on `PATH`, else `~/.local/bin/codex`.
pub fn find_binary(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(p) = var("DIJKSTRAL_CODEX").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let mut candidates: Vec<PathBuf> = var("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("codex")).collect())
        .unwrap_or_default();
    if let Some(home) = var("HOME") {
        candidates.push(Path::new(&home).join(".local/bin/codex"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Whether `codex login status` reports a login (no model call).
pub fn logged_in(bin: &Path) -> bool {
    let Ok(out) = run(
        bin,
        &["login".into(), "status".into()],
        &std::env::temp_dir(),
        String::new(),
        Duration::from_secs(20),
    ) else {
        return false;
    };
    out.success && format!("{}{}", out.stdout, out.stderr).contains("Logged in")
}

/// An empty directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<TempDir> {
        static N: AtomicUsize = AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "dijkstral-codex-{}-{}-{nanos}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in `codex` that checks its arguments and answers like `codex exec --json`.
    fn fake_codex(dir: &Path, reply: &str) -> PathBuf {
        let bin = dir.join("codex");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = login ]; then echo "Logged in using ChatGPT"; exit 0; fi
args="$*"
echo "$args" > "{log}"
cat > "{stdin}"
case "$args" in *"--ephemeral"*"--ignore-user-config"*"--ignore-rules"*"-s read-only"*) ;; *) echo bad args >&2; exit 2;; esac
pwd > "{cwd}"
ls -A > "{listing}"
echo '{{"type":"thread.started"}}'
printf '%s\n' '{{"type":"item.completed","item":{{"type":"agent_message","text":{reply}}}}}'
echo '{{"type":"turn.completed","usage":{{"input_tokens":120,"cached_input_tokens":100,"output_tokens":7}}}}'
"#,
            log = dir.join("args.txt").display(),
            stdin = dir.join("stdin.txt").display(),
            cwd = dir.join("cwd.txt").display(),
            listing = dir.join("listing.txt").display(),
            reply = serde_json::to_string(reply).unwrap(),
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[test]
    fn runs_codex_isolated_with_explicit_model_and_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = fake_codex(tmp.path(), r#"{"nodes":[]}"#);
        assert!(logged_in(&bin));
        let c = Codex::new(bin, "gpt-x".into(), "high".into(), Duration::from_secs(30));
        let r = c
            .complete(&Request {
                system: "SYS".into(),
                user_text: "USER".into(),
                images: vec![("image/png".into(), vec![137, 80], "a/b.png".into())],
                max_output_tokens: 10,
            })
            .unwrap();
        assert_eq!(r.text, r#"{"nodes":[]}"#);
        assert_eq!((r.input_tokens, r.output_tokens), (Some(120), Some(7)));
        let args = std::fs::read_to_string(tmp.path().join("args.txt")).unwrap();
        assert!(
            args.contains("-m gpt-x -c model_reasoning_effort=\"high\""),
            "{args}"
        );
        assert!(
            args.contains(" -i ") && args.contains("image0.png"),
            "{args}"
        );
        let stdin = std::fs::read_to_string(tmp.path().join("stdin.txt")).unwrap();
        assert!(stdin.contains("<<system>>\nSYS") && stdin.contains("<<user>>\nUSER"));
        assert!(stdin.contains("[attached image 1: a/b.png]"));
        // The working directory held only the attached image; it is removed afterwards.
        let listing = std::fs::read_to_string(tmp.path().join("listing.txt")).unwrap();
        assert_eq!(listing.trim(), "image0.png");
        let cwd = std::fs::read_to_string(tmp.path().join("cwd.txt")).unwrap();
        assert!(!Path::new(cwd.trim()).exists());
    }

    #[test]
    fn reports_failures_and_timeouts() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("codex");
        std::fs::write(&bin, "#!/bin/sh\necho 'not logged in' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!logged_in(&bin));
        let req = Request {
            system: String::new(),
            user_text: "x".into(),
            images: vec![],
            max_output_tokens: 1,
        };
        let c = Codex::new(
            bin.clone(),
            "m".into(),
            "low".into(),
            Duration::from_secs(5),
        );
        let e = c.complete(&req).unwrap_err().to_string();
        assert!(e.contains("not logged in"), "{e}");
        std::fs::write(&bin, "#!/bin/sh\nsleep 5\n").unwrap();
        let c = Codex::new(bin, "m".into(), "low".into(), Duration::from_millis(200));
        let e = c.complete(&req).unwrap_err().to_string();
        assert!(e.contains("timed out"), "{e}");
    }

    #[test]
    fn finds_the_binary() {
        let tmp = tempfile::tempdir().unwrap();
        fake_codex(tmp.path(), "");
        let path = tmp.path().display().to_string();
        let var = |k: &str| (k == "PATH").then(|| path.clone());
        assert_eq!(find_binary(&var), Some(tmp.path().join("codex")));
        assert_eq!(find_binary(&|_: &str| None), None);
        let var = |k: &str| (k == "DIJKSTRAL_CODEX").then(|| "/opt/codex".to_string());
        assert_eq!(find_binary(&var), Some(PathBuf::from("/opt/codex")));
    }
}
