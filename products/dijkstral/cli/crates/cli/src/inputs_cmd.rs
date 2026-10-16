//! `extract`'s inputs besides the walked files (`docs/graphify-inputs.md`): `--cargo`,
//! `--postgres DSN` (read by `dijkstral-live`, `crate::live`) and `--no-gitignore`.

use anyhow::Result;
use clap::Args;
use dijkstral_core::inputs::Inputs;

#[derive(Args, Debug, Default, Clone)]
pub struct InputFlags {
    /// Also add crate → crate dependencies of the Cargo workspace, read from its Cargo.toml
    /// files (cargo is not run).
    #[arg(long)]
    pub cargo: bool,
    /// Also add the schema of a live PostgreSQL database: tables, views, functions and procedures,
    /// and foreign-key links (columns are not represented). DSN: postgresql://user@host/db or
    /// "host=… dbname=…"; "" uses the PGHOST, PGPORT, PGUSER, PGPASSWORD and PGDATABASE
    /// variables. Prefer PGPASSWORD over a password in the DSN, which other users can see in the
    /// process list. Plain TCP only (no TLS).
    #[arg(long, value_name = "DSN")]
    pub postgres: Option<String>,
    /// Do not read .gitignore files or .git/info/exclude; .graphifyignore and .dijkstralignore
    /// still apply.
    #[arg(long)]
    pub no_gitignore: bool,
}

impl InputFlags {
    /// The build inputs; connects to PostgreSQL when `--postgres` is given.
    pub fn inputs(&self) -> Result<Inputs> {
        Ok(Inputs {
            gitignore: !self.no_gitignore,
            cargo: self.cargo,
            postgres: self
                .postgres
                .as_deref()
                .map(crate::live::postgres)
                .transpose()?,
        })
    }
}
