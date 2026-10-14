//! A minimal Neo4j Bolt client over plain TCP: version negotiation (5.0–5.8, 4.4), basic
//! authentication, auto-commit `RUN` + `PULL`, `RESET` and `GOODBYE`, with PackStream encoding
//! of the values the push sends. TLS (`bolt+s`, `neo4j+s`) is not available in this build.

use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// A PackStream value.
#[derive(Debug, Clone, PartialEq)]
pub enum Pack {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Pack>),
    Map(Vec<(String, Pack)>),
    Struct(u8, Vec<Pack>),
}

impl Pack {
    pub fn str(s: &str) -> Pack {
        Pack::Str(s.to_string())
    }

    pub fn get(&self, key: &str) -> Option<&Pack> {
        match self {
            Pack::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Pack::Str(s) => Some(s),
            _ => None,
        }
    }
}

fn size_marker(out: &mut Vec<u8>, n: usize, tiny: u8, m8: u8, m16: u8, m32: u8) {
    if n < 16 && tiny != 0 {
        out.push(tiny | n as u8);
    } else if n <= u8::MAX as usize && m8 != 0 {
        out.push(m8);
        out.push(n as u8);
    } else if n <= u16::MAX as usize {
        out.push(m16);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(m32);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    }
}

pub fn encode_str(s: &str, out: &mut Vec<u8>) {
    size_marker(out, s.len(), 0x80, 0xD0, 0xD1, 0xD2);
    out.extend_from_slice(s.as_bytes());
}

