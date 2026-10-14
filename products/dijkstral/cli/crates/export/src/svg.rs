//! `graph.svg`: a static drawing of the graph (`docs/graphify-exports.md`, SVG section).
//!
//! Graphify draws with matplotlib, so its bytes cannot be reproduced; dijkstral keeps what is
//! observable — the dark canvas, grey translucent links, one circle per node coloured by
//! community (Tableau 10) and sized `300 + 1200·degree/max` pt², every label in white 7 pt
//! centred on its node, and a community legend in the top-left corner — and draws it with its
//! own deterministic layout:
//!
//! - up to [`FORCE_LIMIT`] nodes, a seeded Fruchterman–Reingold force layout (fixed seed, fixed
//!   iteration count, fixed summation order, so identical input gives identical bytes);
//! - above it, communities packed on a sunflower spiral (largest at the centre) with members on
//!   a smaller spiral inside each disc, best connected first — linear time at any size.
//!
//! Large graphs are bounded: at most [`EDGE_CAP`] links (those between the best-connected
//! endpoints) and, above the force limit, the [`LABEL_CAP`] best-connected labels are drawn.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::html::PALETTE;
use crate::nx::NxGraph;
use crate::view::Names;

pub const SVG_FILE: &str = "graph.svg";
/// Graphs with at most this many nodes get the force layout.
pub const FORCE_LIMIT: usize = 3000;
/// At most this many links are drawn.
pub const EDGE_CAP: usize = 100_000;
/// Above [`FORCE_LIMIT`], only this many labels (the best-connected nodes) are drawn.
pub const LABEL_CAP: usize = 2000;
/// At most this many communities are listed in the legend (the largest).
pub const LEGEND_CAP: usize = 40;

/// Graphify's figure: 20 × 14 in at 72 pt/in.
const WIDTH: f64 = 1440.0;
const HEIGHT: f64 = 1008.0;
const BACKGROUND: &str = "#1a1a2e";
const SEED: u64 = 42;
const FORCE_ITERATIONS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Force,
    Packed,
}

#[derive(Debug, Clone)]
pub struct SvgStats {
    pub nodes: usize,
    pub edges: usize,
    pub edges_drawn: usize,
    pub labels_drawn: usize,
    pub layout: Layout,
    pub width: f64,
    pub height: f64,
}

/// Write `graph.svg`.
pub fn write(g: &NxGraph, names: &Names, path: &Path) -> Result<SvgStats> {
    let (svg, stats) = render(g, names);
    fs::write(path, svg).with_context(|| format!("writing {}", path.display()))?;
    Ok(stats)
}

