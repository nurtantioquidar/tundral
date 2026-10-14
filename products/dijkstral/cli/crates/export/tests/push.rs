//! `--push` against in-process fake servers (`docs/graphify-exports.md` §10.3): the Bolt and
//! Redis framing, authentication, the statements sent and the rows they carry. Real Neo4j 5.26
//! and FalkorDB servers were checked by hand with `target/s4b/pushcmp.py`-style comparisons
//! (§10.5); these tests keep the protocol code honest offline.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use dijkstral_export::bolt::{decode, encode, Pack};
use dijkstral_export::cypher::push_plan;
use dijkstral_export::nx::NxGraph;
use dijkstral_export::push::{self, Credentials};
use serde_json::json;

fn graph() -> NxGraph {
    NxGraph::from_json(json!({
        "directed": false, "multigraph": false, "graph": {},
        "nodes": [
            {"id": "a", "label": "A'\n", "file_type": "code", "community": 0, "_origin": "ast",
             "list": [1, 2], "nul": null},
            {"id": "b", "label": "B", "file_type": "concept", "weight": 1.5, "flag": true},
            {"id": "c`x", "label": "C"},
        ],
        "links": [
            {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED",
             "_origin": "ast", "weight": 1.0},
            {"source": "c`x", "target": "a", "relation": "has-part"},
        ],
    }))
    .unwrap()
}

fn read_message(r: &mut impl Read) -> Option<Pack> {
    let mut body = Vec::new();
    loop {
        let mut n = [0u8; 2];
        r.read_exact(&mut n).ok()?;
        let n = u16::from_be_bytes(n) as usize;
        if n == 0 {
            return Some(decode(&body).unwrap());
        }
        let start = body.len();
        body.resize(start + n, 0);
        r.read_exact(&mut body[start..]).ok()?;
    }
}

fn write_message(w: &mut impl Write, v: &Pack) {
    let mut b = Vec::new();
    encode(v, &mut b);
    w.write_all(&(b.len() as u16).to_be_bytes()).unwrap();
    w.write_all(&b).unwrap();
    w.write_all(&[0, 0]).unwrap();
}

/// A Bolt 5.4 server that answers SUCCESS to everything except index creation (FAILURE, which
/// the client must reset and ignore) and records every message.
fn fake_neo4j(listener: TcpListener) -> Vec<Pack> {
    let (mut s, _) = listener.accept().unwrap();
    let mut hs = [0u8; 20];
    s.read_exact(&mut hs).unwrap();
    assert_eq!(&hs[..4], &[0x60, 0x60, 0xB0, 0x17]);
    s.write_all(&[0, 0, 4, 5]).unwrap();
    let mut seen = Vec::new();
    let mut failing = false;
    while let Some(m) = read_message(&mut s) {
        let Pack::Struct(tag, ref fields) = m else {
            panic!("not a struct")
        };
        let reply = match tag {
            0x02 => break,
            0x10 => {
                let q = fields[0].as_str().unwrap();
                failing = q.starts_with("CREATE INDEX");
                if failing {
                    Pack::Struct(
                        0x7F,
                        vec![Pack::Map(vec![
                            ("code".into(), Pack::str("Neo.ClientError.Test")),
                            ("message".into(), Pack::str("index exists")),
                        ])],
                    )
                } else {
                    Pack::Struct(0x70, vec![Pack::Map(vec![])])
                }
            }
            0x3F if failing => Pack::Struct(0x7E, vec![]),
            0x0F => {
                failing = false;
                Pack::Struct(0x70, vec![Pack::Map(vec![])])
            }
            _ => Pack::Struct(0x70, vec![Pack::Map(vec![])]),
        };
        seen.push(m);
        write_message(&mut s, &reply);
    }
    seen
}

