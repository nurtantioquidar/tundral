//! `--push`: load a graph into Neo4j (Bolt) or FalkorDB (Redis protocol) with the records of
//! Graphify's push (`docs/graphify-exports.md` §10.3), batched with `UNWIND` instead of one
//! statement per node and edge. Nothing is sent anywhere but the URI the caller passes.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::net::TcpStream;

use anyhow::{bail, Context, Result};

use crate::bolt::{self, Bolt, Pack, Reply};
use crate::cypher::{quoted, Prop, Props, PushPlan};

/// Rows per statement for Neo4j.
const NEO4J_BATCH: usize = 5_000;
/// Approximate query text per statement for FalkorDB.
const FALKOR_BATCH_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub user: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PushReport {
    pub nodes: usize,
    pub edges: usize,
    /// Statements sent (index creation included).
    pub statements: usize,
}

fn node_query(label: &str) -> String {
    format!(
        "UNWIND $rows AS row MERGE (n:{} {{id: row.id}}) SET n += row.props",
        quoted(label)
    )
}

fn edge_query(source: &str, ty: &str, target: &str) -> String {
    format!(
        "UNWIND $rows AS row MATCH (a:{} {{id: row.src}}) MATCH (b:{} {{id: row.tgt}}) \
         MERGE (a)-[r:{}]->(b) SET r += row.props",
        quoted(source),
        quoted(target),
        quoted(ty)
    )
}

// ---------------------------------------------------------------------------------------------
// Neo4j

fn pack_props(p: &Props) -> Pack {
    Pack::Map(
        p.iter()
            .map(|(k, v)| {
                let v = match v {
                    Prop::Str(s) => Pack::Str(s.clone()),
                    Prop::Int(i) => Pack::Int(*i),
                    Prop::Float(f) => Pack::Float(*f),
                    Prop::Bool(b) => Pack::Bool(*b),
                };
                (k.clone(), v)
            })
            .collect(),
    )
}

fn neo4j_run(bolt: &mut Bolt, query: &str, rows: Vec<Pack>) -> Result<()> {
    match bolt.run(query, vec![("rows".into(), Pack::List(rows))])? {
        Reply::Failure { code, message } => bail!("Neo4j: {code}: {message}"),
        _ => Ok(()),
    }
}

pub fn neo4j(plan: &PushPlan, uri: &str, cred: &Credentials) -> Result<PushReport> {
    let user = cred.user.as_deref().unwrap_or("neo4j");
    let agent = concat!("dijkstral/", env!("CARGO_PKG_VERSION"));
    let mut bolt = Bolt::connect(uri, user, cred.password.as_deref(), agent)?;
    let mut report = PushReport {
        nodes: plan.node_count,
        edges: plan.edge_count,
        statements: 0,
    };
    // Index creation failures are ignored, as Graphify does.
    for label in &plan.labels {
        let q = format!(
            "CREATE INDEX IF NOT EXISTS FOR (n:{}) ON (n.id)",
            quoted(label)
        );
        bolt.run(&q, Vec::new())?;
        report.statements += 1;
    }
    if !plan.labels.is_empty() {
        bolt.run("CALL db.awaitIndexes(300)", Vec::new())?;
    }
    for (label, nodes) in &plan.nodes {
        let q = node_query(label);
        for chunk in nodes.chunks(NEO4J_BATCH) {
            let rows = chunk
                .iter()
                .map(|n| {
                    Pack::Map(vec![
                        ("id".into(), Pack::Str(n.id.clone())),
                        ("props".into(), pack_props(&n.props)),
                    ])
                })
                .collect();
            neo4j_run(&mut bolt, &q, rows)?;
            report.statements += 1;
        }
    }
    for ((s, ty, t), edges) in &plan.edges {
        let q = edge_query(s, ty, t);
        for chunk in edges.chunks(NEO4J_BATCH) {
            let rows = chunk
                .iter()
                .map(|e| {
                    Pack::Map(vec![
                        ("src".into(), Pack::Str(e.source.clone())),
                        ("tgt".into(), Pack::Str(e.target.clone())),
                        ("props".into(), pack_props(&e.props)),
                    ])
                })
                .collect();
            neo4j_run(&mut bolt, &q, rows)?;
            report.statements += 1;
        }
    }
    bolt.close();
    Ok(report)
}

// ---------------------------------------------------------------------------------------------
// FalkorDB