pub fn encode(v: &Pack, out: &mut Vec<u8>) {
    match v {
        Pack::Null => out.push(0xC0),
        Pack::Bool(false) => out.push(0xC2),
        Pack::Bool(true) => out.push(0xC3),
        Pack::Int(i) => {
            let i = *i;
            if (-16..=127).contains(&i) {
                out.push(i as i8 as u8);
            } else if i8::try_from(i).is_ok() {
                out.push(0xC8);
                out.push(i as i8 as u8);
            } else if let Ok(x) = i16::try_from(i) {
                out.push(0xC9);
                out.extend_from_slice(&x.to_be_bytes());
            } else if let Ok(x) = i32::try_from(i) {
                out.push(0xCA);
                out.extend_from_slice(&x.to_be_bytes());
            } else {
                out.push(0xCB);
                out.extend_from_slice(&i.to_be_bytes());
            }
        }
        Pack::Float(f) => {
            out.push(0xC1);
            out.extend_from_slice(&f.to_bits().to_be_bytes());
        }
        Pack::Str(s) => encode_str(s, out),
        Pack::List(l) => {
            size_marker(out, l.len(), 0x90, 0xD4, 0xD5, 0xD6);
            for x in l {
                encode(x, out);
            }
        }
        Pack::Map(m) => {
            size_marker(out, m.len(), 0xA0, 0xD8, 0xD9, 0xDA);
            for (k, x) in m {
                encode_str(k, out);
                encode(x, out);
            }
        }
        Pack::Struct(tag, fields) => {
            out.push(0xB0 | fields.len() as u8);
            out.push(*tag);
            for x in fields {
                encode(x, out);
            }
        }
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.i + n > self.b.len() {
            bail!("truncated Bolt message");
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn len(&mut self, bytes: usize) -> Result<usize> {
        let s = self.take(bytes)?;
        Ok(s.iter().fold(0usize, |a, &b| (a << 8) | b as usize))
    }
}

fn decode_at(c: &mut Cursor) -> Result<Pack> {
    let m = c.u8()?;
    Ok(match m {
        0x00..=0x7F => Pack::Int(m as i64),
        0xF0..=0xFF => Pack::Int(m as i8 as i64),
        0x80..=0x8F => str_of(c, (m & 0x0F) as usize)?,
        0x90..=0x9F => list_of(c, (m & 0x0F) as usize)?,
        0xA0..=0xAF => map_of(c, (m & 0x0F) as usize)?,
        0xB0..=0xBF => {
            let tag = c.u8()?;
            let mut fields = Vec::new();
            for _ in 0..(m & 0x0F) {
                fields.push(decode_at(c)?);
            }
            Pack::Struct(tag, fields)
        }
        0xC0 => Pack::Null,
        0xC1 => Pack::Float(f64::from_bits(u64::from_be_bytes(
            c.take(8)?.try_into().expect("8 bytes"),
        ))),
        0xC2 => Pack::Bool(false),
        0xC3 => Pack::Bool(true),
        0xC8 => Pack::Int(c.u8()? as i8 as i64),
        0xC9 => Pack::Int(i16::from_be_bytes(c.take(2)?.try_into().expect("2")) as i64),
        0xCA => Pack::Int(i32::from_be_bytes(c.take(4)?.try_into().expect("4")) as i64),
        0xCB => Pack::Int(i64::from_be_bytes(c.take(8)?.try_into().expect("8"))),
        0xCC..=0xCE => {
            // Bytes: skipped.
            let n = c.len(1 << (m - 0xCC))?;
            c.take(n)?;
            Pack::Null
        }
        0xD0..=0xD2 => {
            let n = c.len(1 << (m - 0xD0))?;
            str_of(c, n)?
        }
        0xD4..=0xD6 => {
            let n = c.len(1 << (m - 0xD4))?;
            list_of(c, n)?
        }
        0xD8..=0xDA => {
            let n = c.len(1 << (m - 0xD8))?;
            map_of(c, n)?
        }
        _ => bail!("unknown PackStream marker {m:#04x}"),
    })
}

fn str_of(c: &mut Cursor, n: usize) -> Result<Pack> {
    Ok(Pack::Str(String::from_utf8_lossy(c.take(n)?).into_owned()))
}

fn list_of(c: &mut Cursor, n: usize) -> Result<Pack> {
    let mut l = Vec::new();
    for _ in 0..n {
        l.push(decode_at(c)?);
    }
    Ok(Pack::List(l))
}

fn map_of(c: &mut Cursor, n: usize) -> Result<Pack> {
    let mut m = Vec::new();
    for _ in 0..n {
        let k = match decode_at(c)? {
            Pack::Str(s) => s,
            other => format!("{other:?}"),
        };
        m.push((k, decode_at(c)?));
    }
    Ok(Pack::Map(m))
}

pub fn decode(b: &[u8]) -> Result<Pack> {
    decode_at(&mut Cursor { b, i: 0 })
}

const HELLO: u8 = 0x01;
const GOODBYE: u8 = 0x02;
const RESET: u8 = 0x0F;
const RUN: u8 = 0x10;
const PULL: u8 = 0x3F;
const LOGON: u8 = 0x6A;
const SUCCESS: u8 = 0x70;
const RECORD: u8 = 0x71;
const IGNORED: u8 = 0x7E;
const FAILURE: u8 = 0x7F;

/// A server's answer to one request.
#[derive(Debug)]
pub enum Reply {
    Success(Pack),
    Failure { code: String, message: String },
    Ignored,
}

pub struct Bolt {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
    /// Negotiated `(major, minor)`.
    pub version: (u8, u8),
}

/// `bolt://host:port` or `neo4j://host:port` (connected to directly: no routing table).
pub fn parse_uri(uri: &str) -> Result<(String, u16)> {
    let (scheme, rest) = uri
        .split_once("://")
        .with_context(|| format!("not a Bolt URI: {uri}"))?;
    match scheme {
        "bolt" | "neo4j" => {}
        "bolt+s" | "bolt+ssc" | "neo4j+s" | "neo4j+ssc" => {
            bail!("TLS ({scheme}://) is not available in this build; use bolt:// to a server or proxy without TLS")
        }
        _ => bail!("unsupported scheme {scheme}:// (use bolt:// or neo4j://)"),
    }
    let host_port = rest.split(['/', '?']).next().unwrap_or("");
    let host_port = host_port.rsplit('@').next().unwrap_or("");
    host_port_of(host_port, 7687)
}

/// `host[:port]`, with `[v6]` brackets.
pub fn host_port_of(s: &str, default: u16) -> Result<(String, u16)> {
    if s.is_empty() {
        bail!("no host in URI");
    }
    if let Some(rest) = s.strip_prefix('[') {
        let (host, after) = rest.split_once(']').context("unclosed [ in host")?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().context("bad port")?,
            None => default,
        };
        return Ok((host.to_string(), port));
    }
    match s.rsplit_once(':') {
        Some((h, p)) => Ok((h.to_string(), p.parse().context("bad port")?)),
        None => Ok((s.to_string(), default)),
    }
}

pub fn connect_tcp(host: &str, port: u16) -> Result<TcpStream> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolving {host}:{port}"))?
        .collect();
    let mut last = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(10)) {
            Ok(s) => {
                s.set_nodelay(true).ok();
                return Ok(s);
            }
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) => Err(e).with_context(|| format!("connecting to {host}:{port}")),
        None => bail!("{host}:{port} resolved to no address"),
    }
}

