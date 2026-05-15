// `search_notes` — agent tool that retrieves notes from the user's
// currently-open vault. Tries semantic search first (vector KNN over
// embedded chunks); falls back to FTS5 if no embedding model is
// loaded or no chunks have been embedded yet. Both modes return the
// same shape so the model treats results uniformly.
//
// The vault path is resolved from `SearchState::active_vault`, which
// is set by `vault_index_open` when the user picks a vault. This
// keeps the tool's parameter surface minimal — the model only needs
// to pick a query string, not the user's directory layout.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

use crate::agent::tool::{Tool, ToolContext, ToolError};
use crate::embed::semantic_query;
use crate::search::{vault_search_impl, SearchState};

pub struct SearchNotes;

#[async_trait]
impl Tool for SearchNotes {
    fn name(&self) -> &str {
        "search_notes"
    }

    fn description(&self) -> &str {
        "Search the user's notes vault for relevant content. Returns \
         up to `limit` snippets, each tagged with a note path. Uses \
         vector similarity when an embedding model is loaded; \
         otherwise falls back to full-text keyword search."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language description of what you're looking for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default 8, max 50).",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["query"]
        })
    }

    // Read-only over user files; the same as file_read but bounded to
    // the active vault. The vault was already chosen by the user; we
    // don't prompt again on each query.
    fn requires_confirmation(&self) -> bool {
        false
    }

    async fn dispatch(&self, args: Value, ctx: &ToolContext) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            query: String,
            limit: Option<u32>,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::Argument(format!("invalid args: {e}")))?;
        let query = args.query.trim().to_string();
        if query.is_empty() {
            return Err(ToolError::Argument("query is empty".into()));
        }
        let limit = args.limit.unwrap_or(8).clamp(1, 50) as usize;

        let app = ctx
            .app
            .clone()
            .ok_or_else(|| ToolError::Argument("no app handle available".into()))?;

        let vault = app
            .state::<SearchState>()
            .active_vault()
            .ok_or_else(|| {
                ToolError::Argument(
                    "no vault is open — ask the user to open a vault in the Notes tab".into(),
                )
            })?;

        // Try semantic first.
        let semantic = semantic_query(&app, &vault, &query, limit)
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!(e)))?;
        if !semantic.is_empty() {
            let results: Vec<_> = semantic
                .into_iter()
                .map(|h| json!({ "path": h.path, "snippet": h.snippet }))
                .collect();
            return Ok(json!({
                "vault": vault,
                "mode": "semantic",
                "results": results,
            }));
        }

        // Fall back to FTS5.
        let fts = vault_search_impl(&app, &vault, &query, limit)
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!(e)))?;
        let results: Vec<_> = fts
            .into_iter()
            .map(|h| json!({ "path": h.path, "snippet": h.snippet }))
            .collect();
        Ok(json!({
            "vault": vault,
            "mode": "fulltext",
            "results": results,
        }))
    }
}
