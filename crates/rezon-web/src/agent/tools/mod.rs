// Built-in tool catalog. Each tool lives in its own file. The
// `default_registry` factory assembles the canonical set the agent
// loop is launched with.

pub mod current_time;
pub mod file_read;
pub mod search_notes;
pub mod shell_exec;
pub mod web_fetch;

use std::sync::Arc;

use crate::agent::tool::ToolRegistry;

/// Build the default registry for an agent run.
pub fn default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(current_time::CurrentTime));
    reg.register(Arc::new(file_read::FileRead));
    reg.register(Arc::new(search_notes::SearchNotes));
    reg.register(Arc::new(web_fetch::WebFetch));
    reg.register(Arc::new(shell_exec::ShellExec));
    reg
}
