// Fetch a URL via HTTP(S) GET. Confirmation-required by default —
// the user should approve the destination because the model can
// produce arbitrary URLs (potential SSRF, internal-service probes,
// data exfiltration via attacker-controlled URLs in tool args).
//
// Caps:
//   - 15s request timeout
//   - 1 MiB response body cap; oversized bodies truncated with a flag
//   - Only http/https schemes accepted

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::tool::{Tool, ToolContext, ToolError};

const MAX_BYTES: usize = 1024 * 1024;
const TIMEOUT_SECS: u64 = 15;

pub struct WebFetch;

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "HTTP(S) GET a URL and return status, content-type, and body. \
         Body is decoded as UTF-8 (lossy) and capped at 1MB."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Absolute http(s) URL."
                }
            },
            "required": ["url"]
        })
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn dispatch(&self, args: Value, _ctx: &ToolContext) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            url: String,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::Argument(format!("invalid args: {e}")))?;

        let scheme = args
            .url
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(ToolError::Argument(format!(
                "url scheme must be http or https: {}",
                args.url
            )));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .user_agent("rezo/0.1 (+https://github.com/anthropics/rezo)")
            .build()
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("build http client: {e}")))?;

        let resp = client
            .get(&args.url)
            .send()
            .await
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("get {}: {e}", args.url)))?;

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("read body: {e}")))?;
        let total = bytes.len();
        let truncated = total > MAX_BYTES;
        let slice: &[u8] = if truncated { &bytes[..MAX_BYTES] } else { &bytes };
        let body = String::from_utf8_lossy(slice).into_owned();

        Ok(json!({
            "url": final_url,
            "status": status,
            "contentType": content_type,
            "size": total,
            "truncated": truncated,
            "body": body,
        }))
    }
}
