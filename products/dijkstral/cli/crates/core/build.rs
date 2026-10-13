//! Fingerprint the extraction sources so cached per-file facts never outlive the code that
//! produced them (`cache::content_key` mixes it into every key), and the whole crate so an
//! unchanged rebuild can be skipped (`pipeline`).

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

fn main() {
    let mut files = Vec::new();
    collect(Path::new("src/extract"), &mut files);
    for f in ["src/lang.rs", "src/facts.rs", "src/ids.rs", "Cargo.toml"] {
        files.push(f.into());
    }
    files.sort();
    let mut h = DefaultHasher::new();
    for f in &files {
        println!("cargo:rerun-if-changed={f}");
        f.hash(&mut h);
        fs::read(f).unwrap_or_default().hash(&mut h);
    }
    println!("cargo:rerun-if-changed=src/extract");
    println!(
        "cargo:rustc-env=DIJKSTRAL_EXTRACTOR_FINGERPRINT={:016x}",
        h.finish()
    );

    // Everything that shapes the outputs, for skipping a rebuild whose inputs did not change.
    let mut all = Vec::new();
    collect(Path::new("src"), &mut all);
    all.push("Cargo.toml".into());
    all.sort();
    let mut h = DefaultHasher::new();
    for f in &all {
        println!("cargo:rerun-if-changed={f}");
        f.hash(&mut h);
        fs::read(f).unwrap_or_default().hash(&mut h);
    }
    println!("cargo:rerun-if-changed=src");
    println!(
        "cargo:rustc-env=DIJKSTRAL_BUILD_FINGERPRINT={:016x}",
        h.finish()
    );
}

fn collect(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p.to_string_lossy().into_owned());
        }
    }
}
