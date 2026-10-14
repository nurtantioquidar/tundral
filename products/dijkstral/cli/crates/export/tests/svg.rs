//! `graph.svg` (`docs/graphify-exports.md`, SVG section). Graphify draws with matplotlib, so its
//! bytes and positions cannot be reproduced; `tests/fixtures/exports/<lang>/svg_facts.json` keeps
//! what is observable in its drawing of each clustered fixture — per node, in drawing order, the
//! colour, marker radius and label, and the solid/dashed link counts — and dijkstral must agree
//! on all of it. Layout determinism and the escaping of untrusted strings are checked here too.

use std::fs;
use std::path::{Path, PathBuf};

use dijkstral_export::nx::NxGraph;
use dijkstral_export::svg::{self, Layout, EDGE_CAP, LABEL_CAP};
use dijkstral_export::Names;
use serde_json::{json, Value};

const LANGS: [&str; 7] = [
    "python",
    "typescript",
    "javascript",
    "rust",
    "go",
    "java",
    "mixed",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn graph(lang: &str) -> NxGraph {
    let path = fixtures().join(format!("{lang}.clustered.graphify.json"));
    NxGraph::from_json(serde_json::from_slice(&fs::read(path).unwrap()).unwrap()).unwrap()
}

/// The value of attribute `name` in an element's opening tag.
fn attr<'a>(tag: &'a str, name: &str) -> &'a str {
    let key = format!(" {name}=\"");
    let start = tag.find(&key).unwrap_or_else(|| panic!("{name} in {tag}")) + key.len();
    &tag[start..start + tag[start..].find('"').unwrap()]
}

/// Opening tags of `element` inside the `<g id="group">` element.
fn tags<'a>(svg: &'a str, group: &str, element: &str) -> Vec<&'a str> {
    let open = format!("<g id=\"{group}\"");
    let Some(start) = svg.find(&open) else {
        return Vec::new();
    };
    let body = &svg[start..start + svg[start..].find("</g>").unwrap()];
    let pat = format!("<{element} ");
    body.match_indices(&pat)
        .map(|(i, _)| &body[i..i + body[i..].find('>').unwrap() + 1])
        .collect()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Each `<text>` in the labels group, its lines joined by `\n`, unescaped.
fn label_texts(svg: &str) -> Vec<String> {
    let start = svg.find("<g id=\"labels\"").unwrap();
    let body = &svg[start..start + svg[start..].find("</g>").unwrap()];
    body.split("<text ")
        .skip(1)
        .map(|t| {
            let inner = &t[t.find('>').unwrap() + 1..t.find("</text>").unwrap()];
            if inner.contains("<tspan") {
                inner
                    .split("<tspan ")
                    .skip(1)
                    .map(|s| unescape(&s[s.find('>').unwrap() + 1..s.find("</tspan>").unwrap()]))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                unescape(inner)
            }
        })
        .collect()
}

