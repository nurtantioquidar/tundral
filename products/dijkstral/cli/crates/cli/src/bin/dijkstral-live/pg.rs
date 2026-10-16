//! `extract --postgres DSN` inside `dijkstral-live` (`docs/graphify-inputs.md` §3.1): the only code
//! that talks to a database. The schema → graph mapping is `dijkstral_core::inputs::postgres`.

use anyhow::{anyhow, Context, Result};
use dijkstral_core::inputs::postgres::{
    dsn_source, Catalog, ForeignKey, Function, Relation, Snapshot, View,
};

/// Read the user schema of the database `dsn` names (`docs/graphify-inputs.md` §3.1).
pub fn fetch(dsn: &str) -> Result<Snapshot> {
    let (host, database) = dsn_source(dsn);
    let mut config: postgres::Config = if dsn.trim().is_empty() {
        postgres::Config::new()
    } else {
        // The parser's message never repeats the DSN, which may hold a password.
        dsn.parse()
            .map_err(|e| anyhow!("--postgres: not a PostgreSQL connection string ({e})"))?
    };
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if config.get_hosts().is_empty() {
        config.host(&env("PGHOST").unwrap_or_else(|| "localhost".into()));
    }
    if config.get_ports().is_empty() {
        if let Some(port) = env("PGPORT").and_then(|p| p.parse().ok()) {
            config.port(port);
        }
    }
    if config.get_user().is_none() {
        if let Some(user) = env("PGUSER").or_else(|| env("USER")) {
            config.user(&user);
        }
    }
    if config.get_password().is_none() {
        if let Some(pw) = env("PGPASSWORD") {
            config.password(pw);
        }
    }
    if config.get_dbname().is_none() {
        if let Some(db) = env("PGDATABASE") {
            config.dbname(&db);
        }
    }
    let target = format!(
        "{}/{}",
        host,
        config.get_dbname().unwrap_or(database.as_str())
    );
    let mut client = config
        .connect(postgres::NoTls)
        .with_context(|| format!("could not connect to PostgreSQL at {target}"))?;
    let catalog = read_catalog(&mut client).with_context(|| format!("reading {target}"))?;
    Ok(Snapshot {
        host,
        database,
        catalog,
    })
}

const USER_SCHEMAS: &str = "n.nspname NOT IN ('pg_catalog', 'information_schema') \
     AND n.nspname NOT LIKE 'pg\\_toast%' AND n.nspname NOT LIKE 'pg\\_temp\\_%'";

fn read_catalog(client: &mut postgres::Client) -> Result<Catalog> {
    let rows = |client: &mut postgres::Client, sql: String| client.query(sql.as_str(), &[]);
    let tables = rows(
        client,
        format!(
            "SELECT n.nspname::text, c.relname::text FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r', 'p') AND {USER_SCHEMAS}"
        ),
    )?
    .iter()
    .map(|r| Relation {
        schema: r.get(0),
        name: r.get(1),
    })
    .collect();
    let views = rows(
        client,
        format!(
            "SELECT n.nspname::text, c.relname::text, pg_catalog.pg_get_viewdef(c.oid) \
             FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind = 'v' AND {USER_SCHEMAS}"
        ),
    )?
    .iter()
    .map(|r| View {
        schema: r.get(0),
        name: r.get(1),
        definition: r.get::<_, Option<String>>(2).unwrap_or_default(),
    })
    .collect();
    let foreign_keys = rows(
        client,
        format!(
            "SELECT n.nspname::text, c.relname::text, con.conname::text, rn.nspname::text, \
             rc.relname::text FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_catalog.pg_class rc ON rc.oid = con.confrelid \
             JOIN pg_catalog.pg_namespace rn ON rn.oid = rc.relnamespace \
             WHERE con.contype = 'f' AND {USER_SCHEMAS}"
        ),
    )?
    .iter()
    .map(|r| ForeignKey {
        schema: r.get(0),
        table: r.get(1),
        name: r.get(2),
        ref_schema: r.get(3),
        ref_table: r.get(4),
    })
    .collect();
    let functions = rows(
        client,
        format!(
            "SELECT n.nspname::text, p.proname::text, p.oid, p.prosrc \
             FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE p.prokind IN ('f', 'p') AND {USER_SCHEMAS}"
        ),
    )?
    .iter()
    .map(|r| Function {
        schema: r.get(0),
        name: r.get(1),
        oid: r.get(2),
        source: r.get::<_, Option<String>>(3).unwrap_or_default(),
    })
    .collect();
    let mut catalog = Catalog {
        tables,
        views,
        foreign_keys,
        functions,
    };
    catalog.sort();
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Live check of the catalog reader against the probe databases of
    /// `tests/fixtures/inputs/postgres/` (`<db>.sql` loaded into a fresh `postgres:16`). Needs a
    /// server: `DIJKSTRAL_PG_TEST_DSN=postgresql://user:pw@127.0.0.1:5432` (no database part).
    /// With `DIJKSTRAL_PG_WRITE=1` it rewrites the `<db>.catalog.json` snapshots instead.
    #[test]
    #[ignore = "needs a live PostgreSQL loaded with the probe schemas"]
    fn live_catalogs_match_the_snapshots() {
        let base = std::env::var("DIJKSTRAL_PG_TEST_DSN").expect("DIJKSTRAL_PG_TEST_DSN");
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/inputs/postgres");
        for db in ["shop", "probe2", "probe3"] {
            let snap = super::fetch(&format!("{base}/{db}")).unwrap();
            let path = dir.join(format!("{db}.catalog.json"));
            let text = serde_json::to_string_pretty(&snap).unwrap() + "\n";
            if std::env::var("DIJKSTRAL_PG_WRITE").is_ok() {
                std::fs::write(&path, text).unwrap();
            } else {
                assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{db}");
            }
        }
    }
}
