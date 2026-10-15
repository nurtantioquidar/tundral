//! `dijkstral hook install | uninstall | status` (`docs/graphify-git.md` §2–§3): the two hooks
//! and the merge driver's config and `.gitattributes` entry, one report line each.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use dijkstral_core::pipeline::OUT_DIR;

use crate::hooks::{self, Found, Installed, Refused};
use crate::repo::Repo;
use crate::{attributes, DRIVER, HOOKS};

/// What a command did, line by line; `ok` is false when part of it was refused or failed.
pub struct Report {
    pub lines: Vec<String>,
    pub ok: bool,
}

impl Report {
    fn new() -> Report {
        Report {
            lines: Vec::new(),
            ok: true,
        }
    }

    fn line(&mut self, what: &str, text: impl AsRef<str>) {
        self.lines.push(format!("{what}: {}", text.as_ref()));
    }

    fn fail(&mut self, what: &str, text: impl AsRef<str>) {
        self.ok = false;
        self.line(what, text);
    }
}

const MERGE: &str = "merge driver";

fn read(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(unix)]
fn make_executable(path: &Path, was_present: bool) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = fs::metadata(path)?.permissions();
    let mode = perm.mode();
    let wanted = if was_present {
        // Execute wherever it can be read.
        mode | ((mode & 0o444) >> 2)
    } else {
        0o755
    };
    if wanted == mode {
        return Ok(false);
    }
    perm.set_mode(wanted);
    fs::set_permissions(path, perm)?;
    Ok(was_present && mode & 0o100 == 0)
}

#[cfg(not(unix))]
fn make_executable(_: &Path, _: bool) -> Result<bool> {
    Ok(false)
}

fn driver_command(exe: &Path) -> String {
    format!(
        "{} merge-driver %O %A %B",
        hooks::sh_quote(&exe.to_string_lossy())
    )
}

pub fn install(repo: &Repo, exe: &Path) -> Report {
    let mut r = Report::new();
    for hook in HOOKS {
        if let Err(e) = install_hook(repo, hook, exe, &mut r) {
            r.fail(hook, format!("failed: {e:#}"));
        }
    }
    if let Err(e) = install_driver(repo, exe, &mut r) {
        r.fail(MERGE, format!("failed: {e:#}"));
    }
    if !repo.top.join(OUT_DIR).join("graph.json").is_file() {
        r.lines.push(format!(
            "note: there is no {OUT_DIR}/graph.json yet; the hooks stay idle until \
             `dijkstral extract` or `dijkstral update` builds one"
        ));
    }
    r
}

fn install_hook(repo: &Repo, hook: &str, exe: &Path, r: &mut Report) -> Result<()> {
    let path = repo.hook_path(hook);
    let existing = read(&path)?;
    let block = hooks::block(hook, exe);
    let shown = path.display();
    match hooks::install(existing.as_deref(), &block) {
        Err(Refused::NotShell) => r.fail(
            hook,
            format!(
                "skipped: {shown} is not a shell script; call \
                 `dijkstral hook run {hook} \"$@\"` from it yourself"
            ),
        ),
        Err(Refused::Malformed) => r.fail(
            hook,
            format!(
                "skipped: {shown} has `{}` without `{}`; fix it by hand",
                hooks::START,
                hooks::END
            ),
        ),
        Ok((_, Installed::Unchanged)) => {
            make_executable(&path, true)?;
            r.line(hook, format!("already installed ({shown})"));
        }
        Ok((text, how)) => {
            fs::create_dir_all(&repo.hooks_dir)?;
            fs::write(&path, text).with_context(|| format!("writing {shown}"))?;
            let woke = make_executable(&path, existing.is_some())?;
            let mut msg = match how {
                Installed::Created => format!("installed ({shown})"),
                Installed::Appended => format!("added to the existing hook {shown}"),
                _ => format!("refreshed ({shown})"),
            };
            if woke {
                msg.push_str("; the hook was not executable and now is");
            }
            r.line(hook, msg);
        }
    }
    Ok(())
}

