// Read a file from disk and return its contents as text. Defaults
// to confirmation-required: even though the action is read-only, the
// model picks the path and the user has more context about whether a
// given path is safe to expose.
//
// Caps:
//   - path must be absolute (no implicit working-directory traversal)
//   - body capped at 256 KiB; oversized files truncated with a flag

use std::path::Path;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::agent::tool::{Tool, ToolContext, ToolError};

const MAX_BYTES: usize = 256 * 1024;

pub struct FileRead;

#[async_trait]
impl Tool for FileRead {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file from an absolute path. Returns up to 256KB; \
         larger files are truncated and a `truncated: true` flag is set."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute file path (e.g. /Users/me/notes.txt)."
                }
            },
            "required": ["path"]
        })
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn dispatch(&self, args: Value, _ctx: &ToolContext) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            path: String,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::Argument(format!("invalid args: {e}")))?;
        let path = Path::new(&args.path);
        if !path.is_absolute() {
            return Err(ToolError::Argument(format!(
                "path must be absolute: {}",
                args.path
            )));
        }
        let bytes = fs::read(path)
            .await
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("read {}: {e}", args.path)))?;
        let total = bytes.len();
        let truncated = total > MAX_BYTES;
        let slice: &[u8] = if truncated { &bytes[..MAX_BYTES] } else { &bytes };
        let content = String::from_utf8_lossy(slice).into_owned();
        Ok(json!({
            "path": args.path,
            "size": total,
            "truncated": truncated,
            "content": content,
        }))
    }
}
