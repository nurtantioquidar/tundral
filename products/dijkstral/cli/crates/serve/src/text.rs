//! Python formatting that Graphify's text output depends on.

use std::fmt::Write;

/// Python's `repr(str)`: single quotes unless the text contains `'` and no `"`.
pub fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if (0x80..0xa0).contains(&(c as u32)) => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python's `repr(list_of_str)`.
pub fn py_repr_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let parts: Vec<String> = items.into_iter().map(py_repr).collect();
    format!("[{}]", parts.join(", "))
}

/// Python's `len(str)`: code points.
pub fn py_len(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.chars().count()
    }
}

/// An integer with `,` thousands separators (`f"{n:,}"`).
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A label as Graphify prints it in `NODE`/`EDGE` lines and the MCP node tools: line breaks
/// removed (excalidraw's multi-line destructuring labels; `path`, `explain` and `Start:` keep
/// them).
pub fn line_label(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains(['\n', '\r']) {
        s.replace(['\n', '\r'], "").into()
    } else {
        s.into()
    }
}

/// Python `str.isalnum()` for one character.
pub fn is_alnum(c: char) -> bool {
    c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_python() {
        assert_eq!(py_repr("abc"), "'abc'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(py_repr("tab\there"), "'tab\\there'");
        assert_eq!(py_repr("é"), "'é'");
        assert_eq!(py_repr_list(["a", "b"]), "['a', 'b']");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
    }
}
