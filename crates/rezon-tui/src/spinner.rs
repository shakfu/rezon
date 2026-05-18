// Tiny async spinner for blocking-ish load operations.
//
// Wraps a future and renders a one-line `[frame] label …` indicator on
// the same line every 80 ms until the future resolves. On completion
// the spinner line is cleared so the caller can print its own result.
//
// No external dep: the frame set is the standard braille spinner the
// rest of the rust ecosystem uses (indicatif's default).

use std::future::Future;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use std::io::IsTerminal;

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Run `fut` while displaying a spinner labelled `label`. If stdout
/// isn't a tty the spinner is suppressed so piped runs stay clean.
pub async fn with_spinner<F, T>(label: impl Into<String>, fut: F) -> T
where
    F: Future<Output = T>,
{
    if !std::io::stdout().is_terminal() {
        return fut.await;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let label = label.into();
    let started = std::time::Instant::now();
    let spinner = tokio::spawn(async move {
        let mut i = 0usize;
        while !stop_clone.load(Ordering::Relaxed) {
            let frame = FRAMES[i % FRAMES.len()];
            let secs = started.elapsed().as_secs();
            // Scope the stdout lock so it doesn't span the await
            // below (the anstream guard is !Send, which would
            // disqualify this task from `tokio::spawn`).
            {
                let mut out = anstream::stdout().lock();
                // \r returns to column 0; \x1b[2K clears the whole
                // line. \x1b[?25l hides the cursor; we restore it on
                // stop. Trailing `(Ns)` reassures the user that
                // work is still in progress on slow loads.
                let _ = write!(
                    out,
                    "\r\x1b[2K\x1b[?25l\x1b[35m{frame}\x1b[0m {label} \x1b[2m({secs}s)\x1b[0m"
                );
                let _ = out.flush();
            }
            tokio::time::sleep(Duration::from_millis(80)).await;
            i += 1;
        }
        let mut out = anstream::stdout().lock();
        let _ = write!(out, "\r\x1b[2K\x1b[?25h");
        let _ = out.flush();
    });
    let result = fut.await;
    stop.store(true, Ordering::Relaxed);
    let _ = spinner.await;
    result
}