/// The whole document.
pub fn render(g: &NxGraph, names: &Names) -> (String, SvgStats) {
    let n = g.nodes.len();
    let degree: Vec<usize> = (0..n as u32).map(|i| g.degree(i)).collect();
    let max_degree = degree.iter().copied().max().unwrap_or(0);
    let radius: Vec<f64> = degree
        .iter()
        .map(|&d| {
            let area = if max_degree == 0 {
                300.0
            } else {
                300.0 + 1200.0 * d as f64 / max_degree as f64
            };
            area.sqrt() / 2.0
        })
        .collect();
    let max_r = radius.iter().copied().fold(0.0, f64::max);

    let layout = if n <= FORCE_LIMIT {
        Layout::Force
    } else {
        Layout::Packed
    };
    let raw = match layout {
        Layout::Force => force_layout(g),
        Layout::Packed => packed_layout(g, &degree),
    };
    let margin = 24.0 + max_r;
    let labelled = labels_to_draw(n, layout, &degree);
    // Room for the half-width of the widest drawn label (about 4 pt per character at 7 pt).
    let half_label = labelled
        .iter()
        .map(|&i| {
            g.label(i as u32)
                .split('\n')
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0) as f64
        * 2.0;
    let margin_x = margin.max((half_label + 8.0).min(160.0));
    let (width, height, pos) = match layout {
        Layout::Force => {
            let f = (n as f64 / 800.0).sqrt().max(1.0);
            let (w, h) = (round1(WIDTH * f), round1(HEIGHT * f));
            (w, h, fit(&raw, w, h, (margin_x, margin), false))
        }
        Layout::Packed => {
            let (w, h) = extent(&raw);
            let (w, h) = (
                round1((w + 2.0 * margin_x).max(WIDTH)),
                round1((h + 2.0 * margin).max(HEIGHT)),
            );
            (w, h, fit(&raw, w, h, (margin_x, margin), true))
        }
    };

    let walk = g.edge_walk();
    let drawn = edges_to_draw(&walk, &degree);

    let mut out = String::with_capacity(256 + walk.len().min(EDGE_CAP) * 64 + n * 120);
    let _ = write!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" \
         font-family=\"DejaVu Sans, Verdana, Arial, sans-serif\">\n\
         <title>Knowledge graph: {n} nodes, {m} links</title>\n\
         <rect width=\"{w}\" height=\"{h}\" fill=\"{BACKGROUND}\"/>\n",
        w = num(width),
        h = num(height),
        m = g.edges.len(),
    );

    // Links whose confidence is present and not EXTRACTED are dashed and fainter.
    let dashed = |e: u32| {
        g.edge_str(e, "confidence")
            .is_some_and(|c| c != "EXTRACTED")
    };
    for (id, style, want) in [
        (
            "links",
            "stroke-opacity=\"0.6\" stroke-linecap=\"square\"",
            false,
        ),
        (
            "uncertain-links",
            "stroke-opacity=\"0.3\" stroke-dasharray=\"2.96 1.28\"",
            true,
        ),
    ] {
        let _ = writeln!(
            out,
            "<g id=\"{id}\" stroke=\"#aaaaaa\" stroke-width=\"0.8\" {style}>"
        );
        for &(u, v, e) in drawn.iter().map(|&k| &walk[k]) {
            if dashed(e) != want {
                continue;
            }
            let (a, b) = (pos[u as usize], pos[v as usize]);
            let _ = writeln!(
                out,
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
                num(a.0),
                num(a.1),
                num(b.0),
                num(b.1)
            );
        }
        out.push_str("</g>\n");
    }

    out.push_str("<g id=\"nodes\" fill-opacity=\"0.9\" stroke-opacity=\"0.9\">\n");
    for i in 0..n {
        let c = colour(g.community(i as u32));
        let _ = writeln!(
            out,
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{c}\" stroke=\"{c}\"/>",
            num(pos[i].0),
            num(pos[i].1),
            num(radius[i])
        );
    }
    out.push_str("</g>\n");

    out.push_str("<g id=\"labels\" fill=\"#ffffff\" font-size=\"7\" text-anchor=\"middle\">\n");
    for &i in &labelled {
        label(&mut out, g.label(i as u32), pos[i]);
    }
    out.push_str("</g>\n");

    legend(&mut out, g, names);
    out.push_str("</svg>\n");

    let stats = SvgStats {
        nodes: n,
        edges: g.edges.len(),
        edges_drawn: drawn.len(),
        labels_drawn: labelled.len(),
        layout,
        width,
        height,
    };
    (out, stats)
}

/// Tableau 10 by community modulo 10; a node without a community takes community 0's colour.
fn colour(community: Option<i64>) -> String {
    PALETTE[community.unwrap_or(0).rem_euclid(10) as usize].to_ascii_lowercase()
}

/// Links to draw, as indices into `walk`, in walk order: all of them up to [`EDGE_CAP`], else
/// those whose endpoints' degrees sum highest (ties in walk order). A self-loop is a
/// zero-length line, as in Graphify's drawing.
fn edges_to_draw(walk: &[(u32, u32, u32)], degree: &[usize]) -> Vec<usize> {
    let mut keep: Vec<usize> = (0..walk.len()).collect();
    if keep.len() > EDGE_CAP {
        let score = |k: usize| degree[walk[k].0 as usize] + degree[walk[k].1 as usize];
        keep.sort_by(|&a, &b| score(b).cmp(&score(a)).then(a.cmp(&b)));
        keep.truncate(EDGE_CAP);
        keep.sort_unstable();
    }
    keep
}

/// Nodes whose label is drawn, in node order.
fn labels_to_draw(n: usize, layout: Layout, degree: &[usize]) -> Vec<usize> {
    if layout == Layout::Force || n <= LABEL_CAP {
        return (0..n).collect();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| degree[b].cmp(&degree[a]).then(a.cmp(&b)));
    order.truncate(LABEL_CAP);
    order.sort_unstable();
    order
}

/// One `<text>`, a `<tspan>` per line, vertically centred on the node.
fn label(out: &mut String, text: &str, (x, y): (f64, f64)) {
    let lines: Vec<String> = text.split('\n').map(clean).collect();
    let (x, y) = (num(x), num(y));
    if lines.len() == 1 {
        let _ = write!(out, "<text x=\"{x}\" y=\"{y}\" dy=\"0.35em\">");
        escape(&lines[0], out);
        out.push_str("</text>\n");
        return;
    }
    let first = 0.35 - 0.6 * (lines.len() - 1) as f64;
    let _ = write!(out, "<text x=\"{x}\" y=\"{y}\">");
    for (k, line) in lines.iter().enumerate() {
        let dy = if k == 0 { first } else { 1.2 };
        let _ = write!(out, "<tspan x=\"{x}\" dy=\"{}em\">", num(dy));
        escape(line, out);
        out.push_str("</tspan>");
    }
    out.push_str("</text>\n");
}

