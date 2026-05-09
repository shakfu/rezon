// Run a shell command and return stdout / stderr / exit status.
// Confirmation-required by default — this is the canonical destructive
// tool, and the user should explicitly approve every command the
// model proposes.
//
// Caps:
//   - 60s wall-clock timeout (process is killed on overrun)
//   - 256 KiB total stdout/stderr cap each; oversize is truncated with a
//     flag so the model knows the output was clipped
//   - Runs via the user's shell ($SHELL or /bin/sh) so the command can
//     use pipes, redirects, globs, etc.
//
// Future polish (see TODO.md): stdin piping, cwd override, env
// pass-through, mid-run cancel via SIGTERM.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::agent::tool::{Tool, ToolContext, ToolError};

const TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

pub struct ShellExec;

#[async_trait]
impl Tool for ShellExec {
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Run a shell command (via $SHELL or /bin/sh) and return its stdout, \
         stderr, and exit status. 60s timeout; output capped at 256KB per \
         stream."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command line (e.g. \"ls -la ~/projects | head\")."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional absolute working directory. Defaults to the rezo process cwd."
                }
            },
            "required": ["command"]
        })
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn dispatch(&self, args: Value, _ctx: &ToolContext) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            command: String,
            #[serde(default)]
            cwd: Option<String>,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::Argument(format!("invalid args: {e}")))?;

        let shell =
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

        let mut cmd = Command::new(&shell);
        cmd.arg("-c").arg(&args.command);
        if let Some(dir) = &args.cwd {
            let p = std::path::Path::new(dir);
            if !p.is_absolute() {
                return Err(ToolError::Argument(format!(
                    "cwd must be absolute: {dir}"
                )));
            }
            cmd.current_dir(p);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        // Put the child in its own process group so a timeout kill can
        // signal the whole group (the shell + everything it spawned),
        // not just the shell. Without this, a `sh -c "sleep 120"` is
        // killed at the shell layer but `sleep` is orphaned and keeps
        // running.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("spawn {shell}: {e}")))?;

        // Take pipes once, up front. read_capped borrows them mutably;
        // child.wait() borrows child mutably. Keeping the pipes outside
        // the child means we can call child.start_kill() / child.wait()
        // independently after a timeout.
        let mut stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_truncated = false;
        let mut stderr_truncated = false;

        // Race the natural finish (all three: stdout EOF, stderr EOF, child
        // wait) against the wall-clock timeout. select! drops the losing
        // arm's future, releasing all borrows it held — so on timeout we
        // can reach back into `child` to kill it.
        let timed_out = tokio::select! {
            _ = async {
                let _ = tokio::join!(
                    read_capped(&mut stdout, &mut stdout_buf, &mut stdout_truncated),
                    read_capped(&mut stderr, &mut stderr_buf, &mut stderr_truncated),
                    child.wait(),
                );
            } => false,
            _ = tokio::time::sleep(Duration::from_secs(TIMEOUT_SECS)) => true,
        };

        if timed_out {
            // SIGKILL the entire process group so descendants of the
            // shell die too (e.g. a `sleep` started by `sh -c`). On
            // platforms without process_group support, fall back to
            // killing the immediate child.
            kill_process_tree(&mut child);
            let _ = child.wait().await;
            return Ok(json!({
                "command": args.command,
                "timedOut": true,
                "timeoutSecs": TIMEOUT_SECS,
                "stdout": String::from_utf8_lossy(&stdout_buf).into_owned(),
                "stderr": String::from_utf8_lossy(&stderr_buf).into_owned(),
                "stdoutTruncated": stdout_truncated,
                "stderrTruncated": stderr_truncated,
            }));
        }

        // Natural finish path. The wait inside select! has already been
        // resolved, but we don't capture the ExitStatus from inside join!
        // due to borrow shape — re-call wait, which is now a no-op that
        // returns the cached status.
        let status = child
            .wait()
            .await
            .map_err(|e| ToolError::Runtime(anyhow::anyhow!("wait: {e}")))?;

        Ok(json!({
            "command": args.command,
            "exitCode": status.code(),
            "success": status.success(),
            "stdout": String::from_utf8_lossy(&stdout_buf).into_owned(),
            "stderr": String::from_utf8_lossy(&stderr_buf).into_owned(),
            "stdoutTruncated": stdout_truncated,
            "stderrTruncated": stderr_truncated,
        }))
    }
}

/// SIGKILL the child's whole process group on Unix (so descendants
/// like `sleep` started by `sh -c` die too). On other platforms,
/// fall back to killing just the immediate child.
fn kill_process_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            // Negative pid = kill the whole process group whose pgid
            // equals abs(pid). Safe: just an FFI syscall, no data race.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            return;
        }
    }
    let _ = child.start_kill();
}

/// Read from a pipe into `buf` until EOF or `MAX_OUTPUT_BYTES`. After the
/// cap, drains and discards remaining bytes so the child can still
/// progress (a blocked write on a full stdout pipe would otherwise stall
/// it).
async fn read_capped<R>(reader: &mut R, buf: &mut Vec<u8>, truncated: &mut bool)
where
    R: AsyncReadExt + Unpin,
{
    let mut tmp = [0u8; 8192];
    loop {
        match reader.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < MAX_OUTPUT_BYTES {
                    let take = (MAX_OUTPUT_BYTES - buf.len()).min(n);
                    buf.extend_from_slice(&tmp[..take]);
                    if take < n {
                        *truncated = true;
                    }
                } else {
                    *truncated = true;
                }
            }
            Err(_) => break,
        }
    }
}