fn install_driver(repo: &Repo, exe: &Path, r: &mut Report) -> Result<()> {
    let name_key = format!("merge.{DRIVER}.name");
    let driver_key = format!("merge.{DRIVER}.driver");
    let command = driver_command(exe);
    let mut changed = false;
    if repo.config_get(&driver_key)?.as_deref() != Some(command.as_str()) {
        repo.config_set(&name_key, "dijkstral graph.json merge")?;
        repo.config_set(&driver_key, &command)?;
        changed = true;
    }
    let attrs = repo.top.join(".gitattributes");
    if let Some(text) = attributes::add(&read(&attrs)?.unwrap_or_default()) {
        fs::write(&attrs, text).with_context(|| format!("writing {}", attrs.display()))?;
        changed = true;
    }
    if changed {
        r.line(
            MERGE,
            format!(
                "configured (git config merge.{DRIVER}.*, `{}` in .gitattributes)",
                attributes::line()
            ),
        );
    } else {
        r.line(MERGE, "already configured");
    }
    Ok(())
}

pub fn uninstall(repo: &Repo) -> Report {
    let mut r = Report::new();
    for hook in HOOKS {
        if let Err(e) = uninstall_hook(repo, hook, &mut r) {
            r.fail(hook, format!("failed: {e:#}"));
        }
    }
    if let Err(e) = uninstall_driver(repo, &mut r) {
        r.fail(MERGE, format!("failed: {e:#}"));
    }
    r
}

fn uninstall_hook(repo: &Repo, hook: &str, r: &mut Report) -> Result<()> {
    let path = repo.hook_path(hook);
    let shown = path.display();
    let Some(text) = read(&path)? else {
        r.line(hook, "nothing to remove");
        return Ok(());
    };
    match hooks::remove(&text) {
        None => r.line(hook, "nothing to remove"),
        Some(rest) if hooks::is_empty_script(&rest) => {
            fs::remove_file(&path)?;
            r.line(hook, format!("removed {shown}"));
        }
        Some(rest) => {
            fs::write(&path, rest)?;
            r.line(
                hook,
                format!("removed dijkstral's block from {shown}; the rest is kept"),
            );
        }
    }
    Ok(())
}

fn uninstall_driver(repo: &Repo, r: &mut Report) -> Result<()> {
    let mut removed = Vec::new();
    if repo.config_remove_section(&format!("merge.{DRIVER}"))? {
        removed.push(format!("git config merge.{DRIVER}.*"));
    }
    let attrs = repo.top.join(".gitattributes");
    if let Some(rest) = read(&attrs)?.as_deref().and_then(attributes::remove) {
        if rest.trim().is_empty() {
            fs::remove_file(&attrs)?;
            removed.push(".gitattributes (nothing else was in it)".to_string());
        } else {
            fs::write(&attrs, rest)?;
            removed.push("its .gitattributes entry".to_string());
        }
    }
    if removed.is_empty() {
        r.line(MERGE, "nothing to remove");
    } else {
        r.line(MERGE, format!("removed {}", removed.join(" and ")));
    }
    Ok(())
}

pub fn status(repo: &Repo) -> Report {
    let mut r = Report::new();
    for hook in HOOKS {
        let text = match read(&repo.hook_path(hook)) {
            Ok(t) => t,
            Err(e) => {
                r.fail(hook, format!("unreadable: {e:#}"));
                continue;
            }
        };
        let state = match text.as_deref().map(|t| (hooks::find(t), t)) {
            None => "not installed".to_string(),
            Some((Found::Absent, _)) => {
                "not installed (a hook exists, without dijkstral's block)".to_string()
            }
            Some((Found::Malformed, _)) => format!(
                "broken (`{}` without `{}`); fix it by hand",
                hooks::START,
                hooks::END
            ),
            Some((Found::Block(range), t)) => {
                let block = &t[range];
                let binary =
                    hooks::installed_binary(block).is_some_and(|b| is_executable(Path::new(&b)));
                if hooks::is_current(block, hook) && binary {
                    "installed".to_string()
                } else {
                    "installed, outdated (run `dijkstral hook install` again)".to_string()
                }
            }
        };
        r.line(hook, state);
    }
    let config = repo
        .config_get(&format!("merge.{DRIVER}.driver"))
        .ok()
        .flatten()
        .is_some();
    let attrs = read(&repo.top.join(".gitattributes"))
        .ok()
        .flatten()
        .is_some_and(|t| attributes::has_entry(&t));
    let state = match (config, attrs) {
        (true, true) => "configured",
        (true, false) => "half configured (git config set, .gitattributes entry missing)",
        (false, true) => "half configured (.gitattributes entry set, git config missing)",
        (false, false) => "not configured",
    };
    r.line(MERGE, state);
    r
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}
