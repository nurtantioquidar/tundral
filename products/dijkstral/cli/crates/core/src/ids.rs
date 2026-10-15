//! Node identifiers compatible with Graphify's schema 1.
//!
//! An id is built from parts joined by `_`, case-folded and NFKC-normalized until stable, with
//! every run of non-word characters replaced by a single `_` and leading/trailing `_` removed.

use unicode_normalization::UnicodeNormalization;

/// Build an id from parts. Empty parts are kept (they collapse away with the separators).
pub fn make_id<S: AsRef<str>>(parts: &[S]) -> String {
    let mut joined = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            joined.push('_');
        }
        joined.push_str(part.as_ref());
    }
    normalize(&joined)
}

/// Normalize free text the way ids are normalized.
pub fn normalize(text: &str) -> String {
    let mut s = text.to_string();
    loop {
        let next: String = casefold(&s).nfkc().collect();
        if next == s {
            break;
        }
        s = next;
    }
    let mut out = String::with_capacity(s.len());
    let mut pending_sep = false;
    for c in s.chars() {
        if is_word(c) && c != '_' {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.push(c);
        } else {
            // `_` and any run of non-word characters collapse to one separator.
            pending_sep = true;
        }
    }
    out
}

/// Python's `\w` for str patterns: Unicode letters, digits and underscore. Combining marks (the
/// vowel signs of Devanagari headings, `परिचय` → `पर_चय`) are alphabetic in Rust but not
/// `isalnum()` in Python, so they separate words.
fn is_word(c: char) -> bool {
    c == '_' || (c.is_alphanumeric() && !unicode_normalization::char::is_combining_mark(c))
}

/// Unicode default case folding for the characters where it differs from lowercasing.
fn casefold(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'ß' | 'ẞ' => out.push_str("ss"),
            'ſ' => out.push('s'),
            'ς' => out.push('σ'),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// The id of a source file: its repo-relative path without the final extension.
pub fn file_id(rel_path: &str) -> String {
    make_id(&[strip_extension(rel_path)])
}

/// `a/b/c.tar.gz` -> `a/b/c.tar`; dotfiles such as `.eslintrc` keep their name.
pub fn strip_extension(rel_path: &str) -> &str {
    let name_start = rel_path.rfind('/').map_or(0, |i| i + 1);
    match rel_path[name_start..].rfind('.') {
        Some(dot) if dot > 0 => &rel_path[..name_start + dot],
        _ => rel_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ids_drop_extension_and_join_path() {
        assert_eq!(
            file_id("crates/globset/src/pathutil.rs"),
            "crates_globset_src_pathutil"
        );
        assert_eq!(file_id("setup.py"), "setup");
        assert_eq!(file_id("src/auth/session.py"), "src_auth_session");
        assert_eq!(file_id(".eslintrc"), "eslintrc");
    }

    #[test]
    fn symbols_are_casefolded_and_collapsed() {
        assert_eq!(make_id(&["src_app", "HttpClient"]), "src_app_httpclient");
        assert_eq!(make_id(&["a", "__init__"]), "a_init");
        assert_eq!(make_id(&["x--y", "", "z"]), "x_y_z");
        assert_eq!(make_id(&["Straße"]), "strasse");
        assert_eq!(make_id(&["ﬁle"]), "file");
        // Graphify's id of the Hindi heading `परिचय (About)`.
        assert_eq!(make_id(&["परिचय (About)"]), "पर_चय_about");
    }
}
