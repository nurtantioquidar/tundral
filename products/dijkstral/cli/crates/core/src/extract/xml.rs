//! A strict, non-validating XML 1.0 reader for the XML-based project and markup files
//! (`.csproj`, `.slnx`, `.xaml`, `.lpk`; `docs/graphify-format.md` §43, §44).
//!
//! Graphify reads these with a conforming, namespace-aware parser and emits nothing at all for a
//! file it cannot parse, so this reader rejects what such a parser rejects: unbalanced or
//! unclosed tags, content after the root element, an XML declaration not at the very start,
//! unquoted, duplicate or `<`-containing attribute values, undefined entities, `--` inside
//! comments, `]]>` in text, control characters, and prefixes with no `xmlns:` declaration in
//! scope. A DOCTYPE is skipped (its entity declarations are not honoured).

/// One element: its qualified name as written, attributes in document order, children, and the
/// 1-based line of its `<`.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<XmlAttr>,
    pub children: Vec<Element>,
    /// Character data directly inside the element before its first child (ElementTree's `text`).
    pub text: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct XmlAttr {
    /// Qualified name as written (`x:Class`).
    pub name: String,
    /// Decoded value (entities expanded, whitespace characters normalised to spaces).
    pub value: String,
    /// 1-based line of the attribute name.
    pub line: u32,
}

impl Element {
    /// The name without its prefix.
    pub fn local(&self) -> &str {
        local(&self.name)
    }

    /// The prefix, if any.
    pub fn prefix(&self) -> Option<&str> {
        self.name.split_once(':').map(|(p, _)| p)
    }

    /// The value of the attribute with exactly this qualified name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }

    /// The element and every descendant, in document order.
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            out.push(e);
            for c in e.children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }
}

/// The part of a qualified name after its prefix.
pub fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

/// Parse a document; `None` when it is not well-formed.
pub fn parse(src: &[u8]) -> Option<Element> {
    let text = std::str::from_utf8(src).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        line: 1,
        ns: vec![Vec::new()],
    };
    p.document()
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    line: u32,
    /// Declared prefixes per open element (`xml` is always bound).
    ns: Vec<Vec<String>>,
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':' || (c as u32) >= 0x80 && !c.is_whitespace()
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.' || c == '\u{b7}'
}

