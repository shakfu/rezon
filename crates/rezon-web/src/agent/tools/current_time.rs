// Read-only sample tool: returns the current local time. No
// confirmation needed.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::tool::{Tool, ToolContext, ToolError};

pub struct CurrentTime;

#[async_trait]
impl Tool for CurrentTime {
    fn name(&self) -> &str {
        "current_time"
    }

    fn description(&self) -> &str {
        "Return the current local time as { hour, minute, timezone }."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn dispatch(&self, _args: Value, _ctx: &ToolContext) -> Result<Value, ToolError> {
        // Shell out to `date` for now to avoid pulling in chrono/time
        // for a stub. Real implementation will use a typed time crate.
        let out = std::process::Command::new("date")
            .arg("+%H %M %Z")
            .output()
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("invoking date: {e}")))?;
        let s = String::from_utf8_lossy(&out.stdout);
        let mut parts = s.split_whitespace();
        let hour: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        let minute: u32 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        let tz = parts.next().unwrap_or("local").to_string();
        Ok(json!({ "hour": hour, "minute": minute, "timezone": tz }))
    }
}
