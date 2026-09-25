//! Convert Markdown text into [`ratatui_core::text::Text`] for display in a TUI.
//!
//! # Overview
//!
//! This crate parses Markdown with [`pulldown_cmark`] and maps every element to
//! styled [`ratatui_core::text::Span`]s and [`ratatui_core::text::Line`]s,
//! producing a self-contained [`ratatui_core::text::Text<'static>`] ready to
//! hand to a `ratatui` `Paragraph` widget (or any other widget that accepts
//! `Text`).
//!
//! # Quick start
//!
//! ```rust
//! use the_other_tui_markdown::into_text;
//!
//! let text = into_text("# Hello\n\nSome **bold** and _italic_ text.");
//! // `text` is a `ratatui_core::text::Text<'static>` ready to render.
//! ```
//!
//! # Theming
//!
//! Every Markdown element has a corresponding [`Style`] field on [`Theme`].
//! All defaults are based on the 16-colour ANSI palette (no true-colour
//! required). Override what you need:
//!
//! ```rust
//! use the_other_tui_markdown::{RendererBuilder, Theme, into_text_with_renderer};
//! use ratatui_core::style::{Color, Modifier, Style};
//!
//! let mut theme = Theme::default();
//! theme.h1 = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
//!
//! let renderer = RendererBuilder::new().with_theme(theme).build();
//! let text = into_text_with_renderer("# My heading", &renderer);
//! ```
//!
//! # Per-element rendering customization
//!
//! [`RendererBuilder`] lets you replace the default rendering for any element
//! type with a custom closure. This is especially useful for links — you can
//! map each URL to a short hint number so the user can open it by typing a
//! number:
//!
//! ```rust
//! use the_other_tui_markdown::{RendererBuilder, into_text_with_renderer};
//! use ratatui_core::text::Span;
//! use std::collections::HashMap;
//! use std::sync::{Arc, Mutex};
//! use std::sync::atomic::{AtomicU32, Ordering};
//!
//! // A shared hint table: url → number.
//! let hints: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
//! let counter = Arc::new(AtomicU32::new(0));
//!
//! let hints_clone = Arc::clone(&hints);
//! let counter_clone = Arc::clone(&counter);
//! let renderer = RendererBuilder::new()
//!     .with_link(move |alt, url| {
//!         let mut map = hints_clone.lock().unwrap();
//!         let n = *map.entry(url.to_owned()).or_insert_with(|| {
//!             counter_clone.fetch_add(1, Ordering::Relaxed) + 1
//!         });
//!         vec![Span::raw(format!("[{}]({})", alt, n))]
//!     })
//!     .build();
//!
//! let text = into_text_with_renderer(
//!     "Visit [the docs](https://docs.rs) and [crates.io](https://crates.io).",
//!     &renderer,
//! );
//! // Renders as: "Visit [the docs](1) and [crates.io](2)."
//! // `hints` now maps "https://docs.rs" → 1, "https://crates.io" → 2.
//! ```
//!
//! # Mouse support (hit-testing)
//!
//! The `into_document*` family works exactly like `into_text*`, but returns a
//! [`MarkdownDocument`] that remembers which Markdown element produced every
//! span. Given a mouse event's screen coordinates you can then ask what is
//! under the cursor:
//!
//! ```rust
//! use the_other_tui_markdown::{ElementKind, ParagraphView, into_document};
//! use ratatui_core::layout::Rect;
//!
//! let doc = into_document("Click [a link](https://example.com)!");
//!
//! // Describe how the `Paragraph` renders the text (area, scroll, wrap):
//! let view = ParagraphView::new(Rect::new(0, 0, 40, 10)).wrap(true);
//!
//! // A left click at screen (row 0, column 7):
//! let hit = doc.element_at_screen(0, 7, &view).expect("hit");
//! assert!(matches!(&hit.kind, ElementKind::Link { url, .. } if url == "https://example.com"));
//! ```
//!
//! [`MarkdownDocument::element_at_screen`] accounts for the widget area,
//! vertical/horizontal scroll, word wrapping (`Wrap { trim }`), and text
//! alignment. [`MarkdownDocument::element_at`] is the simpler Text-space
//! variant (line index + display column) when no wrapping is involved. Each
//! hit also links to its enclosing elements via
//! [`MarkdownDocument::ancestors_of`], so you can tell e.g. that a link sits
//! inside a list item inside a block quote.
//!
//! See `examples/mouse.rs` for a complete interactive demo.
//!
//! # Supported Markdown elements
//!
//! | Element | Default output |
//! |---|---|
//! | `# H1` … `###### H6` | `# text` prefix + heading style |
//! | `**bold**` / `__bold__` | [`Modifier::BOLD`] |
//! | `*italic*` / `_italic_` | [`Modifier::ITALIC`] |
//! | `~~strikethrough~~` | [`Modifier::CROSSED_OUT`] |
//! | `^superscript^` | [`Modifier::DIM`] |
//! | `~subscript~` | [`Modifier::DIM`] |
//! | `` `inline code` `` | yellow foreground |
//! | ` ```lang … ``` ` | `[lang]` label + yellow foreground |
//! | `[alt](url)` | `[alt](url)` in link style |
//! | `![alt](url)` | `🖼 alt(url)` in image style |
//! | `> quote` | `▌ text` with quote style (GFM alerts supported) |
//! | `- item` / `1. item` | `• item` / `1. item` with nesting |
//! | `- [x] done` | `[x] ` / `[ ] ` prefix |
//! | `---` (rule) | `────────────────────────────────────────` |
//! | Tables | aligned columns with `─┼─` separator |
//! | `[^1]` footnote refs | `[^1]` in dim style; defs appended at end |
//! | Inline / block HTML | verbatim, unstyled |
//! | Inline / display math | verbatim content in math style |
//! | Definition lists | term in bold, definition indented |

