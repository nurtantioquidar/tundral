//! Persistent per-file facts cache in one redb file.
//!
//! Facts are keyed by blake3 over the extractor fingerprint, language, relative path and content, so
//! a hit is always valid. A stat index (mtime, size → key) lets unchanged files skip reading and
//! hashing entirely; a file modified within `RACY_WINDOW_NS` of the last cache write is re-hashed,
//! because its mtime cannot prove it was not rewritten in the same timestamp tick.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use redb::{Builder, Database, Durability, ReadableTable, TableDefinition};

use crate::facts::FileFacts;
use crate::lang::Lang;

/// Changes whenever any extraction source changes (see `build.rs`).
pub const EXTRACTOR_FINGERPRINT: &str = env!("DIJKSTRAL_EXTRACTOR_FINGERPRINT");

const FACTS: TableDefinition<&[u8; 32], &[u8]> = TableDefinition::new("facts");
const STATS: TableDefinition<&str, &[u8]> = TableDefinition::new("stats");
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
const RACY_WINDOW_NS: u64 = 2_000_000_000;
/// Files encoded and inserted per transaction during a commit.
const COMMIT_CHUNK: usize = 2048;
/// redb page cache; the default (1 GiB) dominated peak memory on large cold builds.
const CACHE_BYTES: usize = 64 << 20;

pub type Key = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub mtime_ns: u64,
    pub size: u64,
}

pub struct Cache {
    db: Database,
    stats: HashMap<String, (Stat, Key)>,
    written_at_ns: u64,
}

/// The extractor fingerprint as stored in the cache's metadata.
fn fingerprint_u64() -> u64 {
    u64::from_str_radix(EXTRACTOR_FINGERPRINT, 16).unwrap_or(0)
}

pub fn content_key(lang: Lang, rel: &str, content: &[u8]) -> Key {
    let mut h = blake3::Hasher::new();
    h.update(EXTRACTOR_FINGERPRINT.as_bytes());
    h.update(&[lang as u8]);
    h.update(rel.as_bytes());
    h.update(&[0]);
    h.update(content);
    *h.finalize().as_bytes()
}

impl Cache {
    pub fn open(path: &Path) -> Result<Cache> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let create = || Builder::new().set_cache_size(CACHE_BYTES).create(path);
        let db = match create() {
            Ok(db) => db,
            // A corrupt or incompatible cache is disposable: start over.
            Err(_) => {
                std::fs::remove_file(path).ok();
                create().context("creating cache")?
            }
        };
        let tx = db.begin_write()?;
        tx.open_table(FACTS)?;
        let mut written_at_ns;
        {
            let mut meta = tx.open_table(META)?;
            written_at_ns = meta.get("written_at_ns")?.map_or(0, |v| v.value());
            let recorded = meta.get("extractor")?.map(|v| v.value());
            // The stat index maps a file to the key it was last hashed under; keys mix in the
            // extractor fingerprint, so a stat hit from an older extractor would serve its facts.
            // A different extractor invalidates the whole index (facts are pruned on commit).
            if recorded != Some(fingerprint_u64()) {
                tx.delete_table(STATS)?;
                meta.insert("extractor", fingerprint_u64())?;
                written_at_ns = 0;
            }
        }
        tx.open_table(STATS)?;
        tx.commit()?;

