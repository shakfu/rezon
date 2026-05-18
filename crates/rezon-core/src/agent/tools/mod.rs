// Built-in tools. Each tool lives in its own file.
//
// `register_core_tools` stacks the shell-independent tools. Tools that
// need long-lived state (e.g. `search_notes` reaching into SearchState
// + EmbedState) ship their own register helper that takes the state
// at construction time.

pub mod current_time;
pub mod file_read;
pub mod search_notes;
pub mod shell_exec;
pub mod web_fetch;

use std::sync::Arc;

use crate::agent::tool::ToolRegistry;
use crate::embed::EmbedState;
use crate::search::SearchState;

/// Register the shell-independent tools onto an existing registry.
pub fn register_core_tools(reg: &mut ToolRegistry) {
    reg.register(Arc::new(current_time::CurrentTime));
    reg.register(Arc::new(file_read::FileRead));
    reg.register(Arc::new(web_fetch::WebFetch));
    reg.register(Arc::new(shell_exec::ShellExec));
}

/// Register `search_notes`, which needs the shared SearchState +
/// EmbedState. Shells decide when (or whether) to register it.
pub fn register_search_notes(
    reg: &mut ToolRegistry,
    search: Arc<SearchState>,
    embed: Arc<EmbedState>,
) {
    reg.register(Arc::new(search_notes::SearchNotes::new(search, embed)));
}

