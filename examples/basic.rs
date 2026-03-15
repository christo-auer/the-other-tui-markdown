//! # basic
//!
//! The simplest possible usage: call `into_text()` and print the styled output
//! to the terminal using ANSI escape codes.
//!
//! No interactive TUI, no event loop — just a direct demonstration that the
//! `Text` produced by the library contains the right spans and styles.
//!
//! Run with:
//!     cargo run --example basic

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Text;
use the_other_tui_markdown::into_text;

// A compact markdown document that exercises the most common elements.
const MARKDOWN: &str = r##"
# The Other TUI Markdown

Convert Markdown into **ratatui** `Text` with zero boilerplate.

## Features

- **Bold**, _italic_, and ~~strikethrough~~ text
- `Inline code` with a distinct colour
- Links: [ratatui on crates.io](https://crates.io/crates/ratatui)
- Images: ![ratatui logo](https://ratatui.rs/logo.png)

## A short table

| Crate          | Purpose              |
|----------------|----------------------|
| ratatui-core   | Text / Style types   |
| pulldown-cmark | Markdown parser      |
| unicode-width  | Column width for CJK |

## Code example

```rust
use the_other_tui_markdown::into_text;

let text = into_text("# Hello\n\n**world**");
// pass `text` to ratatui's Paragraph widget
```

> **Note:** All output is `'static` — no lifetime juggling needed.

---

That's all.
"##;

fn main() {
    let text = into_text(MARKDOWN);
    print_text_ansi(&text);
}

/// Walk every [`ratatui::text::Line`] / [`ratatui::text::Span`] and emit the
/// appropriate ANSI escape codes so the styled output is visible directly in
/// any ANSI-capable terminal.
fn print_text_ansi(text: &Text) {
    for line in &text.lines {
        for span in &line.spans {
            let s = &span.style;
            print!("{}", ansi_open(s));
            print!("{}", span.content);
            print!("{}", ANSI_RESET);
        }
        println!();
    }
}

const ANSI_RESET: &str = "\x1b[0m";

fn ansi_open(style: &Style) -> String {
    let mut codes: Vec<&str> = Vec::new();

    // Foreground colour.
    let fg_code: String;
    if let Some(color) = style.fg {
        fg_code = ansi_fg(color);
        codes.push(&fg_code);
    }

    // Modifiers.
    if style.add_modifier.contains(Modifier::BOLD) {
        codes.push("\x1b[1m");
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        codes.push("\x1b[3m");
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        codes.push("\x1b[4m");
    }
    if style.add_modifier.contains(Modifier::CROSSED_OUT) {
        codes.push("\x1b[9m");
    }
    if style.add_modifier.contains(Modifier::DIM) {
        codes.push("\x1b[2m");
    }

    codes.join("")
}

fn ansi_fg(color: Color) -> String {
    match color {
        Color::Black => "\x1b[30m".into(),
        Color::Red => "\x1b[31m".into(),
        Color::Green => "\x1b[32m".into(),
        Color::Yellow => "\x1b[33m".into(),
        Color::Blue => "\x1b[34m".into(),
        Color::Magenta => "\x1b[35m".into(),
        Color::Cyan => "\x1b[36m".into(),
        Color::Gray => "\x1b[37m".into(),
        Color::DarkGray => "\x1b[90m".into(),
        Color::LightRed => "\x1b[91m".into(),
        Color::LightGreen => "\x1b[92m".into(),
        Color::LightYellow => "\x1b[93m".into(),
        Color::LightBlue => "\x1b[94m".into(),
        Color::LightMagenta => "\x1b[95m".into(),
        Color::LightCyan => "\x1b[96m".into(),
        Color::White => "\x1b[97m".into(),
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
        Color::Indexed(i) => format!("\x1b[38;5;{i}m"),
        _ => String::new(),
    }
}