        let mut stats = HashMap::new();
        let tx = db.begin_read()?;
        for row in tx.open_table(STATS)?.iter()? {
            let (k, v) = row?;
            if let Some(entry) = decode_stat(v.value()) {
                stats.insert(k.value().to_string(), entry);
            }
        }
        Ok(Cache {
            db,
            stats,
            written_at_ns,
        })
    }

    /// The key recorded for `rel` if its stat still matches and is not racy.
    pub fn key_from_stat(&self, rel: &str, stat: Stat) -> Option<Key> {
        let (cached, key) = self.stats.get(rel)?;
        let racy = stat.mtime_ns.saturating_add(RACY_WINDOW_NS) >= self.written_at_ns;
        (*cached == stat && !racy).then_some(*key)
    }

    /// Look up many keys; missing or undecodable entries are absent from the result.
    pub fn get_many(&self, keys: &[Key]) -> Result<HashMap<Key, FileFacts>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(FACTS)?;
        let mut raw = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(v) = table.get(key)? {
                raw.push((*key, v.value().to_vec()));
            }
        }
        use rayon::prelude::*;
        Ok(raw
            .into_par_iter()
            .filter_map(|(k, bytes)| bincode::deserialize(&bytes).ok().map(|f| (k, f)))
            .collect())
    }

    /// Persist new facts, the current stat index, and drop entries no file uses any more.
    pub fn commit(
        &mut self,
        new_facts: &[(Key, &FileFacts)],
        stats: HashMap<String, (Stat, Key)>,
        now_ns: u64,
    ) -> Result<()> {
        // Encode and insert a bounded chunk per transaction so neither the encoded bytes nor
        // redb's dirty pages for a whole cold build are held at once. Intermediate commits are
        // non-durable; the final commit below makes them durable together.
        for chunk in new_facts.chunks(COMMIT_CHUNK) {
            let encoded: Vec<(Key, Vec<u8>)> = {
                use rayon::prelude::*;
                chunk
                    .par_iter()
                    .map(|(k, f)| Ok((*k, bincode::serialize(f)?)))
                    .collect::<Result<_>>()?
            };
            let mut tx = self.db.begin_write()?;
            tx.set_durability(Durability::None);
            {
                let mut facts = tx.open_table(FACTS)?;
                for (k, bytes) in &encoded {
                    facts.insert(k, bytes.as_slice())?;
                }
            }
            tx.commit()?;
        }
        let live: HashSet<Key> = stats.values().map(|(_, k)| *k).collect();
        let tx = self.db.begin_write()?;
        {
            let mut facts = tx.open_table(FACTS)?;
            let stale: Vec<Key> = facts
                .iter()?
                .filter_map(|row| row.ok().map(|(k, _)| *k.value()))
                .filter(|k| !live.contains(k))
                .collect();
            for k in &stale {
                facts.remove(k)?;
            }
            let mut table = tx.open_table(STATS)?;
            let gone: Vec<String> = self
                .stats
                .keys()
                .filter(|k| !stats.contains_key(*k))
                .cloned()
                .collect();
            for k in &gone {
                table.remove(k.as_str())?;
            }
            for (rel, entry) in &stats {
                if self.stats.get(rel) != Some(entry) {
                    table.insert(rel.as_str(), encode_stat(entry).as_slice())?;
                }
            }
            tx.open_table(META)?.insert("written_at_ns", now_ns)?;
        }
        tx.commit()?;
        self.stats = stats;
        self.written_at_ns = now_ns;
        Ok(())
    }
}

fn encode_stat((stat, key): &(Stat, Key)) -> [u8; 48] {
    let mut out = [0u8; 48];
    out[..8].copy_from_slice(&stat.mtime_ns.to_le_bytes());
    out[8..16].copy_from_slice(&stat.size.to_le_bytes());
    out[16..].copy_from_slice(key);
    out
}

fn decode_stat(bytes: &[u8]) -> Option<(Stat, Key)> {
    if bytes.len() != 48 {
        return None;
    }
    let mtime_ns = u64::from_le_bytes(bytes[..8].try_into().ok()?);
    let size = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    Some((Stat { mtime_ns, size }, bytes[16..].try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_different_extractor_invalidates_the_stat_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        let stat = Stat {
            mtime_ns: 1,
            size: 2,
        };
        let key = [7u8; 32];
        {
            let mut cache = Cache::open(&path).unwrap();
            let facts = FileFacts::default();
            let mut stats = HashMap::new();
            stats.insert("a.py".to_string(), (stat, key));
            cache.commit(&[(key, &facts)], stats, u64::MAX).unwrap();
            assert_eq!(
                Cache::open(&path).unwrap().stats.get("a.py"),
                Some(&(stat, key))
            );
        }
        // Simulate a cache written by another extractor build.
        {
            let db = Builder::new().create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(META)
                .unwrap()
                .insert("extractor", 1u64)
                .unwrap();
            tx.commit().unwrap();
        }
        let cache = Cache::open(&path).unwrap();
        assert!(cache.stats.is_empty());
        assert_eq!(cache.key_from_stat("a.py", stat), None);
    }
}
