//! # link_hints
//!
//! Demonstrates the flagship customization use-case: replace each link's URL
//! with a short numeric hint so the user can "open" it by typing its number.
//!
//! This is the pattern you would use in a TUI browser, a document viewer, or
//! any app that wants keyboard-driven link navigation.
//!
//! ## How it works
//!
//! 1. A shared `HashMap<url, hint_number>` is populated lazily as links are
//!    encountered during rendering.
//! 2. The `with_link` closure replaces `[alt](url)` with `[alt](N)`.
//! 3. The app maintains a reverse map `hint_number → url` so that when the
//!    user types a number the URL can be looked up instantly.
//!
//! Controls:
//!   `j` / `↓`  scroll down
//!   `k` / `↑`  scroll up
//!   `0`–`9`    type a hint number then Enter to "open" the link
//!   `q` / Esc  quit
//!
//! Run with:
//!     cargo run --example link_hints

use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

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
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use the_other_tui_markdown::{RendererBuilder, into_text_with_renderer};

const MARKDOWN: &str = r#"
# Link hints demo

This document contains several links. Each one is replaced with a short
numeric hint — press the hint number to "open" the link.

## Rust ecosystem

- The [Rust book](https://doc.rust-lang.org/book/) is the best starting point.
- [crates.io](https://crates.io) hosts all published crates.
- [docs.rs](https://docs.rs) generates documentation automatically.
- [The Rustonomicon](https://doc.rust-lang.org/nomicon/) covers unsafe Rust.

## TUI libraries

| Library                                | Purpose                          |
|----------------------------------------|----------------------------------|
| [ratatui](https://ratatui.rs)          | Terminal UI framework            |
| [crossterm](https://github.com/crossterm-rs/crossterm) | Cross-platform terminal control |
| [tui-input](https://github.com/sayanarijit/tui-input) | Text input widget               |

## More links

See [the other tui markdown](https://github.com/example/the-other-tui-markdown)
for the source of this example.

Also check the [ratatui showcase](https://github.com/ratatui/ratatui/wiki/Showcase)
for inspiration.
"#;

struct App {
    /// The rendered `Text` with hint numbers substituted for URLs.
    text: ratatui::text::Text<'static>,
    /// Reverse map: hint number → URL.
    hints: HashMap<u32, String>,
    /// Vertical scroll offset.
    scroll: u16,
    /// Digit(s) typed so far (building a hint number).
    input: String,
    /// Last "opened" URL (shown in the status bar).
    last_opened: Option<String>,
}

impl App {
    fn new() -> Self {
        // Forward map built during rendering: url → hint number.
        let url_to_hint: Arc<Mutex<HashMap<String, u32>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));

        let url_map = Arc::clone(&url_to_hint);
        let ctr = Arc::clone(&counter);

        let renderer = RendererBuilder::new()
            .with_link(move |alt, url| {
                let mut map = url_map.lock().unwrap();
                let n = *map.entry(url.to_owned()).or_insert_with(|| {
                    ctr.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
                });
                vec![
                    Span::styled(
                        format!("[{}]", alt),
                        Style::new()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::UNDERLINED),
                    ),
                    Span::styled(
                        format!("({})", n),
                        Style::new()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]
            })
            .build();

        let text = into_text_with_renderer(MARKDOWN, &renderer);

        // Build the reverse map: hint number → url.
        let forward = url_to_hint.lock().unwrap();
        let hints: HashMap<u32, String> =
            forward.iter().map(|(url, n)| (*n, url.clone())).collect();

        Self {
            text,
            hints,
            scroll: 0,
            input: String::new(),
            last_opened: None,
        }
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// Try to interpret the accumulated `input` as a hint number and open it.
    fn try_open(&mut self) {
        if let Ok(n) = self.input.parse::<u32>() {
            if let Some(url) = self.hints.get(&n) {
                self.last_opened = Some(url.clone());
            } else {
                self.last_opened = Some(format!("(no link with hint {})", n));
            }
        }
        self.input.clear();
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

            // Outer layout: main content + status bar.
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(3)])
                .split(area);

            // ── Main content ──────────────────────────────────────────────────
            let block = Block::default()
                .title(" link_hints — type a hint number + Enter to open (q to quit) ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray));
            let inner = block.inner(chunks[0]);
            frame.render_widget(block, chunks[0]);

            let para = Paragraph::new(app.text.clone())
                .wrap(Wrap { trim: false })
                .scroll((app.scroll, 0));
            frame.render_widget(para, inner);

            // ── Status bar ────────────────────────────────────────────────────
            let status_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray));
            let status_inner = status_block.inner(chunks[1]);
            frame.render_widget(status_block, chunks[1]);

            let hint_count = app.hints.len();
            let input_display = if app.input.is_empty() {
                String::new()
            } else {
                format!(" typing: {}", app.input)
            };
            let opened_display = match &app.last_opened {
                Some(url) => format!("  →  opened: {}", url),
                None => String::new(),
            };

            let status = Line::from(vec![
                Span::styled(
                    format!(" {} links found", hint_count),
                    Style::new().fg(Color::DarkGray),
                ),
                Span::styled(input_display, Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(opened_display, Style::new().fg(Color::Green)),
            ]);
            let status_para = Paragraph::new(status);
            frame.render_widget(status_para, status_inner);
        })?;

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
                        KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            app.input.push(c);
                        }
                        KeyCode::Enter => app.try_open(),
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
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
