//! Per-element rendering customization.
//!
//! [`RendererBuilder`] lets you replace the default rendering for any Markdown
//! element type with a custom closure. Call [`RendererBuilder::build`] to
//! produce a [`Renderer`] that can be passed to
//! [`crate::into_text_with_renderer`].
//!
//! # Quick start
//!
//! ```rust
//! use the_other_tui_markdown::{RendererBuilder, into_text_with_renderer};
//! use ratatui_core::text::Span;
//!
//! // Replace link rendering: show hint number instead of full URL.
//! let hints: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
//! let renderer = RendererBuilder::new()
//!     .with_link(move |alt, url| {
//!         let hint = hints.get(url).map(|n| n.to_string()).unwrap_or_else(|| url.to_owned());
//!         vec![Span::raw(format!("[{}]({})", alt, hint))]
//!     })
//!     .build();
//!
//! let text = into_text_with_renderer("See [the docs](https://example.com).", &renderer);
//! ```

use ratatui_core::text::{Line, Span};

use crate::Theme;

// ── Type aliases for the renderer callbacks ──────────────────────────────────

/// Renders a link into a sequence of [`Span`]s.
///
/// Arguments: `alt_text`, `url`.
pub type LinkFn = dyn Fn(&str, &str) -> Vec<Span<'static>> + Send + Sync;

/// Renders an image into a sequence of [`Span`]s.
///
/// Arguments: `alt_text`, `url`.
pub type ImageFn = dyn Fn(&str, &str) -> Vec<Span<'static>> + Send + Sync;

/// Renders inline code into a sequence of [`Span`]s.
///
/// Argument: `code_content`.
pub type InlineCodeFn = dyn Fn(&str) -> Vec<Span<'static>> + Send + Sync;

/// Renders a fenced/indented code block into a sequence of [`Line`]s.
///
/// Arguments: `language` (empty string if none), `code_content`.
pub type CodeBlockFn = dyn Fn(&str, &str) -> Vec<Line<'static>> + Send + Sync;

