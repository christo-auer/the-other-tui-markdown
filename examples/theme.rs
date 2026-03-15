//! # theme
//!
//! Demonstrates how to replace the default colour scheme with a custom
//! [`Theme`].  This example uses a "dark ocean" palette — deep blue headings,
//! teal links, amber code — and renders the result inside a real ratatui TUI.
//!
//! Controls: `q` or `Esc` to quit.
//!
//! Run with:
//!     cargo run --example theme

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
    widgets::{Block, Borders, Paragraph, Wrap},
};
use the_other_tui_markdown::{Theme, into_text_with_theme};

const MARKDOWN: &str = r#"
# Dark Ocean Theme

A custom colour palette that replaces every default ANSI colour with
hand-picked values.

## Inline styles

Normal text, **bold ocean blue**, _italic seafoam_, ~~struck grey~~,
and `amber inline code`.

## Links & images

Visit [ratatui.rs](https://ratatui.rs) for widget docs.
The logo is at ![ratatui logo](https://ratatui.rs/logo.png).

## Block quote

> "The sea does not reward those who are too anxious, too greedy,
> or too impatient."
> — Anne Morrow Lindbergh

## Code block

```rust
fn greet(name: &str) {
    println!("Hello, {name}!");
}
```

## Lists

1. Dive in
2. Explore the depths
3. Surface safely

- Coral reefs
  - Brain coral
  - Staghorn coral
- Open ocean
"#;

/// Build the custom "dark ocean" theme.
fn ocean_theme() -> Theme {
    let mut t = Theme::default();

    // Headings: deep blue → mid blue → teal gradient.
    t.h1 = Style::new()
        .fg(Color::Rgb(100, 180, 255))
        .add_modifier(Modifier::BOLD);
    t.h2 = Style::new()
        .fg(Color::Rgb(80, 160, 220))
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    t.h3 = Style::new()
        .fg(Color::Rgb(60, 200, 200))
        .add_modifier(Modifier::BOLD);

    // Inline.
    t.strong = Style::new()
        .fg(Color::Rgb(100, 180, 255))
        .add_modifier(Modifier::BOLD);
    t.emphasis = Style::new()
        .fg(Color::Rgb(120, 220, 180))
        .add_modifier(Modifier::ITALIC);
    t.strikethrough = Style::new()
        .fg(Color::Rgb(100, 100, 130))
        .add_modifier(Modifier::CROSSED_OUT);
    t.inline_code = Style::new().fg(Color::Rgb(255, 190, 80));

    // Links & images.
    t.link = Style::new()
        .fg(Color::Rgb(80, 200, 255))
        .add_modifier(Modifier::UNDERLINED);
    t.image = Style::new()
        .fg(Color::Rgb(60, 200, 200))
        .add_modifier(Modifier::UNDERLINED);

    // Code block.
    t.code_block = Style::new().fg(Color::Rgb(255, 190, 80));
    t.code_block_lang = Style::new()
        .fg(Color::Rgb(150, 150, 180))
        .add_modifier(Modifier::ITALIC);

    // Block quote.
    t.block_quote = Style::new()
        .fg(Color::Rgb(120, 160, 200))
        .add_modifier(Modifier::ITALIC);

    // Rule & misc.
    t.rule = Style::new().fg(Color::Rgb(60, 80, 120));
    t.list_marker = Style::new().fg(Color::Rgb(80, 120, 180));

    t
}

fn main() -> io::Result<()> {
    // Set up the terminal.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let text = into_text_with_theme(MARKDOWN, ocean_theme());

    loop {
        terminal.draw(|frame| {
            let area = frame.area();

            // Outer block with a title.
            let block = Block::default()
                .title(" theme example — custom 'dark ocean' palette (q to quit) ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Rgb(60, 80, 120)));

            let inner = block.inner(area);
            frame.render_widget(block, area);

            // Two equal columns.
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(inner);

            // Left column: default theme.
            let left_block = Block::default()
                .title(" Default theme ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray));
            let left_inner = left_block.inner(cols[0]);
            frame.render_widget(left_block, cols[0]);

            let default_text = the_other_tui_markdown::into_text(MARKDOWN);
            let left_para = Paragraph::new(default_text)
                .wrap(Wrap { trim: false });
            frame.render_widget(left_para, left_inner);

            // Right column: ocean theme.
            let right_block = Block::default()
                .title(" Dark ocean theme ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Rgb(60, 80, 120)));
            let right_inner = right_block.inner(cols[1]);
            frame.render_widget(right_block, cols[1]);

            let right_para = Paragraph::new(text.clone())
                .wrap(Wrap { trim: false });
            frame.render_widget(right_para, right_inner);
        })?;

        // Event handling.
        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
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
