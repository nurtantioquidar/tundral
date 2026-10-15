//! dijkstral's block inside a git hook script: written, refreshed and removed between two marker
//! lines, so whatever else the hook runs is kept byte for byte (`docs/graphify-git.md` §2).

use std::path::Path;

pub const START: &str = "# dijkstral-hook-start";
pub const END: &str = "# dijkstral-hook-end";

/// The line of the block that names the binary; it differs between installs, not between
/// versions of the block.
const BIN_PREFIX: &str = "  _dijkstral=";

/// The block for `hook`, calling `exe` (or `dijkstral` on `PATH` when `exe` is gone).
pub fn block(hook: &str, exe: &Path) -> String {
    let after = match hook {
        "post-checkout" => "each branch switch",
        _ => "each commit",
    };
    format!(
        "{START}\n\
         # dijkstral: refresh dijkstral-out/ after {after}. The rebuild runs in the\n\
         # background and can neither slow down nor fail git. `dijkstral hook uninstall` removes it.\n\
         if [ -z \"${{DIJKSTRAL_HOOK_SKIP:-}}\" ]; then\n\
         {BIN_PREFIX}{exe}\n\
         \x20 [ -x \"$_dijkstral\" ] || _dijkstral=$(command -v dijkstral 2>/dev/null) || _dijkstral=\n\
         \x20 if [ -n \"$_dijkstral\" ]; then\n\
         \x20   \"$_dijkstral\" hook run {hook} \"$@\" </dev/null >/dev/null 2>&1 || :\n\
         \x20 fi\n\
         \x20 unset _dijkstral\n\
         fi\n\
         {END}\n",
        exe = sh_quote(&exe.to_string_lossy()),
    )
}

/// `s` as one POSIX shell word.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Where the block sits in a hook script.
#[derive(Debug, PartialEq)]
pub enum Found {
    Absent,
    /// Byte range from the start marker through the end marker's newline.
    Block(std::ops::Range<usize>),
    /// A start marker without an end marker after it.
    Malformed,
}

pub fn find(text: &str) -> Found {
    let mut start = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let end = pos + line.len();
        match (line.trim(), start) {
            (START, None) => start = Some(pos),
            (END, Some(s)) => return Found::Block(s..end),
            _ => {}
        }
        pos = end;
    }
    if start.is_some() {
        Found::Malformed
    } else {
        Found::Absent
    }
}

/// The binary a block was installed with.
pub fn installed_binary(block: &str) -> Option<String> {
    let line = block.lines().find(|l| l.starts_with(BIN_PREFIX))?;
    let word = line[BIN_PREFIX.len()..].trim();
    let inner = word.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("'\\''", "'"))
}

