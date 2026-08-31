//! File ranking (personalized PageRank over the name-reference graph) and the
//! token-budgeted repo-map renderer. Aider-style: edges are heuristic — file A
//! points at file B when A references a name B defines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::Atlas;

const DAMPING: f64 = 0.85;
const ITERATIONS: usize = 30;
const MAX_DEFS_PER_FILE: usize = 10;
const FOCUS_WEIGHT: f64 = 10.0;

impl Atlas {
    /// Rank indexed files by personalized PageRank. `focus` entries boost the
    /// teleport weight of files whose path contains the entry or that define
    /// a symbol with that exact name (typically: the session's working-state
    /// files plus caller-provided symbols/paths).
    pub fn ranked_files(&self, focus: &[String]) -> Vec<(PathBuf, f64)> {
        let paths: Vec<&PathBuf> = {
            let mut paths: Vec<&PathBuf> = self.files().map(|(path, _)| path).collect();
            paths.sort();
            paths
        };
        if paths.is_empty() {
            return Vec::new();
        }
        let index_of: HashMap<&PathBuf, usize> =
            paths.iter().enumerate().map(|(i, p)| (*p, i)).collect();

        // name -> defining files
        let mut defined_in: HashMap<&str, Vec<usize>> = HashMap::new();
        for (path, file) in self.files() {
            let id = index_of[path];
            for def in &file.extraction.definitions {
                defined_in.entry(def.name.as_str()).or_default().push(id);
            }
        }
        // Edges: referencing file -> defining file, weighted by count.
        let mut out_edges: Vec<HashMap<usize, f64>> = vec![HashMap::new(); paths.len()];
        for (path, file) in self.files() {
            let source = index_of[path];
            for reference in &file.extraction.references {
                if let Some(targets) = defined_in.get(reference.name.as_str()) {
                    let share = 1.0 / targets.len() as f64;
                    for &target in targets {
                        if target != source {
                            *out_edges[source].entry(target).or_insert(0.0) += share;
                        }
                    }
                }
            }
        }

        // Personalization vector; also remember direct matches for a
        // post-rank boost (PageRank pushes mass OUT of a focused leaf toward
        // whatever it references, so teleport weight alone cannot guarantee
        // the focused file itself surfaces).
        let mut teleport: Vec<f64> = vec![1.0; paths.len()];
        let mut focus_matched: Vec<bool> = vec![false; paths.len()];
        for entry in focus {
            let lowered = entry.to_lowercase();
            for (i, path) in paths.iter().enumerate() {
                if path.to_string_lossy().to_lowercase().contains(&lowered) {
                    teleport[i] += FOCUS_WEIGHT;
                    focus_matched[i] = true;
                }
            }
            if let Some(targets) = defined_in.get(entry.as_str()) {
                for &target in targets {
                    teleport[target] += FOCUS_WEIGHT;
                    focus_matched[target] = true;
                }
            }
        }
        let teleport_sum: f64 = teleport.iter().sum();
        for weight in &mut teleport {
            *weight /= teleport_sum;
        }

        let out_totals: Vec<f64> = out_edges
            .iter()
            .map(|edges| edges.values().sum::<f64>())
            .collect();
        let mut rank = teleport.clone();
        for _ in 0..ITERATIONS {
            let mut next = vec![0.0; paths.len()];
            let mut dangling = 0.0;
            for source in 0..paths.len() {
                if out_totals[source] == 0.0 {
                    dangling += rank[source];
                    continue;
                }
                for (&target, &weight) in &out_edges[source] {
                    next[target] += rank[source] * weight / out_totals[source];
                }
            }
            for i in 0..paths.len() {
                next[i] =
                    (1.0 - DAMPING) * teleport[i] + DAMPING * (next[i] + dangling * teleport[i]);
            }
            rank = next;
        }

        let mut ranked: Vec<(PathBuf, f64)> = paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let boost = if focus_matched[i] { 2.5 } else { 1.0 };
                ((*path).clone(), rank[i] * boost)
            })
            .collect();
        // Deterministic: score desc, then path asc.
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        ranked
    }

    /// Render a token-budgeted map: ranked files with their definition
    /// signatures. Budget is approximate (bytes/4).
    pub fn render_map(&self, focus: &[String], max_tokens: usize) -> String {
        let budget_bytes = max_tokens.saturating_mul(4);
        let ranked = self.ranked_files(focus);
        if ranked.is_empty() {
            return "Atlas: no indexable files found (rs, py, ts, tsx, js).".to_string();
        }
        let mut out = String::from("Atlas repo map (files ranked by relevance):\n");
        let mut rendered_files = 0usize;
        for (path, _score) in &ranked {
            let Some(file) = self.file(path) else {
                continue;
            };
            let mut block = format!("\n{}\n", path.display());
            let defs = &file.extraction.definitions;
            for def in defs.iter().take(MAX_DEFS_PER_FILE) {
                let container = def
                    .container
                    .as_deref()
                    .map(|c| format!("{c}::"))
                    .unwrap_or_default();
                block.push_str(&format!(
                    "  {} {}{}  · L{} · {}\n",
                    def.kind.label(),
                    container,
                    def.name,
                    def.line,
                    def.signature
                ));
            }
            if defs.len() > MAX_DEFS_PER_FILE {
                block.push_str(&format!(
                    "  … {} more definitions\n",
                    defs.len() - MAX_DEFS_PER_FILE
                ));
            }
            if out.len() + block.len() > budget_bytes {
                break;
            }
            out.push_str(&block);
            rendered_files += 1;
        }
        if rendered_files < ranked.len() {
            out.push_str(&format!(
                "\n({} of {} indexed files shown; refine with `focus` for more depth)\n",
                rendered_files,
                ranked.len()
            ));
        }
        out
    }

    pub fn file(&self, path: &Path) -> Option<&crate::FileIndex> {
        self.files()
            .find(|(candidate, _)| candidate.as_path() == path)
            .map(|(_, file)| file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // hub.rs is referenced by everyone; leaf.rs references hub.
        std::fs::write(
            dir.path().join("hub.rs"),
            "pub fn core_util() {}\npub fn other() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("leaf.rs"),
            "pub fn leaf_only() { core_util(); core_util(); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("app.ts"),
            "export function main() { return core_util(); }\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn referenced_files_outrank_leaves_and_focus_biases() {
        let dir = workspace();
        let mut atlas = Atlas::new(dir.path().to_path_buf());
        atlas.refresh();
        let ranked = atlas.ranked_files(&[]);
        assert_eq!(ranked[0].0, PathBuf::from("hub.rs"), "{ranked:?}");
        // Focusing a leaf symbol pulls its file up.
        let focused = atlas.ranked_files(&["leaf_only".to_string()]);
        assert_eq!(focused[0].0, PathBuf::from("leaf.rs"), "{focused:?}");
        // Determinism.
        assert_eq!(atlas.ranked_files(&[]), atlas.ranked_files(&[]));
    }

    #[test]
    fn map_respects_token_budget_and_shows_signatures() {
        let dir = workspace();
        let mut atlas = Atlas::new(dir.path().to_path_buf());
        atlas.refresh();
        let map = atlas.render_map(&[], 2_000);
        assert!(map.contains("hub.rs"));
        assert!(map.contains("fn core_util"));
        let tiny = atlas.render_map(&[], 40);
        assert!(tiny.len() < 400, "budget ignored: {}", tiny.len());
        assert!(tiny.contains("of 3 indexed files shown"));
    }
}