#[test]
fn drawing_agrees_with_graphify() {
    for lang in LANGS {
        let g = graph(lang);
        let (svg, stats) = svg::render(&g, &Names::Placeholder);
        let facts: Value = serde_json::from_slice(
            &fs::read(fixtures().join("exports").join(lang).join("svg_facts.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stats.layout, Layout::Force, "{lang}");
        assert!(svg.contains(&format!(
            "fill=\"{}\"",
            facts["background"].as_str().unwrap()
        )));

        let want = facts["nodes"].as_array().unwrap();
        let circles = tags(&svg, "nodes", "circle");
        let labels = label_texts(&svg);
        assert_eq!(circles.len(), want.len(), "{lang}: markers");
        assert_eq!(labels.len(), want.len(), "{lang}: labels");
        for (k, w) in want.iter().enumerate() {
            assert_eq!(
                attr(circles[k], "fill"),
                w["colour"],
                "{lang}: node {k} colour"
            );
            let r: f64 = attr(circles[k], "r").parse().unwrap();
            let wr = w["radius"].as_f64().unwrap();
            assert!(
                (r - wr).abs() < 0.011,
                "{lang}: node {k} radius {r} vs {wr}"
            );
            assert_eq!(
                labels[k],
                w["label"].as_str().unwrap(),
                "{lang}: node {k} label"
            );
        }

        let solid = tags(&svg, "links", "line").len();
        let dashed = tags(&svg, "uncertain-links", "line").len();
        assert_eq!(
            solid as u64,
            facts["links_solid"].as_u64().unwrap(),
            "{lang}"
        );
        assert_eq!(
            dashed as u64,
            facts["links_dashed"].as_u64().unwrap(),
            "{lang}"
        );

        // Graphify draws no legend without a labels file (quirk S2); dijkstral always does.
        assert_eq!(facts["legend"], false);
        assert!(svg.contains("<g id=\"legend\""), "{lang}: legend");

        // Every node lies inside the canvas.
        let (w, h) = (stats.width, stats.height);
        for c in &circles {
            let (x, y): (f64, f64) = (
                attr(c, "cx").parse().unwrap(),
                attr(c, "cy").parse().unwrap(),
            );
            assert!(
                x > 0.0 && x < w && y > 0.0 && y < h,
                "{lang}: {c} outside {w}×{h}"
            );
        }
    }
}

#[test]
fn layout_is_deterministic() {
    for lang in LANGS {
        let a = svg::render(&graph(lang), &Names::Placeholder).0;
        let b = svg::render(&graph(lang), &Names::Placeholder).0;
        assert_eq!(a, b, "{lang}");
    }
    let big = synthetic(3500, 3);
    let a = svg::render(&big, &Names::Placeholder).0;
    let b = svg::render(&synthetic(3500, 3), &Names::Placeholder).0;
    assert_eq!(a, b, "packed layout");
}

const HOSTILE: [&str; 9] = [
    "</text><script>alert(1)</script>",
    "]]> <![CDATA[ x ]]>",
    "& &amp; \"q\" 'q' `q`",
    "nul\u{0}bel\u{7}esc\u{1b}vt\u{b}ff\u{c}",
    "line\nbreak\r\ntab\there",
    "<!-- comment --> <?pi?>",
    "\u{FFFE}\u{FFFF}ünï 漢字 \u{2028}",
    "--> \" onload=\"alert(2)",
    "",
];

fn hostile() -> NxGraph {
    let nodes: Vec<Value> = HOSTILE
        .iter()
        .enumerate()
        .map(|(i, label)| {
            json!({"id": format!("n{i}\"<x>"), "label": label, "community": i % 4,
                   "source_file": format!("{label}.py")})
        })
        .collect();
    let links: Vec<Value> = (1..HOSTILE.len())
        .map(|i| {
            json!({"source": format!("n{}\"<x>", i - 1), "target": format!("n{i}\"<x>"),
                   "relation": HOSTILE[i], "confidence": HOSTILE[i]})
        })
        .collect();
    NxGraph::from_json(json!({"nodes": nodes, "links": links})).unwrap()
}

/// What an XML 1.0 parser would reject or read as markup.
fn assert_well_formed(svg: &str) {
    for c in svg.chars() {
        let ok =
            matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}');
        assert!(ok, "forbidden character U+{:04X}", c as u32);
    }
    assert!(!svg.contains("]]>"), "]]> in text");
    assert!(!svg.contains("<script"), "markup injected");
    assert!(!svg.contains("<!--"), "comment injected");
    assert!(!svg.contains("<?pi"), "processing instruction injected");
    // Tags balance: every element opened in the body is closed or self-closing.
    let body = &svg[svg.find("<svg ").unwrap()..];
    let (mut depth, mut i) = (0i64, 0);
    while let Some(off) = body[i..].find('<') {
        let at = i + off;
        let end = at + body[at..].find('>').unwrap();
        let tag = &body[at..=end];
        assert!(!tag[1..].contains('<'), "'<' inside a tag: {tag}");
        if tag.starts_with("</") {
            depth -= 1;
        } else if !tag.ends_with("/>") {
            depth += 1;
        }
        i = end + 1;
    }
    assert_eq!(depth, 0, "unbalanced elements");
}

#[test]
fn hostile_strings_stay_text() {
    let g = hostile();
    let mut names = std::collections::HashMap::new();
    names.insert(0, "</text><b>community</b> & ]]>".to_string());
    names.insert(1, "line\nbreak\u{0}".to_string());
    let (svg, stats) = svg::render(&g, &Names::Named(names));
    assert_well_formed(&svg);
    assert_eq!(stats.labels_drawn, HOSTILE.len());
    let labels = label_texts(&svg);
    for (i, label) in HOSTILE.iter().enumerate() {
        // Characters XML cannot carry are dropped, a tab becomes a space; lines are kept.
        let want: String = label
            .chars()
            .filter_map(|c| match c {
                '\t' => Some(' '),
                '\n' => Some('\n'),
                c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => None,
                c => Some(c),
            })
            .collect();
        // A label of "" draws nothing visible but keeps its slot.
        assert_eq!(labels[i], want, "label {i}");
    }
    assert!(svg.contains("&lt;/text&gt;&lt;b&gt;community&lt;/b&gt; &amp; ]]&gt; (3)"));
    assert!(svg.contains("line break (2)"));
}

/// `n` nodes in `n / 50` communities, each linked to the next `k` nodes.
fn synthetic(n: usize, k: usize) -> NxGraph {
    let nodes: Vec<Value> = (0..n)
        .map(|i| json!({"id": format!("n{i}"), "label": format!("node {i}"), "community": i / 50}))
        .collect();
    let mut links = Vec::new();
    for i in 0..n {
        for d in 1..=k {
            links.push(
                json!({"source": format!("n{i}"), "target": format!("n{}", (i + d * d) % n),
                              "confidence": if d == 1 { "EXTRACTED" } else { "INFERRED" }}),
            );
        }
    }
    NxGraph::from_json(json!({"nodes": nodes, "links": links})).unwrap()
}

#[test]
fn large_graphs_are_bounded() {
    let g = synthetic(3500, 31);
    assert!(g.edges.len() > EDGE_CAP);
    let (svg, stats) = svg::render(&g, &Names::Placeholder);
    assert_eq!(stats.layout, Layout::Packed);
    assert_eq!(stats.edges_drawn, EDGE_CAP);
    assert_eq!(stats.labels_drawn, LABEL_CAP);
    assert_eq!(tags(&svg, "nodes", "circle").len(), 3500);
    let lines = tags(&svg, "links", "line").len() + tags(&svg, "uncertain-links", "line").len();
    assert_eq!(lines, EDGE_CAP);
    assert_eq!(label_texts(&svg).len(), LABEL_CAP);
    // 70 communities: the legend lists the 40 largest and counts the rest.
    assert!(svg.contains("… and 30 more communities"));
    for c in tags(&svg, "nodes", "circle") {
        let (x, y): (f64, f64) = (
            attr(c, "cx").parse().unwrap(),
            attr(c, "cy").parse().unwrap(),
        );
        assert!(x > 0.0 && x < stats.width && y > 0.0 && y < stats.height);
    }
}

#[test]
fn empty_and_tiny_graphs() {
    for doc in [
        json!({"nodes": [], "links": []}),
        json!({"nodes": [{"id": "a"}], "links": []}),
        json!({"nodes": [{"id": "a"}, {"id": "b"}], "links": [{"source": "a", "target": "a"}]}),
    ] {
        let g = NxGraph::from_json(doc).unwrap();
        let (svg, stats) = svg::render(&g, &Names::Placeholder);
        assert_well_formed(&svg);
        assert_eq!(stats.edges_drawn, g.edges.len());
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
}