/// Whether an installed block is the current version of the block for `hook`, ignoring which
/// binary it names.
pub fn is_current(installed: &str, hook: &str) -> bool {
    let normalise = |s: &str| {
        s.lines()
            .map(|l| {
                if l.starts_with(BIN_PREFIX) {
                    BIN_PREFIX
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    normalise(installed) == normalise(&block(hook, Path::new("")))
}

/// Whether appending shell code keeps the script working: no shebang, or a POSIX-family shell.
pub fn is_shell(text: &str) -> bool {
    let Some(first) = text.lines().next() else {
        return true;
    };
    let Some(cmd) = first.strip_prefix("#!") else {
        return true;
    };
    let mut words = cmd.split_whitespace();
    let mut interp = words.next().unwrap_or("");
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    if base(interp) == "env" {
        interp = words.find(|w| !w.starts_with('-')).unwrap_or("");
    }
    matches!(
        base(interp).as_str(),
        "sh" | "bash" | "dash" | "zsh" | "ksh" | "ash" | "mksh"
    )
}

/// What [`install`] did to a hook script.
#[derive(Debug, PartialEq)]
pub enum Installed {
    /// A new script holding only the block.
    Created,
    /// The block appended after the existing script.
    Appended,
    /// An older block replaced in place.
    Refreshed,
    Unchanged,
}

#[derive(Debug, PartialEq)]
pub enum Refused {
    /// The existing hook is not a shell script, so shell code cannot be appended.
    NotShell,
    /// A start marker without an end marker.
    Malformed,
}

/// The script with `block` installed into `existing` (`None`: no hook yet).
pub fn install(existing: Option<&str>, block: &str) -> Result<(String, Installed), Refused> {
    let Some(text) = existing.filter(|t| !t.trim().is_empty()) else {
        return Ok((format!("#!/bin/sh\n{block}"), Installed::Created));
    };
    match find(text) {
        Found::Block(r) if &text[r.clone()] == block => {
            Ok((text.to_string(), Installed::Unchanged))
        }
        Found::Block(r) => {
            let mut out = String::with_capacity(text.len() + block.len());
            out.push_str(&text[..r.start]);
            out.push_str(block);
            out.push_str(&text[r.end..]);
            Ok((out, Installed::Refreshed))
        }
        Found::Malformed => Err(Refused::Malformed),
        Found::Absent if !is_shell(text) => Err(Refused::NotShell),
        Found::Absent => {
            let mut out = text.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(block);
            Ok((out, Installed::Appended))
        }
    }
}

/// The script without the block (and the blank line `install` put before it); `None` when it
/// has no block. A result that is empty or only a shebang means the hook can be deleted.
pub fn remove(text: &str) -> Option<String> {
    let Found::Block(r) = find(text) else {
        return None;
    };
    let mut head = &text[..r.start];
    if head.ends_with("\n\n") {
        head = &head[..head.len() - 1];
    }
    Some(format!("{head}{}", &text[r.end..]))
}

/// Nothing left but blank lines and a shebang.
pub fn is_empty_script(text: &str) -> bool {
    text.lines()
        .enumerate()
        .all(|(i, l)| l.trim().is_empty() || (i == 0 && l.starts_with("#!")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> &'static Path {
        Path::new("/opt/it's here/dijkstral")
    }

    #[test]
    fn block_round_trips_through_a_foreign_hook() {
        let b = block("post-commit", exe());
        assert!(b.starts_with(START) && b.ends_with(&format!("{END}\n")));
        assert_eq!(
            installed_binary(&b).as_deref(),
            Some("/opt/it's here/dijkstral")
        );
        assert!(is_current(&b, "post-commit"));
        assert!(!is_current(&b, "post-checkout"));

        let foreign = "#!/bin/bash\nset -e\necho user hook\n";
        let (with, how) = install(Some(foreign), &b).unwrap();
        assert_eq!(how, Installed::Appended);
        assert_eq!(install(Some(&with), &b).unwrap().1, Installed::Unchanged);
        let other = block("post-commit", Path::new("/usr/bin/dijkstral"));
        let (moved, how) = install(Some(&with), &other).unwrap();
        assert_eq!(how, Installed::Refreshed);
        assert_eq!(remove(&moved).unwrap(), foreign);
        assert_eq!(remove(foreign), None);

        let (fresh, how) = install(None, &b).unwrap();
        assert_eq!(how, Installed::Created);
        assert!(is_empty_script(&remove(&fresh).unwrap()));
    }

    #[test]
    fn refuses_what_it_cannot_extend() {
        let b = block("post-commit", exe());
        assert_eq!(
            install(Some("#!/usr/bin/env python3\nprint(1)\n"), &b),
            Err(Refused::NotShell)
        );
        assert!(is_shell("#!/usr/bin/env -S bash -e\n"));
        assert!(is_shell("echo no shebang\n"));
        assert_eq!(
            install(Some(&format!("#!/bin/sh\n{START}\necho x\n")), &b),
            Err(Refused::Malformed)
        );
    }

    #[test]
    fn foreign_text_around_the_block_is_kept() {
        let b = block("post-checkout", exe());
        let text = format!("#!/bin/sh\necho before\n{b}echo after\n");
        assert_eq!(
            remove(&text).unwrap(),
            "#!/bin/sh\necho before\necho after\n"
        );
        assert!(!is_empty_script("#!/bin/sh\necho before\n"));
    }
}