#[test]
fn neo4j_push_speaks_bolt() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || fake_neo4j(listener));
    let plan = push_plan(&graph());
    let cred = Credentials {
        user: None,
        password: Some("secret".into()),
    };
    let report = push::neo4j(&plan, &format!("bolt://127.0.0.1:{port}"), &cred).unwrap();
    assert_eq!((report.nodes, report.edges), (3, 2));
    let seen = server.join().unwrap();

    // HELLO without credentials, then LOGON with them (Bolt 5.1+).
    let Pack::Struct(0x01, hello) = &seen[0] else {
        panic!("HELLO first")
    };
    assert!(hello[0].get("credentials").is_none());
    assert!(hello[0].get("bolt_agent").is_some());
    let Pack::Struct(0x6A, logon) = &seen[1] else {
        panic!("LOGON second")
    };
    assert_eq!(logon[0].get("principal"), Some(&Pack::str("neo4j")));
    assert_eq!(logon[0].get("credentials"), Some(&Pack::str("secret")));

    let runs: Vec<(&str, &Pack)> = seen
        .iter()
        .filter_map(|m| match m {
            Pack::Struct(0x10, f) => Some((f[0].as_str().unwrap(), &f[1])),
            _ => None,
        })
        .collect();
    let queries: Vec<&str> = runs.iter().map(|(q, _)| *q).collect();
    assert_eq!(
        queries,
        [
            "CREATE INDEX IF NOT EXISTS FOR (n:`Code`) ON (n.id)",
            "CREATE INDEX IF NOT EXISTS FOR (n:`Concept`) ON (n.id)",
            "CREATE INDEX IF NOT EXISTS FOR (n:`Entity`) ON (n.id)",
            "CALL db.awaitIndexes(300)",
            "UNWIND $rows AS row MERGE (n:`Code` {id: row.id}) SET n += row.props",
            "UNWIND $rows AS row MERGE (n:`Concept` {id: row.id}) SET n += row.props",
            "UNWIND $rows AS row MERGE (n:`Entity` {id: row.id}) SET n += row.props",
            "UNWIND $rows AS row MATCH (a:`Code` {id: row.src}) MATCH (b:`Concept` {id: row.tgt}) MERGE (a)-[r:`CALLS`]->(b) SET r += row.props",
            "UNWIND $rows AS row MATCH (a:`Entity` {id: row.src}) MATCH (b:`Code` {id: row.tgt}) MERGE (a)-[r:`HAS_PART`]->(b) SET r += row.props",
        ]
    );
    // Node a: strings, ints, floats and booleans kept in file order, `_`-fields, lists and nulls
    // dropped, `id` last; the label travels unescaped as a parameter.
    let rows = runs[4].1.get("rows").unwrap();
    let Pack::List(rows) = rows else { panic!() };
    assert_eq!(
        rows[0],
        Pack::Map(vec![
            ("id".into(), Pack::str("a")),
            (
                "props".into(),
                Pack::Map(vec![
                    ("label".into(), Pack::str("A'\n")),
                    ("file_type".into(), Pack::str("code")),
                    ("community".into(), Pack::Int(0)),
                    ("id".into(), Pack::str("a")),
                ])
            ),
        ])
    );
    let Pack::List(rows) = runs[7].1.get("rows").unwrap() else {
        panic!()
    };
    assert_eq!(
        rows[0].get("props"),
        Some(&Pack::Map(vec![
            ("relation".into(), Pack::str("calls")),
            ("confidence".into(), Pack::str("EXTRACTED")),
            ("weight".into(), Pack::Float(1.0)),
        ]))
    );
    // A failed index creation is followed by RESET.
    assert!(seen.iter().any(|m| matches!(m, Pack::Struct(0x0F, _))));
}

