//! Byte-exact Python `json.dumps` output for the sidecars Graphify writes with the standard
//! library (`indent=2`, `ensure_ascii=True`) and Python's float `repr`.

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// Keys in insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn str(s: &str) -> Json {
        Json::Str(s.to_string())
    }
}

/// `json.dumps(value, indent=2)` (ASCII-escaped, no trailing newline).
pub fn dumps_indent(value: &Json) -> String {
    let mut out = String::new();
    write_indented(value, 0, true, &mut out);
    out
}

/// `json.dumps(value, indent=2, ensure_ascii=False)`: non-ASCII text written as UTF-8.
pub fn dumps_indent_utf8(value: &Json) -> String {
    let mut out = String::new();
    write_indented(value, 0, false, &mut out);
    out
}

fn write_indented(v: &Json, depth: usize, ascii: bool, out: &mut String) {
    match v {
        Json::Array(items) if !items.is_empty() => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(depth + 1, out);
                write_indented(x, depth + 1, ascii, out);
            }
            newline(depth, out);
            out.push(']');
        }
        Json::Object(items) if !items.is_empty() => {
            out.push('{');
            for (i, (k, x)) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(depth + 1, out);
                write_str_as(k, ascii, out);
                out.push_str(": ");
                write_indented(x, depth + 1, ascii, out);
            }
            newline(depth, out);
            out.push('}');
        }
        other => write_scalar(other, ascii, out),
    }
}

fn newline(depth: usize, out: &mut String) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_scalar(v: &Json, ascii: bool, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Json::Float(f) => out.push_str(&float_repr(*f)),
        Json::Str(s) => write_str_as(s, ascii, out),
        Json::Array(_) => out.push_str("[]"),
        Json::Object(_) => out.push_str("{}"),
    }
}

/// A JSON string literal with Python's `ensure_ascii=True` escaping.
pub fn write_str(s: &str, out: &mut String) {
    write_str_as(s, true, out);
}

/// A JSON string literal as Python escapes it; without `ascii`, only `"`, `\` and control
/// characters are escaped.
fn write_str_as(s: &str, ascii: bool, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            c if !ascii && c >= ' ' => out.push(c),
            _ => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Python's `repr(float)`: shortest round-trip digits, scientific notation below 1e-4 and from
/// 1e16 up.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent value");
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::from(sign);
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.push_str(&digits);
                out.push_str(&"0".repeat(int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.push_str(&"0".repeat((-exp - 1) as usize));
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_like_python() {
        for (x, want) in [
            (1e-05, "1e-05"),
            (1.5e-07, "1.5e-07"),
            (1e16, "1e+16"),
            (123.0, "123.0"),
            (0.1, "0.1"),
            (0.059940059940059943, "0.059940059940059943"),
            (1.0 / 3.0, "0.3333333333333333"),
            (2.5e-5, "2.5e-05"),
            (0.0001, "0.0001"),
            (1.0, "1.0"),
        ] {
            assert_eq!(float_repr(x), want);
        }
    }

    #[test]
    fn indent_and_escaping() {
        let v = Json::Object(vec![
            (
                "a".into(),
                Json::Array(vec![Json::str("Café"), Json::Int(3)]),
            ),
            ("b".into(), Json::Object(vec![])),
            ("c".into(), Json::Array(vec![])),
        ]);
        assert_eq!(
            dumps_indent(&v),
            "{\n  \"a\": [\n    \"Caf\\u00e9\",\n    3\n  ],\n  \"b\": {},\n  \"c\": []\n}"
        );
    }
}