/// Characters XML 1.0 cannot carry are dropped; a tab becomes a space.
fn clean(line: &str) -> String {
    line.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if (c as u32) < 0x20 => None,
            '\u{FFFE}' | '\u{FFFF}' => None,
            c => Some(c),
        })
        .collect()
}

/// Text content: `&`, `<` and `>` as entities (so `]]>` cannot occur either).
fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// The legend: every community with members, by id (the [`LEGEND_CAP`] largest when there are
/// more), as `name (members)` beside its colour.
fn legend(out: &mut String, g: &NxGraph, names: &Names) {
    let mut communities: Vec<(i64, usize)> = g
        .communities()
        .into_iter()
        .map(|(c, m)| (c, m.len()))
        .collect();
    if communities.is_empty() {
        return;
    }
    let total = communities.len();
    if total > LEGEND_CAP {
        communities.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        communities.truncate(LEGEND_CAP);
    }
    communities.sort_by_key(|&(c, _)| c);
    let rows = communities.len() + usize::from(total > LEGEND_CAP);
    let entries: Vec<String> = communities
        .iter()
        .map(|&(c, size)| {
            let name = match u32::try_from(c) {
                Ok(c) => names.get(c),
                Err(_) => format!("Community {c}"),
            };
            format!("{} ({size})", clean(&name.replace('\n', " ")))
        })
        .collect();
    let widest = entries
        .iter()
        .map(|e| e.chars().count())
        .max()
        .unwrap_or(0)
        .max(12);
    let (w, h) = (36.0 + widest as f64 * 5.0, 10.0 + rows as f64 * 12.0);
    let _ = writeln!(
        out,
        "<g id=\"legend\" font-size=\"8\">\n<rect x=\"11.2\" y=\"11.2\" width=\"{}\" height=\"{}\" \
         rx=\"1.6\" fill=\"#2a2a4e\" fill-opacity=\"0.7\" stroke=\"#cccccc\"/>",
        num(w),
        num(h)
    );
    for (k, (&(c, _), entry)) in communities.iter().zip(&entries).enumerate() {
        let y = 20.5 + k as f64 * 12.0;
        let _ = write!(
            out,
            "<rect x=\"16\" y=\"{}\" width=\"14\" height=\"5.6\" fill=\"{}\"/>\
             <text x=\"36.8\" y=\"{}\" dy=\"0.35em\" fill=\"#ffffff\">",
            num(y - 2.8),
            colour(Some(c)),
            num(y)
        );
        escape(entry, out);
        out.push_str("</text>\n");
    }
    if total > LEGEND_CAP {
        let y = 20.5 + communities.len() as f64 * 12.0;
        let _ = writeln!(
            out,
            "<text x=\"16\" y=\"{}\" dy=\"0.35em\" fill=\"#ffffff\">… and {} more communities</text>",
            num(y),
            total - LEGEND_CAP
        );
    }
    out.push_str("</g>\n");
}

/// SplitMix64: a small, portable, seeded generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Fruchterman–Reingold in the unit square: seeded start, linear cooling, a weak pull to the
/// centre so separate components stay in frame. O(n²) per iteration; only used up to
/// [`FORCE_LIMIT`] nodes.
fn force_layout(g: &NxGraph) -> Vec<(f64, f64)> {
    let n = g.nodes.len();
    let mut rng = Rng(SEED);
    let mut pos: Vec<(f64, f64)> = (0..n).map(|_| (rng.unit(), rng.unit())).collect();
    if n < 2 {
        return pos;
    }
    let k = (1.0 / n as f64).sqrt();
    let k2 = k * k;
    let springs: Vec<(usize, usize)> = g
        .edges
        .iter()
        .filter(|e| e.source != e.target)
        .map(|e| (e.source as usize, e.target as usize))
        .collect();
    let mut t = 0.1;
    let dt = t / (FORCE_ITERATIONS as f64 + 1.0);
    let mut disp = vec![(0.0f64, 0.0f64); n];
    for _ in 0..FORCE_ITERATIONS {
        disp.iter_mut().for_each(|d| *d = (0.0, 0.0));
        for i in 0..n {
            let (xi, yi) = pos[i];
            let (mut fx, mut fy) = (0.0, 0.0);
            for (j, &(xj, yj)) in pos.iter().enumerate().skip(i + 1) {
                let (dx, dy) = (xi - xj, yi - yj);
                let d2 = (dx * dx + dy * dy).max(1e-6);
                let f = k2 / d2;
                fx += dx * f;
                fy += dy * f;
                disp[j].0 -= dx * f;
                disp[j].1 -= dy * f;
            }
            disp[i].0 += fx;
            disp[i].1 += fy;
        }
        for &(u, v) in &springs {
            let (dx, dy) = (pos[u].0 - pos[v].0, pos[u].1 - pos[v].1);
            let d = (dx * dx + dy * dy).sqrt().max(1e-3);
            let f = d / k;
            disp[u].0 -= dx * f;
            disp[u].1 -= dy * f;
            disp[v].0 += dx * f;
            disp[v].1 += dy * f;
        }
        for (p, d) in pos.iter_mut().zip(&mut disp) {
            d.0 += (0.5 - p.0) * 2.0 * k;
            d.1 += (0.5 - p.1) * 2.0 * k;
            let len = (d.0 * d.0 + d.1 * d.1).sqrt();
            if len > 0.0 {
                let step = len.min(t) / len;
                p.0 += d.0 * step;
                p.1 += d.1 * step;
            }
        }
        t -= dt;
    }
    pos
}