#[test]
fn neo4j_push_refuses_tls_and_reports_auth_failures() {
    let plan = push_plan(&graph());
    let cred = Credentials {
        user: None,
        password: Some("x".into()),
    };
    let err = push::neo4j(&plan, "neo4j+s://127.0.0.1:1", &cred).unwrap_err();
    assert!(format!("{err:#}").contains("not available in this build"));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut hs = [0u8; 20];
        s.read_exact(&mut hs).unwrap();
        s.write_all(&[0, 0, 4, 5]).unwrap();
        read_message(&mut s).unwrap(); // HELLO
        write_message(&mut s, &Pack::Struct(0x70, vec![Pack::Map(vec![])]));
        read_message(&mut s).unwrap(); // LOGON
        write_message(
            &mut s,
            &Pack::Struct(
                0x7F,
                vec![Pack::Map(vec![
                    (
                        "code".into(),
                        Pack::str("Neo.ClientError.Security.Unauthorized"),
                    ),
                    ("message".into(), Pack::str("bad credentials")),
                ])],
            ),
        );
    });
    let err = push::neo4j(&plan, &format!("bolt://127.0.0.1:{port}"), &cred).unwrap_err();
    server.join().unwrap();
    assert!(format!("{err:#}").contains("Unauthorized"), "{err:#}");
}

fn read_command(r: &mut impl BufRead) -> Option<Vec<Vec<u8>>> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let n: usize = line.trim_end().strip_prefix('*')?.parse().ok()?;
    let mut args = Vec::new();
    for _ in 0..n {
        line.clear();
        r.read_line(&mut line).ok()?;
        let len: usize = line.trim_end().strip_prefix('$')?.parse().ok()?;
        let mut b = vec![0u8; len + 2];
        r.read_exact(&mut b).ok()?;
        b.truncate(len);
        args.push(b);
    }
    Some(args)
}

#[test]
fn falkordb_push_speaks_resp() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (s, _) = listener.accept().unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut w: TcpStream = s;
        let mut seen = Vec::new();
        while let Some(cmd) = read_command(&mut r) {
            let reply: &[u8] = if cmd[0] == b"GRAPH.QUERY" && cmd[2].starts_with(b"CREATE INDEX") {
                b"-Attribute 'id' is already indexed\r\n"
            } else if cmd[0] == b"AUTH" {
                b"+OK\r\n"
            } else {
                b"*1\r\n*1\r\n$16\r\nNodes created: 1\r\n"
            };
            w.write_all(reply).unwrap();
            seen.push(cmd);
        }
        seen
    });
    let plan = push_plan(&graph());
    let cred = Credentials {
        user: Some("u".into()),
        password: Some("p".into()),
    };
    let report =
        push::falkordb(&plan, &format!("falkordb://127.0.0.1:{port}"), "g", &cred).unwrap();
    assert_eq!((report.nodes, report.edges), (3, 2));
    let seen = server.join().unwrap();
    let text: Vec<Vec<String>> = seen
        .iter()
        .map(|c| {
            c.iter()
                .map(|a| String::from_utf8(a.clone()).unwrap())
                .collect()
        })
        .collect();
    assert_eq!(text[0], ["AUTH", "u", "p"]);
    assert_eq!(
        text[1],
        [
            "GRAPH.QUERY",
            "g",
            "CREATE INDEX FOR (n:`Code`) ON (n.id)",
            "--compact"
        ]
    );
    assert_eq!(
        text[4][2],
        "CYPHER rows=[{id:\"a\",props:{`label`:\"A'\\u000a\",`file_type`:\"code\",`community`:0,`id`:\"a\"}}] \
         UNWIND $rows AS row MERGE (n:`Code` {id: row.id}) SET n += row.props"
    );
    // The id holding a backtick is a value, so it travels; only property keys are restricted.
    assert!(text[6][2].starts_with("CYPHER rows=[{id:\"c`x\",props:{`label`:\"C\",`id`:\"c`x\"}}]"));
    assert_eq!(
        text[8][2],
        "CYPHER rows=[{src:\"c`x\",tgt:\"a\",props:{`relation`:\"has-part\"}}] UNWIND $rows AS row \
         MATCH (a:`Entity` {id: row.src}) MATCH (b:`Code` {id: row.tgt}) \
         MERGE (a)-[r:`HAS_PART`]->(b) SET r += row.props"
    );
    assert_eq!(text.len(), 9);
}
