//! `dijkstral export cypher|neo4j|falkordb` (`docs/graphify-exports.md` §10).

use std::path::Path;

use anyhow::{bail, Result};
use clap::Args;
use dijkstral_export::cypher::{self, CYPHER_FILE};
use dijkstral_export::nx::NxGraph;
use dijkstral_export::push::{self, Credentials};

use crate::export_cmd::{graph_path, sibling, GraphOnly};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Neo4j,
    FalkorDb,
}

#[derive(Args, Debug)]
pub struct PushArgs {
    #[command(flatten)]
    pub graph: GraphOnly,
    /// Load the graph into this server instead of writing cypher.txt (Neo4j: bolt://host:7687
    /// or neo4j://; FalkorDB: falkordb://host:6379 or redis://). Plain TCP only.
    #[arg(long, value_name = "URI")]
    pub push: Option<String>,
    /// User for --push (Neo4j default: neo4j).
    #[arg(long, value_name = "U")]
    pub user: Option<String>,
    /// Password for --push; prefer NEO4J_PASSWORD / FALKORDB_PASSWORD to keep it off argv.
    #[arg(long, value_name = "P")]
    pub password: Option<String>,
    /// FalkorDB graph key --push writes to.
    #[arg(long, value_name = "NAME", default_value = "dijkstral")]
    pub name: String,
}

/// `<dir>/cypher.txt` as Graphify prints it (`./cypher.txt` next to a graph in the current
/// directory).
fn shown(graph: &Path) -> String {
    let dir = graph.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = if dir.as_os_str().is_empty() {
        ".".into()
    } else {
        dir.display().to_string()
    };
    format!("{dir}/{CYPHER_FILE}")
}

fn write_file(g: &GraphOnly) -> Result<(String, NxGraph)> {
    let path = graph_path(g);
    let graph = NxGraph::read(&path)?;
    cypher::write(&graph, &sibling(&path, CYPHER_FILE))?;
    Ok((shown(&path), graph))
}

pub fn run_file(g: &GraphOnly) -> Result<String> {
    let (shown, _) = write_file(g)?;
    Ok(format!("wrote {shown}"))
}

pub fn run(a: PushArgs, target: Target) -> Result<String> {
    let Some(uri) = &a.push else {
        let (shown, _) = write_file(&a.graph)?;
        return Ok(match target {
            Target::Neo4j => format!("cypher.txt written - import with: cypher-shell < {shown}"),
            Target::FalkorDb => format!(
                "cypher.txt written ({shown}) - statements are OpenCypher. FalkorDB's \
                 GRAPH.QUERY runs one statement at a time (no bulk script import), so load a \
                 graph with: dijkstral export falkordb --push falkordb://localhost:6379"
            ),
        });
    };
    let env = match target {
        Target::Neo4j => "NEO4J_PASSWORD",
        Target::FalkorDb => "FALKORDB_PASSWORD",
    };
    let cred = Credentials {
        user: a.user.clone(),
        password: a
            .password
            .clone()
            .or_else(|| std::env::var(env).ok().filter(|p| !p.is_empty())),
    };
    if target == Target::Neo4j && cred.password.is_none() {
        bail!("--password (or {env}) required for --push");
    }
    let graph = NxGraph::read(&graph_path(&a.graph))?;
    let plan = cypher::push_plan(&graph);
    drop(graph);
    let (name, report) = match target {
        Target::Neo4j => ("Neo4j", push::neo4j(&plan, uri, &cred)?),
        Target::FalkorDb => ("FalkorDB", push::falkordb(&plan, uri, &a.name, &cred)?),
    };
    Ok(format!(
        "Pushed to {name}: {} nodes, {} edges",
        report.nodes, report.edges
    ))
}