/// A FalkorDB (Redis) reply.
#[derive(Debug, PartialEq)]
pub enum Resp {
    Simple(String),
    Error(String),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Resp>>),
}

pub struct Redis {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

/// `falkordb://[user[:password]@]host[:port]`, `redis://…` or `host[:port]`; the default port
/// is 6379. Returns `(host, port, user, password)` from the URI.
pub fn parse_falkordb_uri(uri: &str) -> Result<(String, u16, Option<String>, Option<String>)> {
    let rest = match uri.split_once("://") {
        Some(("falkordb" | "redis", r)) => r,
        Some(("rediss" | "falkordbs", _)) => {
            bail!("TLS is not available in this build; use falkordb:// or redis://")
        }
        Some((s, _)) => bail!("unsupported scheme {s}:// (use falkordb:// or redis://)"),
        None => uri,
    };
    let rest = rest.split(['/', '?']).next().unwrap_or("");
    let (auth, host_port) = match rest.rsplit_once('@') {
        Some((a, h)) => (Some(a), h),
        None => (None, rest),
    };
    let (user, password) = match auth {
        None => (None, None),
        Some(a) => match a.split_once(':') {
            Some((u, p)) => ((!u.is_empty()).then(|| u.to_string()), Some(p.to_string())),
            None => (Some(a.to_string()), None),
        },
    };
    let (host, port) = bolt::host_port_of(host_port, 6379)?;
    Ok((host, port, user, password))
}

impl Redis {
    pub fn connect(host: &str, port: u16) -> Result<Redis> {
        let stream = bolt::connect_tcp(host, port)?;
        Ok(Redis {
            reader: BufReader::new(stream.try_clone()?),
            writer: BufWriter::new(stream),
        })
    }

    pub fn command(&mut self, args: &[&[u8]]) -> Result<Resp> {
        write!(self.writer, "*{}\r\n", args.len())?;
        for a in args {
            write!(self.writer, "${}\r\n", a.len())?;
            self.writer.write_all(a)?;
            self.writer.write_all(b"\r\n")?;
        }
        self.writer.flush()?;
        self.read()
    }

    fn line(&mut self) -> Result<String> {
        let mut line = Vec::new();
        self.reader.read_until(b'\n', &mut line)?;
        if !line.ends_with(b"\r\n") {
            bail!("the server closed the connection");
        }
        line.truncate(line.len() - 2);
        Ok(String::from_utf8_lossy(&line).into_owned())
    }

    fn read(&mut self) -> Result<Resp> {
        let line = self.line()?;
        let (kind, rest) = line.split_at(1.min(line.len()));
        Ok(match kind {
            "+" => Resp::Simple(rest.to_string()),
            "-" => Resp::Error(rest.to_string()),
            ":" => Resp::Int(rest.parse().context("bad integer reply")?),
            "$" => {
                let n: i64 = rest.parse().context("bad bulk length")?;
                if n < 0 {
                    Resp::Bulk(None)
                } else {
                    let mut b = vec![0u8; n as usize + 2];
                    self.reader.read_exact(&mut b)?;
                    b.truncate(n as usize);
                    Resp::Bulk(Some(b))
                }
            }
            "*" => {
                let n: i64 = rest.parse().context("bad array length")?;
                if n < 0 {
                    Resp::Array(None)
                } else {
                    let mut v = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        v.push(self.read()?);
                    }
                    Resp::Array(Some(v))
                }
            }
            _ => bail!("not a Redis/FalkorDB server (unexpected reply {line:?})"),
        })
    }
}

/// A Cypher string literal FalkorDB parses back to `s`: `"` and `\` escaped, control
/// characters as `\uXXXX`.
pub fn falkor_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A property map literal. FalkorDB cannot quote a key holding a backtick or NUL, so such keys
/// are skipped (Graphify's push fails on them).
fn falkor_map(p: &Props, out: &mut String) {
    out.push('{');
    let mut first = true;
    for (k, v) in p {
        if k.contains(['`', '\0']) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        let _ = write!(out, "`{k}`:");
        match v {
            Prop::Str(s) => falkor_str(s, out),
            Prop::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Prop::Float(f) => {
                let _ = write!(out, "{f:?}");
            }
            Prop::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        }
    }
    out.push('}');
}

fn falkor_query(redis: &mut Redis, graph: &str, query: &str) -> Result<Resp> {
    redis.command(&[
        b"GRAPH.QUERY",
        graph.as_bytes(),
        query.as_bytes(),
        b"--compact",
    ])
}

