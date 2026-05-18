// Inline fuzzy picker — fzf-style but embedded.
//
// Renders a query line + up to MAX_ROWS scrollable items + a status
// line *below* the current cursor position, leaving terminal
// scrollback intact (no alt-screen). Uses nucleo-matcher for ranking
// (the same matcher helix uses) and crossterm for raw-mode key
// reading + cursor moves.
//
// `pick` is synchronous and blocking: the caller is responsible for
// running it inside `tokio::task::spawn_blocking` if it must not stall
// the async runtime. For the REPL's command handlers (which already
// run after rustyline returns) blocking the main thread briefly is
// fine.

use std::io::{self, IsTerminal, Write};

use crossterm::{
    cursor::{Hide, MoveToColumn, MoveToPreviousLine, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

const MAX_ROWS: usize = 10;

/// Open the picker over `items`. Returns the index into the original
/// `items` vector of the selection, or `None` if cancelled.
/// `prompt` is shown before the query (e.g. `"conv "`).
/// `initial_query` pre-seeds the filter buffer.
pub fn pick(items: Vec<String>, prompt: &str, initial_query: &str) -> Option<usize> {
    if items.is_empty() || !io::stdout().is_terminal() {
        return None;
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut state = State {
        items,
        query: initial_query.to_string(),
        selected: 0,
        ranked: Vec::new(),
    };
    state.refilter(&mut matcher);

    let mut stdout = io::stdout();
    if enable_raw_mode().is_err() {
        return None;
    }
    let _ = execute!(stdout, Hide);

    let mut rows_drawn = 0usize;
    let _ = render(&mut stdout, &state, prompt, &mut rows_drawn);

    let result = loop {
        match event::read() {
            Ok(Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            })) => {
                // Ctrl-C / Esc cancel.
                if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                    break None;
                }
                match code {
                    KeyCode::Esc => break None,
                    KeyCode::Enter => {
                        let idx = state.ranked.get(state.selected).map(|(_, i)| *i);
                        break idx;
                    }
                    KeyCode::Up if state.selected > 0 => {
                        state.selected -= 1;
                    }
                    KeyCode::Down if state.selected + 1 < state.ranked.len() => {
                        state.selected += 1;
                    }
                    KeyCode::Backspace => {
                        state.query.pop();
                        state.refilter(&mut matcher);
                        state.selected = 0;
                    }
                    KeyCode::Char(c)
                        if !modifiers.contains(KeyModifiers::CONTROL)
                            && !modifiers.contains(KeyModifiers::ALT) =>
                    {
                        state.query.push(c);
                        state.refilter(&mut matcher);
                        state.selected = 0;
                    }
                    _ => {}
                }
                let _ = render(&mut stdout, &state, prompt, &mut rows_drawn);
            }
            Ok(_) => continue,
            Err(_) => break None,
        }
    };

    let _ = cleanup(&mut stdout, rows_drawn);
    let _ = execute!(stdout, Show);
    let _ = disable_raw_mode();
    result
}

struct State {
    items: Vec<String>,
    query: String,
    selected: usize,
    /// `(score, original_index)` sorted by score descending. Score is
    /// always 0 for the empty-query case (we just preserve order).
    ranked: Vec<(u32, usize)>,
}

impl State {
    fn refilter(&mut self, matcher: &mut Matcher) {
        if self.query.is_empty() {
            self.ranked = (0..self.items.len()).map(|i| (0, i)).collect();
            return;
        }
        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                buf.clear();
                let haystack = Utf32Str::new(s, &mut buf);
                pattern.score(haystack, matcher).map(|sc| (sc, i))
            })
            .collect();
        // Score descending, ties broken by original order (stable).
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.ranked = scored;
    }
}