/// Renders a heading into a sequence of [`Line`]s.
///
/// Arguments: `level` (1–6), `inline_spans` (the already-styled content spans).
pub type HeadingFn = dyn Fn(u8, Vec<Span<'static>>) -> Vec<Line<'static>> + Send + Sync;

/// Renders a thematic break (`---`) into a sequence of [`Line`]s.
pub type RuleFn = dyn Fn() -> Vec<Line<'static>> + Send + Sync;

/// Renders a footnote reference into a sequence of [`Span`]s.
///
/// Argument: `label` (e.g. `"1"` for `[^1]`).
pub type FootnoteRefFn = dyn Fn(&str) -> Vec<Span<'static>> + Send + Sync;

// ── Renderer ─────────────────────────────────────────────────────────────────

/// Holds the [`Theme`] and all optional per-element custom renderers.
///
/// Build one with [`RendererBuilder`].
pub struct Renderer {
    pub(crate) theme: Theme,
    pub(crate) link: Option<Box<LinkFn>>,
    pub(crate) image: Option<Box<ImageFn>>,
    pub(crate) inline_code: Option<Box<InlineCodeFn>>,
    pub(crate) code_block: Option<Box<CodeBlockFn>>,
    pub(crate) heading: Option<Box<HeadingFn>>,
    pub(crate) rule: Option<Box<RuleFn>>,
    pub(crate) footnote_ref: Option<Box<FootnoteRefFn>>,
}

impl Renderer {
    /// Returns a reference to the [`Theme`] used by this renderer.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }
}

// ── RendererBuilder ──────────────────────────────────────────────────────────

/// Builder for [`Renderer`].
///
/// All setters are optional — unset elements use built-in defaults.
/// Use [`RendererBuilder::with_theme`] to customize colours/styles, and the
/// `with_*` element setters to replace the default rendering logic entirely.
pub struct RendererBuilder {
    theme: Theme,
    link: Option<Box<LinkFn>>,
    image: Option<Box<ImageFn>>,
    inline_code: Option<Box<InlineCodeFn>>,
    code_block: Option<Box<CodeBlockFn>>,
    heading: Option<Box<HeadingFn>>,
    rule: Option<Box<RuleFn>>,
    footnote_ref: Option<Box<FootnoteRefFn>>,
}

impl Default for RendererBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererBuilder {
    /// Create a new builder with the default [`Theme`] and no custom renderers.
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
            link: None,
            image: None,
            inline_code: None,
            code_block: None,
            heading: None,
            rule: None,
            footnote_ref: None,
        }
    }

    /// Override the [`Theme`] used for default rendering.
    ///
    /// Has no effect for elements whose rendering is overridden by a custom
    /// `with_*` closure (those closures receive no theme — they own the full
    /// output).
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Override link rendering.
    ///
    /// The closure receives `(alt_text, url)` and must return a `Vec<Span<'static>>`.
    ///
    /// # Example — show hint numbers instead of full URLs
    ///
    /// ```rust
    /// use the_other_tui_markdown::RendererBuilder;
    /// use ratatui_core::text::Span;
    ///
    /// let renderer = RendererBuilder::new()
    ///     .with_link(|alt, url| {
    ///         // In a real app you'd look `url` up in a hint table.
    ///         vec![Span::raw(format!("[{}](→{})", alt, url))]
    ///     })
    ///     .build();
    /// ```
    pub fn with_link(
        mut self,
        f: impl Fn(&str, &str) -> Vec<Span<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.link = Some(Box::new(f));
        self
    }

    /// Override image rendering.
    ///
    /// The closure receives `(alt_text, url)` and must return a `Vec<Span<'static>>`.
    pub fn with_image(
        mut self,
        f: impl Fn(&str, &str) -> Vec<Span<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.image = Some(Box::new(f));
        self
    }

    /// Override inline-code rendering.
    ///
    /// The closure receives the raw code content and must return a
    /// `Vec<Span<'static>>`.
    pub fn with_inline_code(
        mut self,
        f: impl Fn(&str) -> Vec<Span<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.inline_code = Some(Box::new(f));
        self
    }

    /// Override fenced/indented code-block rendering.
    ///
    /// The closure receives `(language, content)` — `language` is an empty
    /// string when no language is specified — and must return a
    /// `Vec<Line<'static>>`.
    pub fn with_code_block(
        mut self,
        f: impl Fn(&str, &str) -> Vec<Line<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.code_block = Some(Box::new(f));
        self
    }

    /// Override heading rendering.
    ///
    /// The closure receives `(level, inline_spans)` where `level` is 1–6 and
    /// `inline_spans` are the already-styled content spans, and must return a
    /// `Vec<Line<'static>>`.
    pub fn with_heading(
        mut self,
        f: impl Fn(u8, Vec<Span<'static>>) -> Vec<Line<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.heading = Some(Box::new(f));
        self
    }

    /// Override thematic-break rendering.
    ///
    /// The closure takes no arguments and must return a `Vec<Line<'static>>`.
    pub fn with_rule(
        mut self,
        f: impl Fn() -> Vec<Line<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.rule = Some(Box::new(f));
        self
    }

    /// Override footnote-reference rendering.
    ///
    /// The closure receives the footnote label (e.g. `"1"` for `[^1]`) and
    /// must return a `Vec<Span<'static>>`.
    pub fn with_footnote_ref(
        mut self,
        f: impl Fn(&str) -> Vec<Span<'static>> + Send + Sync + 'static,
    ) -> Self {
        self.footnote_ref = Some(Box::new(f));
        self
    }

    /// Consume the builder and produce a [`Renderer`].
    pub fn build(self) -> Renderer {
        Renderer {
            theme: self.theme,
            link: self.link,
            image: self.image,
            inline_code: self.inline_code,
            code_block: self.code_block,
            heading: self.heading,
            rule: self.rule,
            footnote_ref: self.footnote_ref,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_core::text::Span;

    #[test]
    fn builder_default_has_no_custom_renderers() {
        let r = RendererBuilder::new().build();
        assert!(r.link.is_none());
        assert!(r.image.is_none());
        assert!(r.inline_code.is_none());
        assert!(r.code_block.is_none());
        assert!(r.heading.is_none());
        assert!(r.rule.is_none());
        assert!(r.footnote_ref.is_none());
    }

    #[test]
    fn builder_with_link_stores_closure() {
        let r = RendererBuilder::new()
            .with_link(|alt, _url| vec![Span::raw(alt.to_owned())])
            .build();
        assert!(r.link.is_some());
        let spans = r.link.as_ref().unwrap()("hello", "http://x");
        assert_eq!(spans[0].content, "hello");
    }

    #[test]
    fn builder_with_image_stores_closure() {
        let r = RendererBuilder::new()
            .with_image(|alt, url| vec![Span::raw(format!("IMG:{alt}@{url}"))])
            .build();
        assert!(r.image.is_some());
        let spans = r.image.as_ref().unwrap()("cat", "cat.png");
        assert_eq!(spans[0].content, "IMG:cat@cat.png");
    }

    #[test]
    fn builder_with_inline_code_stores_closure() {
        let r = RendererBuilder::new()
            .with_inline_code(|code| vec![Span::raw(format!("`{code}`"))])
            .build();
        let spans = r.inline_code.as_ref().unwrap()("foo");
        assert_eq!(spans[0].content, "`foo`");
    }

    #[test]
    fn builder_with_code_block_stores_closure() {
        let r = RendererBuilder::new()
            .with_code_block(|lang, content| {
                vec![Line::raw(format!("{lang}: {content}"))]
            })
            .build();
        let lines = r.code_block.as_ref().unwrap()("rust", "fn main() {}");
        assert_eq!(lines[0].spans[0].content, "rust: fn main() {}");
    }

    #[test]
    fn builder_with_rule_stores_closure() {
        let r = RendererBuilder::new()
            .with_rule(|| vec![Line::raw("---")])
            .build();
        let lines = r.rule.as_ref().unwrap()();
        assert_eq!(lines[0].spans[0].content, "---");
    }

    #[test]
    fn builder_with_footnote_ref_stores_closure() {
        let r = RendererBuilder::new()
            .with_footnote_ref(|label| vec![Span::raw(format!("[^{label}]"))])
            .build();
        let spans = r.footnote_ref.as_ref().unwrap()("42");
        assert_eq!(spans[0].content, "[^42]");
    }

    #[test]
    fn builder_with_theme_replaces_theme() {
        use crate::Theme;
        use ratatui_core::style::{Color, Style};
        let mut custom = Theme::default();
        custom.h1 = Style::new().fg(Color::Red);
        let r = RendererBuilder::new().with_theme(custom).build();
        assert_eq!(r.theme.h1.fg, Some(Color::Red));
    }

    #[test]
    fn builder_default_impl_same_as_new() {
        let r = RendererBuilder::default().build();
        assert!(r.link.is_none());
    }
}
