// Vault commands. Wraps `rezon_core::{search, embed, vault}` behind a
// small VaultCtx that owns the SearchState + EmbedState arcs needed
// by both the search commands and the agent's search_notes tool.
//
// SearchState is created eagerly (cheap — just a map + a data dir);
// the per-vault SQLite database isn't opened until `open(path)` is
// called. EmbedState is also eager but holds no model until the user
// runs `/embed <path>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use rezon_core::embed::{self, EmbedState, EmbedStatus};
use rezon_core::search::{self, SearchHit, SearchState};
use rezon_core::vault as core_vault;

pub struct VaultCtx {
    pub search: Arc<SearchState>,
    pub embed: Arc<EmbedState>,
}

impl VaultCtx {
    pub fn new() -> Result<Self> {
        let data_dir = data_dir()?;
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("mkdir {}", data_dir.display()))?;
        Ok(Self {
            search: Arc::new(SearchState::new(data_dir)),
            embed: Arc::new(EmbedState::default()),
        })
    }

    pub fn active_vault(&self) -> Option<String> {
        self.search.active_vault()
    }

    /// Open a vault directory: registers the SQLite index, kicks off
    /// the file watcher, and (if an embed model is loaded) starts the
    /// catch-up loop that fills `vec_chunks`.
    pub fn open(&self, path: &str) -> Result<()> {
        let p = Path::new(path);
        if !p.is_dir() {
            return Err(anyhow!("not a directory: {path}"));
        }
        let canonical = std::fs::canonicalize(p)
            .with_context(|| format!("canonicalize {path}"))?
            .to_string_lossy()
            .to_string();
        search::vault_index_open(&self.search, &canonical).map_err(|e| anyhow!(e))?;
        embed::ensure_catchup_started(self.embed.clone(), self.search.clone());
        Ok(())
    }

    pub fn read_note(&self, p: &str) -> Result<String> {
        let vault = self
            .active_vault()
            .ok_or_else(|| anyhow!("no vault is open"))?;
        let resolved = resolve_in_vault(&vault, p);
        core_vault::vault_read(vault, resolved).map_err(|e| anyhow!(e))
    }

    pub fn find(&self, query: &str, limit: usize) -> Result<(Vec<SearchHit>, &'static str)> {
        let vault = self
            .active_vault()
            .ok_or_else(|| anyhow!("no vault is open"))?;
        if self.embed.status().loaded {
            let hits = embed::semantic_query(&self.embed, &self.search, &vault, query, limit)
                .map_err(|e| anyhow!(e))?;
            if !hits.is_empty() {
                return Ok((hits, "semantic"));
            }
        }
        let hits = search::vault_search_impl(&self.search, &vault, query, limit)
            .map_err(|e| anyhow!(e))?;
        Ok((hits, "fulltext"))
    }

    pub async fn load_embed(&self, path: String) -> Result<EmbedStatus> {
        let status = self
            .embed
            .load(path)
            .await
            .map_err(|e| anyhow!("load embed: {e}"))?;
        // Start (or re-arm) the catch-up loop now that the embed
        // worker is alive. `ensure_catchup_started` is idempotent.
        embed::ensure_catchup_started(self.embed.clone(), self.search.clone());
        Ok(status)
    }

    pub fn embed_status(&self) -> EmbedStatus {
        self.embed.status()
    }

    /// Close the active vault index and stop its watcher. Returns
    /// `true` if a vault was actually closed.
    pub fn close(&self) -> bool {
        let Some(vault) = self.active_vault() else {
            return false;
        };
        self.search.close_vault(&vault)
    }

    pub fn shutdown(&self) {
        self.embed.shutdown();
        self.search.shutdown();
    }
}

/// Resolve a path that may be relative to the active vault root.
fn resolve_in_vault(vault: &str, p: &str) -> String {
    if Path::new(p).is_absolute() {
        return p.to_string();
    }
    let mut joined = PathBuf::from(vault);
    joined.push(p);
    // If the user typed a stem without an extension and a `.md`
    // sibling exists, prefer that. Cheap mirror of the wikilink
    // resolver in core.
    if joined.extension().is_none() {
        let with_md = joined.with_extension("md");
        if with_md.is_file() {
            return with_md.to_string_lossy().to_string();
        }
    }
    joined.to_string_lossy().to_string()
}

fn data_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "rezon", "rezon-tui")
        .context("could not resolve user data dir")?;
    Ok(dirs.data_dir().to_path_buf())
}
