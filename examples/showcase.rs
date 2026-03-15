//! # showcase
//!
//! A scrollable TUI that renders a comprehensive Markdown document containing
//! every element type supported by the library.  Use this to see all default
//! styles side-by-side and to verify that nothing unexpected appears.
//!
//! Controls:
//!   `j` / `↓` / `PageDown`  scroll down
//!   `k` / `↑` / `PageUp`    scroll up
//!   `g`                      go to top
//!   `G`                      go to bottom
//!   `q` / `Esc`              quit
//!
//! Run with:
//!     cargo run --example showcase

use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};
use the_other_tui_markdown::into_text;

// A comprehensive document that exercises every supported element type.
const MARKDOWN: &str = r#"
# Showcase — all supported elements

This document is rendered by **the-other-tui-markdown**.
Scroll with `j`/`k` or the arrow keys; press `q` to quit.

---

## Inline formatting

Normal paragraph text.
**Bold text** and __also bold__.
_Italic text_ and *also italic*.
~~Strikethrough text~~.
`Inline code snippet`.
Nested: **bold and _bold-italic_** back to bold.

Superscript: footnote ^ref^ and subscript: H ~2~ O (space-separated due to parser rules).

---

## Headings (all six levels)

# H1 — the largest
## H2 — section
### H3 — subsection
#### H4 — sub-subsection
##### H5 — minor
###### H6 — the smallest

---

## Links and images

A plain link: [ratatui.rs](https://ratatui.rs).
A link with a long URL: [The Rust Programming Language](https://doc.rust-lang.org/book/).

An image: ![Ferris the crab](https://rustacean.net/assets/rustacean-flat-happy.png)

---

## Block quotes

A plain block quote:

> "Programs must be written for people to read, and only incidentally for
> machines to execute."
> — Harold Abelson

A GFM alert note:

> [!NOTE]
> This is a note alert.  Use it to call out useful information.

A GFM warning:

> [!WARNING]
> This operation is irreversible.

---

## Lists

### Unordered

- First item
- Second item
  - Nested A
  - Nested B
    - Doubly nested
- Third item

### Ordered

1. Alpha
2. Beta
3. Gamma
   1. Gamma-one
   2. Gamma-two

### Task list

- [x] Implement `Theme`
- [x] Implement `RendererBuilder`
- [x] Implement table rendering
- [ ] Publish to crates.io

---

## Tables

| Element          | Default style                     | Customisable? |
|------------------|-----------------------------------|---------------|
| H1               | Cyan + bold                       | Yes           |
| H2               | Cyan + bold + underline           | Yes           |
| Bold             | Bold modifier                     | No (use theme)|
| Italic           | Italic modifier                   | No (use theme)|
| `Inline code`    | Yellow foreground                 | Yes           |
| Link             | Blue + underline                  | Yes           |
| Image            | Cyan + underline                  | Yes           |
| Code block       | Yellow foreground                 | Yes           |
| Block quote      | Gray + italic                     | No (use theme)|
| Thematic break   | `─` × 40, dark gray               | Yes           |
| Footnote ref     | `[^n]`, dim dark gray             | Yes           |

---

## Code blocks

A fenced block without a language tag:

```
fn main() {
    println!("Hello, world!");
}
```

A Rust block with a language label:

```rust
use the_other_tui_markdown::{RendererBuilder, into_text_with_renderer};
use ratatui_core::text::Span;

let renderer = RendererBuilder::new()
    .with_link(|alt, url| {
        vec![Span::raw(format!("[{}]({})", alt, url))]
    })
    .build();

let text = into_text_with_renderer("See [ratatui](https://ratatui.rs).", &renderer);
```

A shell snippet:

```sh
cargo run --example showcase
```

---

## Thematic breaks

Three thematic breaks follow, separated by a short paragraph each time.

First paragraph.

---

Second paragraph.

---

Third paragraph.

---

## Inline HTML

Markdown allows <em>inline HTML</em> tags; they are rendered verbatim.

A raw block:

<div>
  <p>This is a raw HTML block.</p>
</div>

---

## Footnotes

The library supports footnote references[^1] inline and collects
footnote definitions[^2] at the end of the document.

[^1]: This is the first footnote definition.
[^2]: And this is the second one.

---

## Definition list

Term one
: The first definition, describing term one in detail.

Term two
: The second definition.
: An additional definition for the same term.

---

## Math (inline and display)

Inline math: $E = mc^2$

Display math:

$$
\int_{-\infty}^{\infty} e^{-x^2} \, dx = \sqrt{\pi}
$$

---

*End of showcase document.*
"#;

struct App {
    text: ratatui::text::Text<'static>,
    scroll: u16,
    total_lines: u16,
}

impl App {
    fn new() -> Self {
        let text = into_text(MARKDOWN);
        let total_lines = text.lines.len() as u16;
        Self {
            text,
            scroll: 0,
            total_lines,
        }
    }

    fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n).min(self.total_lines);
    }

    fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    fn go_top(&mut self) {
        self.scroll = 0;
    }

    fn go_bottom(&mut self) {
        self.scroll = self.total_lines;
    }
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let viewport_height = area.height.saturating_sub(4); // borders + status

            // Outer layout: header + content + status.
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(area);

            // ── Header ────────────────────────────────────────────────────────
            let header = Paragraph::new(Span::styled(
                "  showcase — the-other-tui-markdown  │  j/k scroll  │  g/G top/bottom  │  q quit",
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            frame.render_widget(header, chunks[0]);

            // ── Content ───────────────────────────────────────────────────────
            let content_area = chunks[1];

            // Reserve the rightmost column for the scrollbar.
            let content_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(content_area);

            let block = Block::default()
                .borders(Borders::LEFT | Borders::RIGHT)
                .border_style(Style::new().fg(Color::DarkGray));
            let inner = block.inner(content_chunks[0]);
            frame.render_widget(block, content_chunks[0]);

            let para = Paragraph::new(app.text.clone())
                .wrap(Wrap { trim: false })
                .scroll((app.scroll, 0));
            frame.render_widget(para, inner);

            // Scrollbar.
            let mut scrollbar_state = ScrollbarState::new(app.total_lines as usize)
                .position(app.scroll as usize)
                .viewport_content_length(viewport_height as usize);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            frame.render_stateful_widget(scrollbar, content_chunks[1], &mut scrollbar_state);

            // ── Status bar ────────────────────────────────────────────────────
            let pct = if app.total_lines == 0 {
                100
            } else {
                (app.scroll as usize * 100 / app.total_lines as usize).min(100)
            };
            let status = Paragraph::new(Span::styled(
                format!(
                    "  line {}/{} ({}%)  │  {} element types demonstrated",
                    app.scroll + 1,
                    app.total_lines,
                    pct,
                    24,
                ),
                Style::new().fg(Color::DarkGray),
            ));
            frame.render_widget(status, chunks[2]);
        })?;

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(1),
                        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(1),
                        KeyCode::PageDown => app.scroll_down(20),
                        KeyCode::PageUp => app.scroll_up(20),
                        KeyCode::Char('g') => app.go_top(),
                        KeyCode::Char('G') => app.go_bottom(),
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