fn is_xml_char(c: char) -> bool {
    ((c as u32) >= 0x20 || matches!(c, '\t' | '\n' | '\r')) && c != '\u{fffe}' && c != '\u{ffff}'
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn starts(&self, lit: &str) -> bool {
        self.s[self.i..].starts_with(lit.as_bytes())
    }

    /// Advance over `n` bytes, counting newlines.
    fn bump(&mut self, n: usize) {
        let end = (self.i + n).min(self.s.len());
        self.line += self.s[self.i..end].iter().filter(|&&b| b == b'\n').count() as u32;
        self.i = end;
    }

    fn skip_ws(&mut self) -> bool {
        let start = self.i;
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.bump(1);
            } else {
                break;
            }
        }
        self.i > start
    }

    fn name(&mut self) -> Option<String> {
        let rest = &self.text[self.i..];
        let mut end = 0;
        for (k, c) in rest.char_indices() {
            let ok = if k == 0 {
                is_name_start(c)
            } else {
                is_name_char(c)
            };
            if !ok {
                break;
            }
            end = k + c.len_utf8();
        }
        if end == 0 {
            return None;
        }
        let name = rest[..end].to_string();
        self.bump(end);
        // A namespace-aware parser allows at most one colon, not at either end.
        let colons = name.matches(':').count();
        if colons > 1 || name.starts_with(':') || name.ends_with(':') {
            return None;
        }
        Some(name)
    }

    fn document(&mut self) -> Option<Element> {
        if self.starts("<?xml") && self.s.get(5).is_some_and(|b| b.is_ascii_whitespace()) {
            self.pi()?;
        }
        let mut root = None;
        loop {
            self.skip_ws();
            if self.i >= self.s.len() {
                break;
            }
            if self.starts("<!--") {
                self.comment()?;
            } else if self.starts("<?") {
                self.pi()?;
            } else if self.starts("<!DOCTYPE") {
                if root.is_some() {
                    return None;
                }
                self.doctype()?;
            } else if self.peek() == Some(b'<') {
                if root.is_some() {
                    return None;
                }
                root = Some(self.element()?);
            } else {
                return None;
            }
        }
        root
    }

    fn comment(&mut self) -> Option<()> {
        self.bump(4);
        let rest = &self.text[self.i..];
        let end = rest.find("--")?;
        if !rest[end..].starts_with("-->") || !rest[..end].chars().all(is_xml_char) {
            return None;
        }
        self.bump(end + 3);
        Some(())
    }

    fn pi(&mut self) -> Option<()> {
        let at_start = self.i == 0;
        self.bump(2);
        let target = self.name()?;
        if target.eq_ignore_ascii_case("xml") && !at_start {
            return None;
        }
        let rest = &self.text[self.i..];
        let end = rest.find("?>")?;
        self.bump(end + 2);
        Some(())
    }

    fn doctype(&mut self) -> Option<()> {
        let mut depth = 0i32;
        while let Some(b) = self.peek() {
            self.bump(1);
            match b {
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'>' if depth <= 0 => return Some(()),
                _ => {}
            }
        }
        None
    }

    fn bound(&self, prefix: &str) -> bool {
        prefix == "xml"
            || prefix == "xmlns"
            || self.ns.iter().any(|s| s.iter().any(|p| p == prefix))
    }

    fn element(&mut self) -> Option<Element> {
        let line = self.line;
        self.bump(1);
        let name = self.name()?;
        let mut attrs: Vec<XmlAttr> = Vec::new();
        let mut declared = Vec::new();
        let empty;
        loop {
            let had_ws = self.skip_ws();
            match self.peek()? {
                b'/' => {
                    self.bump(1);
                    if self.peek()? != b'>' {
                        return None;
                    }
                    self.bump(1);
                    empty = true;
                    break;
                }
                b'>' => {
                    self.bump(1);
                    empty = false;
                    break;
                }
                _ => {
                    if !had_ws {
                        return None;
                    }
                    let aline = self.line;
                    let aname = self.name()?;
                    self.skip_ws();
                    if self.peek()? != b'=' {
                        return None;
                    }
                    self.bump(1);
                    self.skip_ws();
                    let value = self.attr_value()?;
                    if attrs.iter().any(|a| a.name == aname) {
                        return None;
                    }
                    if let Some(p) = aname.strip_prefix("xmlns:") {
                        declared.push(p.to_string());
                    }
                    attrs.push(XmlAttr {
                        name: aname,
                        value,
                        line: aline,
                    });
                }
            }
        }
        self.ns.push(declared);
        let prefixes_ok = std::iter::once(name.as_str())
            .chain(attrs.iter().map(|a| a.name.as_str()))
            .all(|n| match n.split_once(':') {
                Some((p, _)) => self.bound(p),
                None => true,
            });
        if !prefixes_ok {
            return None;
        }
        // Expanded attribute names must be unique too (`a:x` and `b:x` bound to one URI is rare;
        // only the literal duplicate check above is applied).
        let mut el = Element {
            name,
            attrs,
            children: Vec::new(),
            text: None,
            line,
        };
        if !empty {
            self.content(&mut el)?;
        }
        self.ns.pop();
        Some(el)
    }

    fn content(&mut self, el: &mut Element) -> Option<()> {
        let mut text = String::new();
        let mut before_child = true;
        loop {
            if self.i >= self.s.len() {
                return None;
            }
            if self.starts("</") {
                self.bump(2);
                let end = self.name()?;
                self.skip_ws();
                if self.peek()? != b'>' || end != el.name {
                    return None;
                }
                self.bump(1);
                break;
            } else if self.starts("<!--") {
                self.comment()?;
            } else if self.starts("<![CDATA[") {
                self.bump(9);
                let rest = &self.text[self.i..];
                let end = rest.find("]]>")?;
                if before_child {
                    text.push_str(&rest[..end]);
                }
                self.bump(end + 3);
            } else if self.starts("<?") {
                self.pi()?;
            } else if self.peek() == Some(b'<') {
                let child = self.element()?;
                el.children.push(child);
                before_child = false;
            } else {
                let rest = &self.text[self.i..];
                let end = rest.find(['<', '&']).unwrap_or(rest.len());
                let chunk = &rest[..end];
                if chunk.contains("]]>") || !chunk.chars().all(is_xml_char) {
                    return None;
                }
                if before_child {
                    text.push_str(&chunk.replace("\r\n", "\n").replace('\r', "\n"));
                }
                self.bump(end);
                if self.peek() == Some(b'&') {
                    let c = self.reference()?;
                    if before_child {
                        text.push_str(&c);
                    }
                }
            }
        }
        if !text.is_empty() {
            el.text = Some(text);
        }
        Some(())
    }

    fn attr_value(&mut self) -> Option<String> {
        let quote = self.peek()?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        self.bump(1);
        let mut out = String::new();
        loop {
            let rest = &self.text[self.i..];
            let end = rest.find([quote as char, '<', '&'])?;
            let chunk = &rest[..end];
            if !chunk.chars().all(is_xml_char) {
                return None;
            }
            out.extend(chunk.chars().map(|c| match c {
                '\t' | '\n' | '\r' => ' ',
                c => c,
            }));
            self.bump(end);
            match self.peek()? {
                b'<' => return None,
                b'&' => out.push_str(&self.reference()?),
                _ => {
                    self.bump(1);
                    return Some(out);
                }
            }
        }
    }

    /// `&name;` or `&#n;` / `&#xh;` at the cursor.
    fn reference(&mut self) -> Option<String> {
        let rest = &self.text[self.i..];
        let end = rest.find(';')?;
        let body = &rest[1..end];
        let out = match body {
            "lt" => "<".to_string(),
            "gt" => ">".to_string(),
            "amp" => "&".to_string(),
            "apos" => "'".to_string(),
            "quot" => "\"".to_string(),
            _ => {
                let code = if let Some(h) = body.strip_prefix("#x") {
                    u32::from_str_radix(h, 16).ok()?
                } else if let Some(d) = body.strip_prefix('#') {
                    d.parse().ok()?
                } else {
                    return None;
                };
                let c = char::from_u32(code)?;
                if !is_xml_char(c) {
                    return None;
                }
                c.to_string()
            }
        };
        self.bump(end + 1);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_elements_attributes_and_lines() {
        let doc = "\u{feff}<?xml version=\"1.0\"?>\n<!-- c -->\n<P a=\"1 &amp; 2\" xmlns:x=\"u\">\n  <x:C\n    x:N=\"v\"/>\n  t&#46;<D>z</D>\n</P>\n";
        let root = parse(doc.as_bytes()).unwrap();
        assert_eq!(root.name, "P");
        assert_eq!(root.line, 3);
        assert_eq!(root.attr("a"), Some("1 & 2"));
        assert_eq!(root.children[0].local(), "C");
        assert_eq!(root.children[0].prefix(), Some("x"));
        assert_eq!(root.children[0].attrs[0].line, 5);
        assert_eq!(root.descendants().len(), 3);
        assert_eq!(root.children[1].text.as_deref(), Some("z"));
    }

    #[test]
    fn rejects_what_a_conforming_parser_rejects() {
        for bad in [
            "",
            "<P>",
            "<P></Q>",
            "<P/><Q/>",
            " <?xml version=\"1.0\"?><P/>",
            "<P a=1/>",
            "<P a=\"1\" a=\"2\"/>",
            "<P a=\"<\"/>",
            "<P>&nbsp;</P>",
            "<P><!-- a -- b --></P>",
            "<x:P/>",
            "<P x:a=\"1\"/>",
            "<P>]]></P>",
        ] {
            assert!(parse(bad.as_bytes()).is_none(), "{bad:?}");
        }
        assert!(parse(b"<P xmlns:x=\"u\"><x:Q/></P>\n<!-- t -->\n").is_some());
    }
}
