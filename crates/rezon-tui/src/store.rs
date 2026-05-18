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

/// Per-conversation overrides for provider / model / agent mode /
/// reasoning visibility. `None` means "fall back to CLI defaults";
/// the REPL composes effective settings on demand. All fields are
/// optional + `#[serde(default)]` so older stores (which don't carry
/// a `settings` field at all) load cleanly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_thinking: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub messages: Vec<ChatMsg>,
    #[serde(default)]
    pub settings: ConversationSettings,
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
            settings: ConversationSettings::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Build a Store rooted at the given path (bypasses ProjectDirs)
    /// for isolated test cases.
    fn fresh_store(path: PathBuf, default_system: &str) -> Store {
        Store {
            path,
            conversations: vec![Conversation::new(default_system.to_string())],
            active: 0,
            active_vault: None,
            disabled_tools: Vec::new(),
        }
    }

    #[test]
    fn maybe_auto_title_picks_first_user_line() {
        let mut c = Conversation::new(String::new());
        assert_eq!(c.title, "untitled");
        c.messages.push(ChatMsg {
            role: "user".into(),
            content: "what is the meaning of life?\nfollow-up".into(),
            ..ChatMsg::default()
        });
        c.maybe_auto_title();
        assert_eq!(c.title, "what is the meaning of life?");
    }

    #[test]
    fn maybe_auto_title_is_idempotent_after_rename() {
        let mut c = Conversation::new(String::new());
        c.title = "explicit name".into();
        c.messages.push(ChatMsg {
            role: "user".into(),
            content: "ignored".into(),
            ..ChatMsg::default()
        });
        c.maybe_auto_title();
        assert_eq!(c.title, "explicit name");
    }

    #[test]
    fn maybe_auto_title_clamps_to_max_chars() {
        let long = "x".repeat(120);
        let mut c = Conversation::new(String::new());
        c.messages.push(ChatMsg {
            role: "user".into(),
            content: long.clone(),
            ..ChatMsg::default()
        });
        c.maybe_auto_title();
        assert!(c.title.chars().count() <= 48);
        assert!(c.title.chars().all(|ch| ch == 'x'));
    }

    #[test]
    fn save_then_reload_via_storefile_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("conv.json");
        let mut s = fresh_store(path.clone(), "you are terse");
        s.active_vault = Some("/path/to/notes".into());
        s.disabled_tools.push("shell_exec".into());
        s.active_mut().title = "after lunch".into();
        s.active_mut().messages.push(ChatMsg {
            role: "user".into(),
            content: "hi".into(),
            ..ChatMsg::default()
        });
        s.save().unwrap();
        // Re-parse via the StoreFile shape directly so we don't drag
        // ProjectDirs into the test.
        let bytes = std::fs::read(&path).unwrap();
        let file: StoreFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(file.version, SCHEMA_VERSION);
        assert_eq!(file.active_vault.as_deref(), Some("/path/to/notes"));
        assert_eq!(file.disabled_tools, vec!["shell_exec".to_string()]);
        assert_eq!(file.conversations.len(), 1);
        assert_eq!(file.conversations[0].title, "after lunch");
        assert_eq!(file.active_id.as_ref(), Some(&file.conversations[0].id));
    }

    #[test]
    fn new_conversation_appends_and_activates() {
        let dir = TempDir::new().unwrap();
        let mut s = fresh_store(dir.path().join("c.json"), "");
        assert_eq!(s.conversations.len(), 1);
        s.new_conversation("");
        assert_eq!(s.conversations.len(), 2);
        assert_eq!(s.active, 1);
    }

    #[test]
    fn delete_active_replaces_blank_when_last() {
        let dir = TempDir::new().unwrap();
        let mut s = fresh_store(dir.path().join("c.json"), "sys");
        let original_id = s.active().id.clone();
        s.delete_active("sys");
        assert_eq!(s.conversations.len(), 1);
        // The replacement is a fresh conversation with a new id.
        assert_ne!(s.active().id, original_id);
        assert_eq!(s.active, 0);
    }

    #[test]
    fn delete_active_clamps_index() {
        let dir = TempDir::new().unwrap();
        let mut s = fresh_store(dir.path().join("c.json"), "");
        s.new_conversation("");
        s.new_conversation("");
        assert_eq!(s.conversations.len(), 3);
        s.select(2);
        s.delete_active("");
        assert_eq!(s.conversations.len(), 2);
        assert_eq!(s.active, 1);
    }

    #[test]
    fn rename_empty_string_is_noop() {
        let dir = TempDir::new().unwrap();
        let mut s = fresh_store(dir.path().join("c.json"), "");
        let before = s.active().title.clone();
        s.rename_active("   ".into());
        assert_eq!(s.active().title, before);
    }
}