fn render(
    out: &mut io::Stdout,
    state: &State,
    prompt: &str,
    rows_drawn: &mut usize,
) -> io::Result<()> {
    // Rewind to the top of the previous draw area, clear from there.
    if *rows_drawn > 0 {
        queue!(out, MoveToPreviousLine(*rows_drawn as u16))?;
    } else {
        queue!(out, MoveToColumn(0))?;
    }
    queue!(out, Clear(ClearType::FromCursorDown))?;

    let term_cols = terminal_cols();
    // Every item line is `"> " + trimmed_item` or `"  " + trimmed_item`.
    // Trim items to keep each one on a single terminal row so our
    // row counter stays accurate when re-drawing.
    let item_budget = term_cols.saturating_sub(2);

    let visible = MAX_ROWS.min(state.ranked.len());
    let scroll = if state.selected >= visible {
        state.selected - visible + 1
    } else {
        0
    };

    // Query line: dim prompt, cyan caret, then current buffer. Trim
    // the query to fit; the user's keystrokes still update the
    // underlying buffer, only the on-screen view is clipped.
    let prompt_visible = prompt.chars().count() + 2; // "<prompt>> "
    let query_budget = term_cols.saturating_sub(prompt_visible);
    let query_display = truncate(&state.query, query_budget);
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(prompt),
        ResetColor,
        SetForegroundColor(Color::Cyan),
        SetAttribute(Attribute::Bold),
        Print("> "),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(query_display),
        Print("\n"),
        MoveToColumn(0),
    )?;
    let mut count = 1usize;

    for vi in 0..visible {
        let ranked_idx = vi + scroll;
        let Some((_, item_idx)) = state.ranked.get(ranked_idx) else {
            continue;
        };
        let item = truncate(&state.items[*item_idx], item_budget);
        let selected = ranked_idx == state.selected;
        if selected {
            queue!(
                out,
                SetForegroundColor(Color::Cyan),
                SetAttribute(Attribute::Bold),
                Print("> "),
                Print(item),
                SetAttribute(Attribute::Reset),
                ResetColor,
            )?;
        } else {
            queue!(out, Print("  "), Print(item))?;
        }
        queue!(out, Print("\n"), MoveToColumn(0))?;
        count += 1;
    }

    // Status line — short, no wrapping concern.
    queue!(
        out,
        SetForegroundColor(Color::DarkGrey),
        Print(format!("  {}/{}", state.ranked.len(), state.items.len())),
        ResetColor,
    )?;
    // Status line has no trailing newline; cursor sits at end of it.

    // `rows_drawn` is "lines to move up from the cursor's current
    // resting position to reach the query line." After this render
    // the cursor sits at the *end* of the status line (no trailing
    // newline), so the distance to row 0 of the picker is `count`
    // (one per `\n` we emitted: query + each item). It is NOT
    // `count + 1` — that off-by-one made every redraw scroll the
    // picker up one row.
    *rows_drawn = count;
    out.flush()?;
    Ok(())
}

fn terminal_cols() -> usize {
    crossterm::terminal::size()
        .map(|(c, _)| c as usize)
        .unwrap_or(80)
        .max(20)
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn cleanup(out: &mut io::Stdout, rows_drawn: usize) -> io::Result<()> {
    queue!(out, MoveToColumn(0))?;
    if rows_drawn > 0 {
        queue!(out, MoveToPreviousLine(rows_drawn as u16))?;
    }
    queue!(out, Clear(ClearType::FromCursorDown))?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_budget_unchanged() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_exact_budget_unchanged() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_over_budget_appends_ellipsis() {
        let out = truncate("hello world", 5);
        // 5 chars total, last is the ellipsis.
        assert_eq!(out.chars().count(), 5);
        assert!(out.ends_with('…'));
        assert!(out.starts_with("hell"));
    }

    #[test]
    fn truncate_zero_or_one_emits_just_ellipsis() {
        assert_eq!(truncate("anything", 0), "…");
        assert_eq!(truncate("anything", 1), "…");
    }

    #[test]
    fn truncate_handles_multibyte_chars() {
        // 5 multi-byte glyphs, budget = 3 → 2 visible + ellipsis.
        let out = truncate("αβγδε", 3);
        assert_eq!(out.chars().count(), 3);
        assert!(out.ends_with('…'));
    }
}