pub mod converter;
pub mod document;
pub mod renderer;
pub mod theme;
mod wrap;

pub use document::{
    Element, ElementId, ElementKind, MarkdownDocument, ParagraphView, QuoteKind,
};
pub use renderer::{
    CodeBlockFn, FootnoteRefFn, HeadingFn, ImageFn, InlineCodeFn, LinkFn, Renderer,
    RendererBuilder, RuleFn,
};
pub use theme::Theme;

use ratatui_core::text::Text;

// ── Public API ────────────────────────────────────────────────────────────────

/// Convert Markdown to [`Text`] using the default [`Theme`] and no custom
/// element renderers.
///
/// Equivalent to `into_text_with_renderer(markdown, &RendererBuilder::new().build())`.
///
/// ```rust
/// use the_other_tui_markdown::into_text;
///
/// let text = into_text("**Hello**, _world_!");
/// assert!(!text.lines.is_empty());
/// ```
pub fn into_text(markdown: &str) -> Text<'static> {
    into_document(markdown).into_text()
}

/// Convert Markdown to [`Text`] using the default [`Theme`] and a custom
/// theme override, but no custom element renderers.
///
/// This is a convenience shorthand for:
/// ```rust,ignore
/// RendererBuilder::new().with_theme(theme).build()
/// ```
///
/// ```rust
/// use the_other_tui_markdown::{Theme, into_text_with_theme};
/// use ratatui_core::style::{Color, Style};
///
/// let mut theme = Theme::default();
/// theme.h1 = Style::new().fg(Color::Red);
///
/// let text = into_text_with_theme("# Red heading", theme);
/// ```
pub fn into_text_with_theme(markdown: &str, theme: Theme) -> Text<'static> {
    into_document_with_theme(markdown, theme).into_text()
}

/// Convert Markdown to [`Text`] using a fully configured [`Renderer`].
///
/// This is the primary entry point when you need custom per-element rendering.
///
/// ```rust
/// use the_other_tui_markdown::{RendererBuilder, into_text_with_renderer};
/// use ratatui_core::text::Span;
///
/// let renderer = RendererBuilder::new()
///     .with_link(|alt, url| vec![Span::raw(format!("[{}]({})", alt, url))])
///     .build();
///
/// let text = into_text_with_renderer("[example](https://example.com)", &renderer);
/// assert!(!text.lines.is_empty());
/// ```
pub fn into_text_with_renderer(markdown: &str, renderer: &Renderer) -> Text<'static> {
    into_document_with_renderer(markdown, renderer).into_text()
}

/// Convert Markdown to a [`MarkdownDocument`] using the default [`Theme`] and
/// no custom element renderers.
///
/// A [`MarkdownDocument`] contains the rendered [`Text`] plus the element
/// annotations needed for hit-testing (see
/// [`MarkdownDocument::element_at_screen`]).
///
/// ```rust
/// use the_other_tui_markdown::{ElementKind, into_document};
///
/// let doc = into_document("A [link](https://example.com).");
/// let hit = doc.element_at(0, 2).expect("hit");
/// assert!(matches!(hit.kind, ElementKind::Link { .. }));
/// ```
pub fn into_document(markdown: &str) -> MarkdownDocument {
    let renderer = RendererBuilder::new().build();
    into_document_with_renderer(markdown, &renderer)
}

/// Convert Markdown to a [`MarkdownDocument`] using a custom [`Theme`].
pub fn into_document_with_theme(markdown: &str, theme: Theme) -> MarkdownDocument {
    let renderer = RendererBuilder::new().with_theme(theme).build();
    into_document_with_renderer(markdown, &renderer)
}

/// Convert Markdown to a [`MarkdownDocument`] using a fully configured
/// [`Renderer`].
///
/// Spans and lines produced by custom `with_*` renderers are annotated with
/// the element they were invoked for, so hit-testing keeps working with
/// custom rendering (e.g. links replaced by hint numbers stay clickable).
pub fn into_document_with_renderer(markdown: &str, renderer: &Renderer) -> MarkdownDocument {
    let mut conv = converter::Converter::new(renderer);
    conv.convert(markdown)
}