impl Bolt {
    /// Connect, negotiate a version and authenticate (`basic` with a password, else `none`).
    pub fn connect(uri: &str, user: &str, password: Option<&str>, agent: &str) -> Result<Bolt> {
        let (host, port) = parse_uri(uri)?;
        let stream = connect_tcp(&host, port)?;
        let mut bolt = Bolt {
            reader: BufReader::new(stream.try_clone()?),
            writer: BufWriter::new(stream),
            version: (0, 0),
        };
        // Magic, then 5.8 down to 5.0, 4.4 down to 4.2, and two empty slots.
        bolt.writer.write_all(&[
            0x60, 0x60, 0xB0, 0x17, 0, 8, 8, 5, 0, 2, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0,
        ])?;
        bolt.writer.flush()?;
        let mut v = [0u8; 4];
        bolt.reader
            .read_exact(&mut v)
            .context("reading the Bolt handshake (is this a Neo4j Bolt port?)")?;
        if v == *b"HTTP" {
            bail!("the server answered HTTP: use the Bolt port (7687), not the browser port");
        }
        if !matches!(v[3], 4 | 5) {
            bail!("the server supports none of Bolt 5.0–5.8 or 4.2–4.4");
        }
        bolt.version = (v[3], v[2]);
        let auth = |m: &mut Vec<(String, Pack)>| match password {
            Some(p) => {
                m.push(("scheme".into(), Pack::str("basic")));
                m.push(("principal".into(), Pack::str(user)));
                m.push(("credentials".into(), Pack::str(p)));
            }
            None => m.push(("scheme".into(), Pack::str("none"))),
        };
        let mut hello = vec![("user_agent".to_string(), Pack::str(agent))];
        if bolt.version >= (5, 3) {
            hello.push((
                "bolt_agent".into(),
                Pack::Map(vec![("product".into(), Pack::str(agent))]),
            ));
        }
        let logon = bolt.version >= (5, 1);
        if !logon {
            auth(&mut hello);
        }
        bolt.send(HELLO, vec![Pack::Map(hello)])?;
        if let Reply::Failure { code, message } = bolt.reply()? {
            bail!("Neo4j refused the connection: {code}: {message}");
        }
        if logon {
            let mut m = Vec::new();
            auth(&mut m);
            bolt.send(LOGON, vec![Pack::Map(m)])?;
            if let Reply::Failure { code, message } = bolt.reply()? {
                bail!("Neo4j authentication failed: {code}: {message}");
            }
        }
        Ok(bolt)
    }

    fn send(&mut self, tag: u8, fields: Vec<Pack>) -> Result<()> {
        let mut body = Vec::new();
        encode(&Pack::Struct(tag, fields), &mut body);
        for chunk in body.chunks(u16::MAX as usize) {
            self.writer.write_all(&(chunk.len() as u16).to_be_bytes())?;
            self.writer.write_all(chunk)?;
        }
        self.writer.write_all(&[0, 0])?;
        self.writer.flush()?;
        Ok(())
    }

