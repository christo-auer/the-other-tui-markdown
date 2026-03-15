//! Styling configuration for the Markdown renderer.
//!
//! [`Theme`] holds one [`Style`] per Markdown element type. All fields have
//! sane defaults based on the 16-colour ANSI palette so they work on any
//! terminal without requiring true-colour support.

use ratatui_core::style::{Color, Modifier, Style};

/// Per-element styling configuration.
///
/// Construct with [`Theme::default()`] and override individual fields as needed:
///
/// ```rust
/// use the_other_tui_markdown::Theme;
/// use ratatui_core::style::{Color, Modifier, Style};
///
/// let mut theme = Theme::default();
/// theme.h1 = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
/// ```
#[derive(Debug, Clone)]
pub struct Theme {
    /// Base style applied to the entire [`ratatui_core::text::Text`] output
    /// (e.g. a background colour for the whole block).
    pub base: Style,

    // ── Headings ────────────────────────────────────────────────────────────
    pub h1: Style,
    pub h2: Style,
    pub h3: Style,
    pub h4: Style,
    pub h5: Style,
    pub h6: Style,

    // ── Inline formatting ───────────────────────────────────────────────────
    pub strong: Style,
    pub emphasis: Style,
    pub strikethrough: Style,
    pub superscript: Style,
    pub subscript: Style,
    pub inline_code: Style,

    // ── Links & images ──────────────────────────────────────────────────────
    pub link: Style,
    pub image: Style,

    // ── Code blocks ─────────────────────────────────────────────────────────
    /// Style for fenced/indented code-block content and the `[lang]` label.
    pub code_block: Style,
    /// Style for the language label line (e.g. `[rust]`) specifically.
    /// Falls back to `code_block` if identical.
    pub code_block_lang: Style,

    // ── Block quotes ────────────────────────────────────────────────────────
    pub block_quote: Style,
    /// GFM alert variants (`> [!NOTE]`, `> [!TIP]`, …).
    pub block_quote_note: Style,
    pub block_quote_tip: Style,
    pub block_quote_warning: Style,
    pub block_quote_caution: Style,
    pub block_quote_important: Style,

    // ── Lists ───────────────────────────────────────────────────────────────
    /// Style for bullet (`•`) and number (`1.`) markers.
    pub list_marker: Style,

    // ── Tables ──────────────────────────────────────────────────────────────
    /// Style for table header cells.
    pub table_header: Style,
    /// Style for table body cells.
    pub table_cell: Style,
    /// Style for the `─┼─` separator line between header and body.
    pub table_separator: Style,

    // ── Miscellaneous ────────────────────────────────────────────────────────
    /// Style for thematic breaks (`---`).
    pub rule: Style,
    /// Style for footnote reference labels (`[^1]`).
    pub footnote_ref: Style,
    /// Style for footnote definition labels (`[1]:`) and content.
    pub footnote_def: Style,
    /// Style for inline math and display math content.
    pub math: Style,
    /// Style for raw HTML blocks and inline HTML.
    pub html: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            base: Style::default(),

            // Headings: cyan/blue palette, decreasing visual weight.
            h1: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            h2: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h3: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            h4: Style::new()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h5: Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            h6: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),

            // Inline.
            strong: Style::new().add_modifier(Modifier::BOLD),
            emphasis: Style::new().add_modifier(Modifier::ITALIC),
            strikethrough: Style::new().add_modifier(Modifier::CROSSED_OUT),
            superscript: Style::new().add_modifier(Modifier::DIM),
            subscript: Style::new().add_modifier(Modifier::DIM),
            inline_code: Style::new().fg(Color::Yellow),

            // Links & images.
            link: Style::new()
                .fg(Color::Blue)
                .add_modifier(Modifier::UNDERLINED),
            image: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),

            // Code blocks.
            code_block: Style::new().fg(Color::Yellow),
            code_block_lang: Style::new()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),

            // Block quotes.
            block_quote: Style::new()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
            block_quote_note: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::ITALIC),
            block_quote_tip: Style::new()
                .fg(Color::Green)
                .add_modifier(Modifier::ITALIC),
            block_quote_warning: Style::new()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
            block_quote_caution: Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::ITALIC),
            block_quote_important: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::ITALIC),

            // Lists.
            list_marker: Style::new().fg(Color::DarkGray),

            // Tables.
            table_header: Style::new().add_modifier(Modifier::BOLD),
            table_cell: Style::default(),
            table_separator: Style::new().fg(Color::DarkGray),

            // Misc.
            rule: Style::new().fg(Color::DarkGray),
            footnote_ref: Style::new()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
            footnote_def: Style::new().fg(Color::DarkGray),
            math: Style::new().fg(Color::Yellow),
            html: Style::new().fg(Color::DarkGray),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_h1_is_bold() {
        let t = Theme::default();
        assert!(t.h1.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn default_theme_h1_fg_is_cyan() {
        let t = Theme::default();
        assert_eq!(t.h1.fg, Some(Color::Cyan));
    }

    #[test]
    fn default_theme_emphasis_is_italic() {
        let t = Theme::default();
        assert!(t.emphasis.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn default_theme_strong_is_bold() {
        let t = Theme::default();
        assert!(t.strong.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn default_theme_strikethrough_is_crossed_out() {
        let t = Theme::default();
        assert!(t.strikethrough.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn theme_is_cloneable() {
        let _cloned = Theme::default().clone();
    }

    #[test]
    fn theme_fields_can_be_overridden() {
        let mut t = Theme::default();
        t.h1 = Style::new().fg(Color::Red);
        assert_eq!(t.h1.fg, Some(Color::Red));
        // Other fields unchanged.
        assert_eq!(t.h2.fg, Some(Color::Cyan));
    }
}
