# the-other-tui-markdown — Project Context

## Project Overview

A Rust library that converts Markdown text into `ratatui_core::text::Text<'static>`
for rendering in terminal UIs. It is an interim replacement for `tui-markdown`.

- **Language**: Rust (edition 2024)
- **License**: GPL-3.0-or-later
- **Repository**: https://github.com/christo-auer/the-other-tui-markdown
- **Package**: `the-other-tui-markdown`

## Architecture

```
src/
├── lib.rs        Public API: into_text, into_text_with_theme, into_text_with_renderer
├── converter.rs  Event-driven Markdown → Text conversion (pulldown-cmark → ratatui)
├── renderer.rs   Renderer and RendererBuilder for per-element customization
└── theme.rs      Theme: per-element Style configuration
```

- `pulldown-cmark` parses Markdown into an event stream.
- `Converter` walks events and builds `Vec<Line<'static>>`.
- `Converter` uses `Parser::into_offset_iter()` to get source positions for
  each `Tag::Item`. When inside an ordered list, `extract_orig_list_number`
  looks back at the raw Markdown to find the original item number (preserving
  non-sequential numbering like `2.`, `4.`, `8.`). The counter in
  `BlockCtx::OrderedList(n)` is updated from the source before rendering the
  marker, then `advance_list_counter` increments for the fallback path.
- `Renderer` holds the `Theme` and optional custom element renderers.
- `Theme` maps each Markdown element to a `ratatui_core::style::Style`.
- Table cells are buffered as `Vec<Span<'static>>` so inline styles (bold,
  italic, code, links, etc.) are preserved inside tables.

## Public API

- `into_text(markdown)` — default theme, no custom renderers.
- `into_text_with_theme(markdown, theme)` — custom theme only.
- `into_text_with_renderer(markdown, renderer)` — full control.
- `RendererBuilder` — fluent builder for custom renderers.
- `TableFn` receives `Vec<Vec<Span<'static>>>` for headers and
  `Vec<Vec<Vec<Span<'static>>>>` for body rows, exposing styled cell content
  to custom table renderers.

## Build & Test

```bash
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

- All 85 unit tests and 9 doctests currently pass.
- `cargo clippy` reports only warnings (no errors).
- `cargo fmt` has been applied to the source tree.

## Dependencies

```toml
[dependencies]
pulldown-cmark = { version = "0.13", default-features = false }
ratatui-core = "0.1"
unicode-width = "0.2"

[dev-dependencies]
ratatui = "0.30"
crossterm = "0.29"
```

## Supported Markdown Elements

Headings (H1–H6), paragraphs, bold/italic/strikethrough, superscript/subscript,
inline code, fenced/indented code blocks, block quotes (incl. GFM alerts),
ordered/unordered/task lists, links, images, thematic breaks, tables, footnotes,
inline/display math, definition lists, inline/block HTML, metadata blocks.

## Development Notes

- Keep comments minimal; only add comments when meaning is not obvious from code.
- Update this file when making architectural changes.
- Examples live in `examples/` and are meant to demonstrate API usage, not
  production TUI patterns.
- `release.toml` configures `cargo-release`: version bump happens on the
  feature branch before merging to master.
- Soft and hard line breaks inside table cells are currently collapsed to a
  single space because each cell is rendered as a single terminal line.
