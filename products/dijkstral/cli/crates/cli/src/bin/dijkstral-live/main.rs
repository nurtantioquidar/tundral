//! `dijkstral-live`: the long-running and networked parts of dijkstral — the file watcher behind
//! `dijkstral watch` and the PostgreSQL client behind `extract --postgres` — in their own binary.
//! The macOS frameworks the watcher (FSEvents) and the client link are loaded at every process
//! start, so keeping them out of `dijkstral` keeps its start, paid on every assistant hook call,
//! as small as a program that does nothing. `dijkstral` runs this binary from its own directory;
//! it is not meant to be run by hand.
//!
//! Protocol: `dijkstral-live <dijkstral version> watch <dijkstral exe> <root> <debounce secs>`
//! and `dijkstral-live <dijkstral version> postgres` (the DSN on stdin, the catalog snapshot as
//! JSON on stdout). Errors go to stderr as `error: …` with exit 1.

mod pg;
mod watch;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((version, rest)) = args.split_first() else {
        return usage();
    };
    if version != VERSION {
        eprintln!(
            "error: dijkstral-live {VERSION} cannot serve dijkstral {version}; reinstall dijkstral \
             so both binaries come from the same build"
        );
        return ExitCode::FAILURE;
    }
    match rest {
        [cmd, exe, root, debounce] if cmd == "watch" => {
            let Ok(debounce) = debounce.parse::<f64>() else {
                return usage();
            };
            watch::run(&PathBuf::from(exe), &PathBuf::from(root), debounce)
        }
        [cmd] if cmd == "postgres" => {
            let mut dsn = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut dsn) {
                eprintln!("error: reading the connection string: {e}");
                return ExitCode::FAILURE;
            }
            match pg::fetch(dsn.trim_end_matches(['\r', '\n'])) {
                Ok(snapshot) => {
                    let json = serde_json::to_vec(&snapshot).expect("a snapshot serialises");
                    let mut out = std::io::stdout().lock();
                    if out.write_all(&json).and_then(|()| out.flush()).is_err() {
                        return ExitCode::FAILURE;
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "dijkstral-live is run by dijkstral (`dijkstral watch`, `dijkstral extract --postgres`), \
         not by hand"
    );
    ExitCode::from(2)
}