    fn message(&mut self) -> Result<Pack> {
        let mut body = Vec::new();
        loop {
            let mut n = [0u8; 2];
            self.reader
                .read_exact(&mut n)
                .context("the server closed the connection")?;
            let n = u16::from_be_bytes(n) as usize;
            if n == 0 {
                if body.is_empty() {
                    continue; // a no-op keep-alive chunk
                }
                return decode(&body);
            }
            let start = body.len();
            body.resize(start + n, 0);
            self.reader.read_exact(&mut body[start..])?;
        }
    }

    /// The summary answering one request; records are skipped.
    fn reply(&mut self) -> Result<Reply> {
        loop {
            match self.message()? {
                Pack::Struct(SUCCESS, mut f) => {
                    return Ok(Reply::Success(f.pop().unwrap_or(Pack::Null)))
                }
                Pack::Struct(RECORD, _) => continue,
                Pack::Struct(IGNORED, _) => return Ok(Reply::Ignored),
                Pack::Struct(FAILURE, f) => {
                    let meta = f.first().cloned().unwrap_or(Pack::Null);
                    let s = |k: &str| meta.get(k).and_then(Pack::as_str).unwrap_or("").to_string();
                    let code = match s("neo4j_code") {
                        c if c.is_empty() => s("code"),
                        c => c,
                    };
                    return Ok(Reply::Failure {
                        code,
                        message: s("message"),
                    });
                }
                other => bail!("unexpected Bolt message {other:?}"),
            }
        }
    }

    /// Run one auto-commit statement and pull its result. A failure is returned (not an error)
    /// after resetting the connection so the next statement can run.
    pub fn run(&mut self, query: &str, params: Vec<(String, Pack)>) -> Result<Reply> {
        let pull_n = vec![("n".to_string(), Pack::Int(-1))];
        self.send(
            RUN,
            vec![Pack::str(query), Pack::Map(params), Pack::Map(Vec::new())],
        )?;
        self.send(PULL, vec![Pack::Map(pull_n)])?;
        let run = self.reply()?;
        let pull = self.reply()?;
        let failed = match (&run, &pull) {
            (Reply::Failure { .. }, _) => Some(run),
            (_, Reply::Failure { .. }) => Some(pull),
            _ => None,
        };
        match failed {
            Some(f) => {
                self.send(RESET, Vec::new())?;
                self.reply()?;
                Ok(f)
            }
            None => Ok(Reply::Success(Pack::Null)),
        }
    }

    pub fn close(mut self) {
        let _ = self.send(GOODBYE, Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packstream_round_trips() {
        let v = Pack::Map(vec![
            ("a".into(), Pack::Int(-17)),
            ("b".into(), Pack::Int(200)),
            ("c".into(), Pack::Int(-40000)),
            ("d".into(), Pack::Int(1 << 40)),
            ("e".into(), Pack::Float(1.5)),
            ("f".into(), Pack::str(&"x".repeat(300))),
            ("g".into(), Pack::List((0..20).map(Pack::Int).collect())),
            ("h".into(), Pack::Bool(true)),
            ("i".into(), Pack::Null),
            ("j".into(), Pack::str("漢字\u{0}")),
        ]);
        let mut b = Vec::new();
        encode(&v, &mut b);
        assert_eq!(decode(&b).unwrap(), v);
        let mut t = Vec::new();
        encode(&Pack::Int(-16), &mut t);
        encode(&Pack::Int(127), &mut t);
        encode(&Pack::Int(-17), &mut t);
        assert_eq!(t, [0xF0, 0x7F, 0xC8, 0xEF]);
    }

    #[test]
    fn uris() {
        assert_eq!(
            parse_uri("bolt://localhost").unwrap(),
            ("localhost".into(), 7687)
        );
        assert_eq!(
            parse_uri("neo4j://u@db.example:7688/x").unwrap(),
            ("db.example".into(), 7688)
        );
        assert_eq!(parse_uri("bolt://[::1]:9").unwrap(), ("::1".into(), 9));
        assert!(parse_uri("neo4j+s://x").is_err());
        assert!(parse_uri("http://x").is_err());
    }
}
