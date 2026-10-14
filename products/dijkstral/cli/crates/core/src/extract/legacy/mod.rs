//! Code Graphify counts and parses without a tree-sitter grammar (`docs/graphify-format.md` §44):
//! Pascal/Delphi (`.pas .dpr .pp .inc .lpr .dpk`), Delphi/Lazarus forms (`.dfm .lfm`), Lazarus
//! packages (`.lpk`), Apex (`.cls .trigger`), COBOL (`.cob .cbl .cpy`) and BYOND interface files
//! (`.dmf .dmi .dmm`). Each is a small line scanner reproducing what Graphify emits; what needs
//! other files (Pascal `uses`, base classes and inherited calls) is recorded in
//! [`LegacyFacts`] and bound by `crate::resolve::legacy`.

mod apex;
mod byond;
mod cobol;
mod forms;
mod lpk;
pub mod pascal;

use serde::{Deserialize, Serialize};

pub use pascal::PascalFacts;

use super::{origin, Ctx};
use crate::facts::{Attr, LangFacts, Loc, Node, NodeKind};
use crate::lang::Lang;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LegacyFacts {
    pub pascal: Option<PascalFacts>,
    /// `.lpk` units: (local node id, unit name, virtual target of the package's `contains`).
    pub units: Vec<(String, String, String)>,
    /// `.lpk`: (package node id, virtual target of the file's `contains` edge to it).
    pub package: Option<(String, String)>,
}

pub fn extract(cx: &mut Ctx) {
    let facts = match cx.lang {
        Lang::Pascal => pascal::extract(cx),
        Lang::DelphiForm => {
            forms::extract(cx);
            LegacyFacts::default()
        }
        Lang::Lpk => lpk::extract(cx),
        Lang::Apex => {
            apex::extract(cx);
            LegacyFacts::default()
        }
        Lang::Cobol => {
            cobol::extract(cx);
            LegacyFacts::default()
        }
        Lang::Byond => {
            byond::extract(cx);
            LegacyFacts::default()
        }
        _ => return,
    };
    if facts != LegacyFacts::default() {
        cx.facts.lang = LangFacts::Legacy(facts);
    }
}

/// The file node, located at `L1` even when the file is empty (these scanners always say so).
pub(crate) fn file_node(cx: &mut Ctx, extra: Vec<(String, Attr)>) {
    cx.add_file_node(extra);
    if let Some(n) = cx.facts.nodes.last_mut() {
        n.loc = Loc::Line(1);
    }
}

/// A definition of this file unless the id is already emitted (the first one wins).
pub(crate) fn def(
    cx: &mut Ctx,
    id: &str,
    label: &str,
    line: u32,
    extra: Vec<(String, Attr)>,
) -> bool {
    if cx.facts.nodes.iter().any(|n| n.id == id) {
        return false;
    }
    let mut attrs = extra;
    attrs.push(origin());
    cx.facts.nodes.push(Node {
        id: id.into(),
        label: label.into(),
        kind: NodeKind::Def,
        file_type: "code".into(),
        source_file: cx.rel.into(),
        loc: Loc::Line(line),
        attrs,
    });
    true
}

/// Line starts of a text, for byte offset → line.
pub(crate) struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    pub fn new(text: &str) -> Lines {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { starts }
    }

    /// 1-based line of a byte offset.
    pub fn line(&self, offset: usize) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }
}

pub(crate) fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Every maximal word (`\w+`) with its byte offset.
pub(crate) fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (is_word(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &text[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out.into_iter()
}