/// Send `rows` (each already a map literal) through `query` in batches of about
/// `FALKOR_BATCH_BYTES`.
fn falkor_batches(
    redis: &mut Redis,
    graph: &str,
    query: &str,
    rows: impl Iterator<Item = String>,
    report: &mut PushReport,
) -> Result<()> {
    let mut flush = |buf: &mut String, redis: &mut Redis| -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let text = format!("CYPHER rows=[{buf}] {query}");
        buf.clear();
        report.statements += 1;
        match falkor_query(redis, graph, &text)? {
            Resp::Error(e) => bail!("FalkorDB: {e}"),
            _ => Ok(()),
        }
    };
    let mut buf = String::new();
    for row in rows {
        if !buf.is_empty() {
            buf.push(',');
        }
        buf.push_str(&row);
        if buf.len() >= FALKOR_BATCH_BYTES {
            flush(&mut buf, redis)?;
        }
    }
    flush(&mut buf, redis)
}

pub fn falkordb(plan: &PushPlan, uri: &str, graph: &str, cred: &Credentials) -> Result<PushReport> {
    let (host, port, uri_user, uri_password) = parse_falkordb_uri(uri)?;
    let mut redis = Redis::connect(&host, port)?;
    let user = cred.user.clone().or(uri_user);
    if let Some(password) = cred.password.clone().or(uri_password) {
        let mut args: Vec<&[u8]> = vec![b"AUTH"];
        if let Some(u) = &user {
            args.push(u.as_bytes());
        }
        args.push(password.as_bytes());
        if let Resp::Error(e) = redis.command(&args)? {
            bail!("FalkorDB authentication failed: {e}");
        }
    }
    let mut report = PushReport {
        nodes: plan.node_count,
        edges: plan.edge_count,
        statements: 0,
    };
    // Index creation fails when the index exists; ignored, as Graphify does.
    for label in &plan.labels {
        let q = format!("CREATE INDEX FOR (n:{}) ON (n.id)", quoted(label));
        if let Resp::Error(e) = falkor_query(&mut redis, graph, &q)? {
            if e.contains("unknown command") {
                bail!("the server at {host}:{port} is not FalkorDB: {e}");
            }
        }
        report.statements += 1;
    }
    for (label, nodes) in &plan.nodes {
        let rows = nodes.iter().map(|n| {
            let mut s = String::from("{id:");
            falkor_str(&n.id, &mut s);
            s.push_str(",props:");
            falkor_map(&n.props, &mut s);
            s.push('}');
            s
        });
        falkor_batches(&mut redis, graph, &node_query(label), rows, &mut report)?;
    }
    for ((s, ty, t), edges) in &plan.edges {
        let rows = edges.iter().map(|e| {
            let mut s = String::from("{src:");
            falkor_str(&e.source, &mut s);
            s.push_str(",tgt:");
            falkor_str(&e.target, &mut s);
            s.push_str(",props:");
            falkor_map(&e.props, &mut s);
            s.push('}');
            s
        });
        falkor_batches(&mut redis, graph, &edge_query(s, ty, t), rows, &mut report)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falkordb_literals() {
        let mut s = String::new();
        falkor_str("a\"b\\c\n\0é`", &mut s);
        assert_eq!(s, "\"a\\\"b\\\\c\\u000a\\u0000é`\"");
        let mut m = String::new();
        falkor_map(
            &vec![
                ("k".into(), Prop::Int(-3)),
                ("bad`key".into(), Prop::Int(1)),
                ("f".into(), Prop::Float(2.0)),
                ("b".into(), Prop::Bool(true)),
            ],
            &mut m,
        );
        assert_eq!(m, "{`k`:-3,`f`:2.0,`b`:true}");
    }

    #[test]
    fn falkordb_uris() {
        assert_eq!(
            parse_falkordb_uri("falkordb://localhost").unwrap(),
            ("localhost".into(), 6379, None, None)
        );
        assert_eq!(
            parse_falkordb_uri("redis://u:p@h:1").unwrap(),
            ("h".into(), 1, Some("u".into()), Some("p".into()))
        );
        assert_eq!(
            parse_falkordb_uri("127.0.0.1:16379").unwrap(),
            ("127.0.0.1".into(), 16379, None, None)
        );
        assert!(parse_falkordb_uri("rediss://h").is_err());
    }
}
