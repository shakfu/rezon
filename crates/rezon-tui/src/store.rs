// Conversation model + on-disk persistence.
//
// One JSON file under `directories::ProjectDirs("com", "rezon",
// "rezon-tui")::config_dir()/conversations.json`, schema-versioned so
// future format bumps can migrate or reject older files.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use rezon_core::llm::ChatMsg;
use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const UNTITLED: &str = "untitled";
const TITLE_MAX_CHARS: usize = 48;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub messages: Vec<ChatMsg>,
}

impl Conversation {
    pub fn new(system: String) -> Self {
        let mut messages = Vec::new();
        if !system.trim().is_empty() {
            messages.push(ChatMsg {
                role: "system".to_string(),
                content: system.clone(),
                ..ChatMsg::default()
            });
        }
        Self {
            id: next_id(),
            title: UNTITLED.to_string(),
            system,
            messages,
        }
    }

    /// Auto-title from the first user message when the title is
    /// still `untitled`. Called after every submission so the
    /// sidebar reads sensibly without forcing a rename.
    pub fn maybe_auto_title(&mut self) {
        if self.title.trim() != UNTITLED && !self.title.trim().is_empty() {
            return;
        }
        let first_user = self
            .messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str());
        let Some(text) = first_user else { return };
        let one_line: String = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(TITLE_MAX_CHARS)
            .collect();
        let trimmed = one_line.trim();
        if !trimmed.is_empty() {
            self.title = trimmed.to_string();
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    #[serde(default)]
    active_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_vault: Option<String>,
    /// Names of tools the user has disabled. Applied via
    /// `ToolRegistry::without` before each agent run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    disabled_tools: Vec<String>,
    conversations: Vec<Conversation>,
}

pub struct Store {
    pub path: PathBuf,
    pub conversations: Vec<Conversation>,
    pub active: usize,
    /// Path of the vault directory the user most recently opened.
    /// Auto-opened on next launch.
    pub active_vault: Option<String>,
    /// User-disabled tools. Applied to every agent run's registry.
    pub disabled_tools: Vec<String>,
}

impl Store {
    /// Load from disk, or build a fresh store with one empty
    /// conversation if the file is absent / malformed. Returns the
    /// `Store` and a flag indicating whether the file already
    /// existed (used by the UI to decide what to show in the status
    /// line on first launch).
    pub fn load_or_new(default_system: &str) -> Result<(Self, bool)> {
        let path = config_path()?;
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(file) = serde_json::from_slice::<StoreFile>(&bytes) {
                if !file.conversations.is_empty() {
                    let active = file
                        .active_id
                        .and_then(|id| file.conversations.iter().position(|c| c.id == id))
                        .unwrap_or(0);
                    return Ok((
                        Store {
                            path,
                            conversations: file.conversations,
                            active,
                            active_vault: file.active_vault,
                            disabled_tools: file.disabled_tools,
                        },
                        true,
                    ));
                }
            }
        }
        let convo = Conversation::new(default_system.to_string());
        Ok((
            Store {
                path,
                conversations: vec![convo],
                active: 0,
                active_vault: None,
                disabled_tools: Vec::new(),
            },
            false,
        ))
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        let file = StoreFile {
            version: SCHEMA_VERSION,
            active_id: self.conversations.get(self.active).map(|c| c.id.clone()),
            active_vault: self.active_vault.clone(),
            disabled_tools: self.disabled_tools.clone(),
            conversations: self.conversations.clone(),
        };
        let json = serde_json::to_vec_pretty(&file).context("serialize conversations")?;
        std::fs::write(&self.path, json)
            .with_context(|| format!("write {}", self.path.display()))?;
        Ok(())
    }

    pub fn active(&self) -> &Conversation {
        &self.conversations[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Conversation {
        &mut self.conversations[self.active]
    }

    pub fn select(&mut self, index: usize) {
        if index < self.conversations.len() {
            self.active = index;
        }
    }

    pub fn new_conversation(&mut self, default_system: &str) {
        self.conversations
            .push(Conversation::new(default_system.to_string()));
        self.active = self.conversations.len() - 1;
    }

    /// Delete the active conversation. Never leaves the store
    /// empty — the last surviving conversation is replaced with a
    /// fresh blank one.
    pub fn delete_active(&mut self, default_system: &str) {
        if self.conversations.len() == 1 {
            self.conversations[0] = Conversation::new(default_system.to_string());
            self.active = 0;
            return;
        }
        let i = self.active;
        self.conversations.remove(i);
        if self.active >= self.conversations.len() {
            self.active = self.conversations.len() - 1;
        }
    }

    pub fn rename_active(&mut self, title: String) {
        let t = title.trim();
        if t.is_empty() {
            return;
        }
        self.conversations[self.active].title = t.to_string();
    }
}

fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "rezon", "rezon-tui")
        .context("could not resolve user config dir")?;
    Ok(dirs.config_dir().join("conversations.json"))
}

fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("conv-{now}-{c}")
}
