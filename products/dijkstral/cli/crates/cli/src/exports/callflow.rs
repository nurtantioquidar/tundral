//! `dijkstral export callflow-html` (`docs/graphify-exports.md` §13): the Mermaid architecture
//! and call-flow page, `<project>-callflow.html` next to the graph by default.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Args, ValueEnum};
use dijkstral_export::callflow::{
    self, CallflowOptions, Lang, LangChoice, CALLFLOW_SUFFIX, DEFAULT_MAX_SECTIONS,
};
use dijkstral_export::nx::NxGraph;

use crate::export_cmd::{graph_path, project_name, sibling, GraphOnly};

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LangArg {
    Auto,
    #[value(name = "zh-CN")]
    ZhCn,
    En,
}

#[derive(Args, Debug)]
pub struct CallflowArgs {
    /// A graph.json, or a directory holding one (itself, dijkstral-out/ or graphify-out/).
    #[arg(value_name = "GRAPH|DIR")]
    pub input: Option<PathBuf>,
    /// Graph to export (same as GRAPH). Default: dijkstral-out/graph.json, else
    /// graphify-out/graph.json.
    #[arg(long, value_name = "PATH", conflicts_with = "input")]
    pub graph: Option<PathBuf>,
    /// Accepted for Graphify compatibility; section names come from the graph's own words.
    #[arg(long, value_name = "PATH")]
    pub labels: Option<PathBuf>,
    /// Accepted for Graphify compatibility; the page is built from the graph alone.
    #[arg(long, value_name = "PATH")]
    pub report: Option<PathBuf>,
    /// Custom section definitions (not supported: Graphify's format is not established).
    #[arg(long, value_name = "PATH")]
    pub sections: Option<PathBuf>,
    /// Page to write. Default: <project>-callflow.html next to the graph.
    #[arg(long, value_name = "HTML")]
    pub output: Option<PathBuf>,
    /// Page language; auto picks Chinese when node labels contain CJK characters.
    #[arg(long, value_enum, default_value = "auto")]
    pub lang: LangArg,
    /// Sections before the rest are merged into "Other" (0: 15; below 2: 4).
    #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_SECTIONS, allow_hyphen_values = true)]
    pub max_sections: i64,
    /// Scale of the diagrams' text.
    #[arg(long, value_name = "N", default_value_t = 1.0)]
    pub diagram_scale: f64,
}

/// The graph a positional GRAPH|DIR or --graph names.
fn resolve(a: &CallflowArgs) -> Result<PathBuf> {
    let given = a.input.clone().or_else(|| a.graph.clone());
    let Some(p) = given else {
        return Ok(graph_path(&GraphOnly { graph: None }));
    };
    if p.is_dir() {
        for cand in [
            "graph.json",
            "dijkstral-out/graph.json",
            "graphify-out/graph.json",
        ] {
            let c = p.join(cand);
            if c.is_file() {
                return Ok(c);
            }
        }
        bail!("no graph.json in {}", p.display());
    }
    Ok(p)
}

/// `YYYY-MM-DD HH:MM UTC` for now.
/// The page's "Generated:" time (UTC, minute resolution). `SOURCE_DATE_EPOCH` pins it, like the
/// report's date, so a page can be rebuilt byte for byte.
fn utc_stamp() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub fn run(a: CallflowArgs) -> Result<String> {
    if a.sections.is_some() {
        bail!("--sections is not supported by dijkstral (Graphify's section-file format is not established)");
    }
    if !(a.diagram_scale.is_finite() && a.diagram_scale > 0.0) {
        bail!("--diagram-scale must be a positive number");
    }
    let path = resolve(&a)?;
    let graph = NxGraph::read(&path)?;
    let (commit, hyperedges) = callflow::graph_trailer(&path);
    let project = project_name(&path);
    let opts = CallflowOptions {
        project: project.clone(),
        lang: match a.lang {
            LangArg::Auto => LangChoice::Auto,
            LangArg::En => LangChoice::Fixed(Lang::En),
            LangArg::ZhCn => LangChoice::Fixed(Lang::ZhCn),
        },
        max_sections: a.max_sections,
        diagram_scale: a.diagram_scale,
        commit,
        hyperedges,
        generated: utc_stamp(),
    };
    let page = callflow::build(&graph, &opts);
    drop(graph);
    let out = match a.output {
        Some(o) => o,
        None => sibling(&path, &format!("{project}{CALLFLOW_SUFFIX}")),
    };
    page.write(&out)
        .with_context(|| format!("writing {}", out.display()))?;
    Ok(format!(
        "wrote {} ({} sections, {} diagrams)",
        out.display(),
        page.sections.len() + 1,
        page.sections.len() + 1
    ))
}