/// Communities (nodes without one form a group of their own) on a sunflower spiral by size,
/// largest at the centre; each group's members on a spiral inside its disc, best connected
/// first. Units are points.
fn packed_layout(g: &NxGraph, degree: &[usize]) -> Vec<(f64, f64)> {
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    let mut groups: Vec<(Option<i64>, Vec<u32>)> = g
        .communities()
        .into_iter()
        .map(|(c, m)| (Some(c), m))
        .collect();
    let loose: Vec<u32> = (0..g.nodes.len() as u32)
        .filter(|&i| g.community(i).is_none())
        .collect();
    if !loose.is_empty() {
        groups.push((None, loose));
    }
    groups.sort_by(|a, b| {
        b.1.len().cmp(&a.1.len()).then_with(|| match (a.0, b.0) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
    });
    let mut pos = vec![(0.0, 0.0); g.nodes.len()];
    let mut area = 0.0;
    for (k, (_, members)) in groups.iter_mut().enumerate() {
        let r = (members.len() as f64).sqrt() * 13.0 + 12.0;
        area += r * r;
        let dist = area.sqrt() * 1.9;
        let angle = k as f64 * golden;
        let (cx, cy) = if k == 0 {
            (0.0, 0.0)
        } else {
            (dist * angle.cos(), dist * angle.sin())
        };
        area += r * r;
        members.sort_by(|&a, &b| degree[b as usize].cmp(&degree[a as usize]).then(a.cmp(&b)));
        let m = members.len() as f64;
        for (j, &i) in members.iter().enumerate() {
            let rr = r * ((j as f64 + 0.5) / m).sqrt();
            let a = j as f64 * golden;
            pos[i as usize] = (cx + rr * a.cos(), cy + rr * a.sin());
        }
    }
    pos
}

fn bounds(pos: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    pos.iter().fold(
        (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ),
        |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
    )
}

fn extent(pos: &[(f64, f64)]) -> (f64, f64) {
    if pos.is_empty() {
        return (0.0, 0.0);
    }
    let (x0, y0, x1, y1) = bounds(pos);
    (x1 - x0, y1 - y0)
}

/// Map positions into the canvas inside the margins; `equal` keeps the aspect ratio (and the
/// layout's own scale when it already fits), otherwise each axis is stretched to fill.
fn fit(pos: &[(f64, f64)], w: f64, h: f64, (mx, my): (f64, f64), equal: bool) -> Vec<(f64, f64)> {
    if pos.is_empty() {
        return Vec::new();
    }
    let (x0, y0, x1, y1) = bounds(pos);
    let (aw, ah) = ((w - 2.0 * mx).max(1.0), (h - 2.0 * my).max(1.0));
    let (sx, sy) = (x1 - x0, y1 - y0);
    let (kx, ky) = if equal {
        let k = (aw / sx.max(1e-9)).min(ah / sy.max(1e-9)).min(1.0);
        (k, k)
    } else {
        (aw / sx.max(1e-9), ah / sy.max(1e-9))
    };
    // Centre the drawing; a zero extent puts every node in the middle.
    let ox = mx + (aw - sx * kx) / 2.0;
    let oy = my + (ah - sy * ky) / 2.0;
    pos.iter()
        .map(|&(x, y)| (ox + (x - x0) * kx, oy + (y - y0) * ky))
        .collect()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// A coordinate with at most two decimals and no trailing zeros (`12`, `3.5`, `-0.25`).
fn num(x: f64) -> String {
    let mut s = format!("{:.2}", x);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short_and_stable() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(3.5), "3.5");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(1.236), "1.24");
    }

    #[test]
    fn sizes_follow_graphify() {
        // 300 + 1200·d/max pt², radius = √s / 2: d=1 of 18 → 9.574, d=18 → 19.365.
        let r = |d: f64, m: f64| (300.0 + 1200.0 * d / m).sqrt() / 2.0;
        assert_eq!(num(r(1.0, 18.0)), "9.57");
        assert_eq!(num(r(18.0, 18.0)), "19.36");
    }

    #[test]
    fn clean_drops_what_xml_forbids() {
        assert_eq!(clean("a\u{0}b\u{7}c\td\u{FFFF}"), "abc d");
    }
}
