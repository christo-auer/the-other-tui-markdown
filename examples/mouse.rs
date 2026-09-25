//! # mouse
//!
//! Demonstrates hit-testing: convert Markdown to a [`MarkdownDocument`],
//! render it in a `Paragraph` with wrapping, and use
//! [`MarkdownDocument::element_at_screen`] to find the Markdown element under
//! the mouse cursor.
//!
//! Controls:
//!   left click      show the element under the cursor in the status bar
//!   scroll wheel    scroll the document
//!   `q` / Esc       quit
//!
//! Run with:
//!     cargo run --example mouse

use std::io;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use the_other_tui_markdown::{
    Element, ElementKind, MarkdownDocument, ParagraphView, into_document,
};

const MARKDOWN: &str = r##"
# Mouse support demo

Click anywhere in this document to see which Markdown element is under the
cursor. The document wraps, so resize the terminal to change the layout.

## Links

- The [Rust book](https://doc.rust-lang.org/book/) is the best starting point.
- [crates.io](https://crates.io) hosts all published crates.
- [docs.rs](https://docs.rs) generates documentation automatically.

## A table

| Library  | Purpose                   |
|----------|---------------------------|
| ratatui  | Terminal UI framework     |
| crossterm| Cross-platform terminals  |

## Code

Inline code like `cargo run` and blocks:

```rust
let doc = into_document("# Hello");
```

> **Tip:** block quotes work too — click the bar on the left.

- [x] implement hit-testing
- [ ] click this task item

That's all, folks. Resize the window and watch the hits follow the wrapping.
"##;

struct App {
    doc: MarkdownDocument,
    /// Vertical scroll offset (in wrapped rows).
    scroll: u16,
    /// Description of the last clicked element.
    last_hit: Option<String>,
    /// The inner area the document paragraph is rendered into.
    area: ratatui::layout::Rect,
}

impl App {
    fn new() -> Self {
        Self {
            doc: into_document(MARKDOWN),
            scroll: 0,
            last_hit: None,
            area: ratatui::layout::Rect::default(),
        }
    }

    /// Describe an element and its ancestors for the status bar.
    fn describe(&self, element: &Element) -> String {
        let mut chain: Vec<String> = std::iter::once(describe_kind(&element.kind))
            .chain(self.doc.ancestors_of(element).map(|e| describe_kind(&e.kind)))
            .collect();
        chain.reverse();
        chain.join(" → ")
    }

    fn click(&mut self, row: u16, column: u16) {
        let view = ParagraphView::new(self.area)
            .scroll(self.scroll, 0)
            .wrap(true);
        self.last_hit = Some(match self.doc.element_at_screen(row, column, &view) {
            Some(element) => self.describe(element),
            None => "(nothing there)".to_string(),
        });
    }
}

fn describe_kind(kind: &ElementKind) -> String {
    match kind {
        ElementKind::Paragraph => "paragraph".into(),
        ElementKind::Heading { level } => format!("heading H{level}"),
        ElementKind::CodeBlock { lang } => format!("code block ({})", lang.as_deref().unwrap_or("no lang")),
        ElementKind::InlineCode => "inline code".into(),
        ElementKind::BlockQuote { kind } => format!("block quote ({kind:?})"),
        ElementKind::List { ordered } => {
            if *ordered { "ordered list".into() } else { "bullet list".into() }
        }
        ElementKind::ListItem { checked } => match checked {
            Some(c) => format!("task item (checked={c})"),
            None => "list item".into(),
        },
        ElementKind::Table => "table".into(),
        ElementKind::TableCell { row, col, header } => {
            format!("table cell r{row}c{col}{}", if *header { " (header)" } else { "" })
        }
        ElementKind::Link { url, .. } => format!("LINK → {url}"),
        ElementKind::Image { alt, url } => format!("image {alt} ({url})"),
        ElementKind::Strong => "bold".into(),
        ElementKind::Emphasis => "italic".into(),
        ElementKind::Strikethrough => "strikethrough".into(),
        ElementKind::Rule => "thematic break".into(),
        other => format!("{other:?}"),
    }
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(3)])
                .split(area);

            // ── Document ──────────────────────────────────────────────────────
            let block = Block::default()
                .title(" mouse — click to inspect, wheel to scroll (q to quit) ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray));
            let inner = block.inner(chunks[0]);
            frame.render_widget(block, chunks[0]);
            app.area = inner;

            let para = Paragraph::new(app.doc.text().clone())
                .wrap(Wrap { trim: true })
                .scroll((app.scroll, 0));
            frame.render_widget(para, inner);

            // ── Status bar ────────────────────────────────────────────────────
            let status_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray));
            let status_inner = status_block.inner(chunks[1]);
            frame.render_widget(status_block, chunks[1]);

            let status = match &app.last_hit {
                Some(hit) => format!(" last click: {hit}"),
                None => " click somewhere in the document…".to_string(),
            };
            frame.render_widget(Paragraph::new(Line::raw(status)), status_inner);
        })?;

        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                        break;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        app.click(mouse.row, mouse.column);
                    }
                    MouseEventKind::ScrollUp => {
                        app.scroll = app.scroll.saturating_sub(1);
                    }
                    MouseEventKind::ScrollDown => {
                        app.scroll = app.scroll.saturating_add(1);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    Ok(())
}
