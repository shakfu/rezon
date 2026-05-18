// Tiny line-based markdown -> ANSI renderer.
//
// Scope (handles the 90% of what an LLM emits in chat):
//   * `**bold**` and `*italic*` inline (left-to-right, greedy paired
//     delimiters; `_…_` deliberately ignored to avoid mangling
//     identifiers like `foo_bar`).
//   * `` `inline code` `` inline.
//   * `# / ## / ###` headings.
//   * `- foo` / `* foo` unordered list items.
//   * `1. foo` ordered list items.
//   * `> quote` blockquotes.
//   * Triple-backtick fenced code blocks (any language tag dimly
//     noted on the opening line; body printed dim, indented 2 cols).
//
// Out of scope (left as literal text): links, images, tables,
// footnotes, HTML, nested list indentation, setext headings,
// reference links, escapes.

use anstyle::{AnsiColor, Color, Style};

const fn fg(c: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(c)))
}

const S_BOLD: Style = Style::new().bold();
const S_ITALIC: Style = Style::new().italic();
const S_DIM: Style = Style::new().dimmed();
const S_CODE: Style = fg(AnsiColor::Cyan);
const S_HEADING: Style = fg(AnsiColor::Cyan).bold();

/// Render `input` as ANSI-styled text. Output ends with a trailing
/// newline so the caller's cursor lands on a fresh row after writing
/// it.
pub fn render(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 32);
    let mut in_code_block = false;
    for line in input.split('\n') {
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            if in_code_block {
                in_code_block = false;
            } else {
                in_code_block = true;
                let lang = rest.trim();
                if !lang.is_empty() {
                    out.push_str(&format!("{S_DIM}  ┄ {lang}{S_DIM:#}\n"));
                }
            }
            continue;
        }
        if in_code_block {
            out.push_str(&format!("  {S_DIM}{line}{S_DIM:#}\n"));
            continue;
        }
        if let Some(t) = line.strip_prefix("### ") {
            out.push_str(&format!("{S_BOLD}### {}{S_BOLD:#}\n", render_inline(t)));
            continue;
        }
        if let Some(t) = line.strip_prefix("## ") {
            out.push_str(&format!("{S_BOLD}## {}{S_BOLD:#}\n", render_inline(t)));
            continue;
        }
        if let Some(t) = line.strip_prefix("# ") {
            out.push_str(&format!("{S_HEADING}# {}{S_HEADING:#}\n", render_inline(t)));
            continue;
        }
        if let Some(t) = strip_unordered_list(line) {
            out.push_str(&format!("  • {}\n", render_inline(t)));
            continue;
        }
        if let Some((num, t)) = strip_ordered_list(line) {
            out.push_str(&format!("  {num}. {}\n", render_inline(t)));
            continue;
        }
        if let Some(t) = line.strip_prefix("> ") {
            out.push_str(&format!("{S_DIM}│ {}{S_DIM:#}\n", render_inline(t)));
            continue;
        }
        out.push_str(&render_inline(line));
        out.push('\n');
    }
    // `split('\n')` produces a trailing empty element when `input`
    // ended with '\n'; we wrote an extra '\n' for it above. Trim one
    // trailing newline if the original input didn't end with '\n'
    // either way — keep at most one.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn strip_unordered_list(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))
}

fn strip_ordered_list(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    let dot = t.find('.')?;
    let num = &t[..dot];
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = t.get(dot + 1..)?.strip_prefix(' ')?;
    Some((num, rest))
}

/// Apply inline markup (bold / italic / inline-code) to a single
/// line. Greedy left-to-right matching of paired delimiters; falls
/// back to literal on missing close.
fn render_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                out.push_str(&format!("{S_BOLD}"));
                out.push_str(&render_inline(&stripped[..end]));
                out.push_str(&format!("{S_BOLD:#}"));
                i += 2 + end + 2;
                continue;
            }
        }
        if let Some(stripped) = rest.strip_prefix('*') {
            if let Some(end) = stripped.find('*') {
                let inner = &stripped[..end];
                if !inner.is_empty() && !inner.starts_with(' ') && !inner.ends_with(' ') {
                    out.push_str(&format!("{S_ITALIC}"));
                    out.push_str(&render_inline(inner));
                    out.push_str(&format!("{S_ITALIC:#}"));
                    i += 1 + end + 1;
                    continue;
                }
            }
        }
        if let Some(stripped) = rest.strip_prefix('`') {
            if let Some(end) = stripped.find('`') {
                out.push_str(&format!("{S_CODE}"));
                out.push_str(&stripped[..end]);
                out.push_str(&format!("{S_CODE:#}"));
                i += 1 + end + 1;
                continue;
            }
        }
        // No marker matched; emit one UTF-8 char and advance.
        let ch = rest.chars().next().expect("non-empty rest");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Number of terminal rows the raw text consumed at the given
/// width. Used by the streaming re-render to know how far to scroll
/// the cursor back before overwriting with the formatted version.
pub fn count_rows(s: &str, width: u16) -> u16 {
    let w = width.max(1) as usize;
    let mut rows: u16 = 0;
    let mut col: usize = 0;
    for ch in s.chars() {
        if ch == '\n' {
            rows = rows.saturating_add(1);
            col = 0;
        } else {
            col += 1;
            if col > w {
                rows = rows.saturating_add(1);
                col = 1;
            }
        }
    }
    if col > 0 {
        rows = rows.saturating_add(1);
    }
    rows
}
