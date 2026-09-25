//! Hit-testing: map (screen) coordinates back to Markdown elements.
//!
//! [`MarkdownDocument`] pairs the rendered [`Text`] with per-span annotations
//! recording which Markdown element produced each piece of output. Use
//! [`MarkdownDocument::element_at`] for coordinates in *Text space* (line
//! index + display-cell column) or [`MarkdownDocument::element_at_screen`]
//! for absolute screen/mouse coordinates, given a [`ParagraphView`]
//! describing how the `Paragraph` widget renders the text (area, scroll,
//! wrap, alignment).
//!
//! ```rust
//! use the_other_tui_markdown::{ElementKind, into_document};
//!
//! let doc = into_document("A [link](https://example.com).");
//! let hit = doc.element_at(0, 2).expect("hit");
//! assert!(matches!(hit.kind, ElementKind::Link { .. }));
//! ```

use ratatui_core::layout::{Alignment, Rect};
use ratatui_core::text::Text;

use crate::wrap;

// ── Element identity ──────────────────────────────────────────────────────────

/// Opaque identifier of an [`Element`] within a [`MarkdownDocument`].
///
/// The same id is shared by every span/line originating from the same logical
/// element occurrence (e.g. all lines of one code block), so it can be used
/// to highlight a whole element on hover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementId(pub u32);

/// A Markdown element that produced (part of) the rendered output.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// What kind of element this is, including payload (e.g. a link's URL).
    pub kind: ElementKind,
    /// The enclosing element, if any (e.g. the list item containing a link).
    pub parent: Option<ElementId>,
}

/// The kind of a Markdown [`Element`].
#[derive(Debug, Clone, PartialEq)]
pub enum ElementKind {
    /// Fallback for output not attributable to a specific element.
    Document,
    /// A plain paragraph.
    Paragraph,
    /// A heading (`#` … `######`).
    Heading {
        /// Heading level, 1–6.
        level: u8,
    },
    /// A fenced or indented code block. `lang` is `None` when unspecified.
    CodeBlock { lang: Option<String> },
    /// Inline code (`` `code` ``).
    InlineCode,
    /// A block quote; `kind` is the GFM alert kind, if any.
    BlockQuote { kind: Option<QuoteKind> },
    /// A list.
    List {
        /// `true` for ordered (`1.`) lists, `false` for bullet lists.
        ordered: bool,
    },
    /// A list item.
    ListItem {
        /// `Some(checked)` for task-list items, `None` for regular items.
        checked: Option<bool>,
    },
    /// A table (covers the `─┼─` separator row and cell separators).
    Table,
    /// A table cell. `row` is 0 for the header row; body rows start at 1.
    TableCell {
        /// Row index (0 = header).
        row: u16,
        /// Column index.
        col: u16,
        /// Whether this is a header cell.
        header: bool,
    },
    /// A hyperlink.
    Link {
        /// The link's (rendered) alt text.
        alt: String,
        /// The destination URL.
        url: String,
    },
    /// An image.
    Image {
        /// The image's alt text.
        alt: String,
        /// The image URL.
        url: String,
    },
    /// Bold text.
    Strong,
    /// Italic text.
    Emphasis,
    /// Strikethrough text.
    Strikethrough,
    /// Superscript text.
    Superscript,
    /// Subscript text.
    Subscript,
    /// Inline or display math.
    Math {
        /// `true` for display math (`$$…$$`), `false` for inline (`$…$`).
        display: bool,
    },
    /// Raw HTML (inline or block).
    Html,
    /// A thematic break (`---`).
    Rule,
    /// A footnote reference (`[^1]`).
    FootnoteRef {
        /// The footnote label.
        label: String,
    },
    /// A footnote definition (rendered at the end of the document).
    FootnoteDef {
        /// The footnote label.
        label: String,
    },
    /// A definition list.
    DefinitionList,
    /// A definition-list term.
    DefinitionTitle,
    /// A definition-list definition.
    Definition,
}

/// The GFM alert kind of a block quote (`> [!NOTE]` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteKind {
    /// `> [!NOTE]`
    Note,
    /// `> [!TIP]`
    Tip,
    /// `> [!WARNING]`
    Warning,
    /// `> [!CAUTION]`
    Caution,
    /// `> [!IMPORTANT]`
    Important,
}

impl From<pulldown_cmark::BlockQuoteKind> for QuoteKind {
    fn from(kind: pulldown_cmark::BlockQuoteKind) -> Self {
        use pulldown_cmark::BlockQuoteKind as K;
        match kind {
            K::Note => QuoteKind::Note,
            K::Tip => QuoteKind::Tip,
            K::Warning => QuoteKind::Warning,
            K::Caution => QuoteKind::Caution,
            K::Important => QuoteKind::Important,
        }
    }
}

// ── Span annotations ──────────────────────────────────────────────────────────

/// Display-cell column range within one output line, tagged with its element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpanRange {
    /// Start column (inclusive), in display cells.
    pub start: u16,
    /// End column (exclusive), in display cells.
    pub end: u16,
    /// The element that produced this range.
    pub element: ElementId,
}

// ── Paragraph view ────────────────────────────────────────────────────────────

/// How a `Paragraph` widget renders the document: everything needed to map
/// absolute screen coordinates back to Text-space coordinates.
///
/// Construct with [`ParagraphView::new`] and adjust via the fluent setters:
///
/// ```rust
/// use the_other_tui_markdown::ParagraphView;
/// use ratatui_core::layout::Rect;
///
/// let view = ParagraphView::new(Rect::new(1, 1, 40, 20))
///     .scroll(3, 0)
///     .wrap(true);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParagraphView {
    /// The area the `Paragraph` is rendered into (i.e. the block's inner area).
    pub area: Rect,
    /// The `(y, x)` offset passed to `Paragraph::scroll`. With wrapping
    /// enabled, `y` counts *wrapped* rows and `x` is ignored (matching
    /// ratatui's behaviour).
    pub scroll: (u16, u16),
    /// `Some(trim)` when the paragraph uses `.wrap(Wrap { trim })`.
    pub wrap: Option<bool>,
    /// The paragraph-level alignment. Lines with their own
    /// [`ratatui_core::text::Line::alignment`] take precedence.
    pub alignment: Alignment,
}

impl ParagraphView {
    /// A view of a paragraph rendered into `area` with no scroll, no wrap,
    /// left alignment.
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            scroll: (0, 0),
            wrap: None,
            alignment: Alignment::Left,
        }
    }

    /// Set the `(y, x)` scroll offset (see [`ParagraphView::scroll`]).
    pub fn scroll(mut self, y: u16, x: u16) -> Self {
        self.scroll = (y, x);
        self
    }

    /// Enable wrapping, with the given `trim` flag (see `ratatui`'s `Wrap`).
    pub fn wrap(mut self, trim: bool) -> Self {
        self.wrap = Some(trim);
        self
    }

    /// Set the paragraph-level alignment.
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }
}

// ── MarkdownDocument ──────────────────────────────────────────────────────────

/// A rendered Markdown document: the [`Text`] for display plus the element
/// annotations needed for hit-testing.
///
/// Produced by [`crate::into_document`] and friends.
#[derive(Debug, Clone)]
pub struct MarkdownDocument {
    text: Text<'static>,
    /// Arena of all elements; indexed by [`ElementId`].
    elements: Vec<Element>,
    /// Per-line span annotations, parallel to `text.lines`.
    line_annotations: Vec<Vec<SpanRange>>,
}

impl MarkdownDocument {
    pub(crate) fn from_parts(
        text: Text<'static>,
        elements: Vec<Element>,
        line_annotations: Vec<Vec<SpanRange>>,
    ) -> Self {
        debug_assert_eq!(text.lines.len(), line_annotations.len());
        Self {
            text,
            elements,
            line_annotations,
        }
    }

    /// The rendered text, ready to hand to a `ratatui` `Paragraph` widget.
    pub fn text(&self) -> &Text<'static> {
        &self.text
    }

    /// Consume the document and return just the rendered [`Text`].
    pub fn into_text(self) -> Text<'static> {
        self.text
    }

    /// Look up an element by its [`ElementId`].
    pub fn element(&self, id: ElementId) -> &Element {
        &self.elements[id.0 as usize]
    }

    /// Iterate over the ancestors of `element`, innermost first.
    pub fn ancestors_of<'a>(
        &'a self,
        element: &'a Element,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        let mut next = element.parent;
        std::iter::from_fn(move || {
            let id = next?;
            let el = self.element(id);
            next = el.parent;
            Some(el)
        })
    }

    /// Return the innermost element covering the given **Text-space**
    /// position: `row` is a line index into [`Text::lines`], `column` a
    /// display-cell column within that line.
    ///
    /// Returns `None` for blank lines, padding, and out-of-bounds positions.
    ///
    /// This assumes each Text line maps to exactly one screen row — i.e. no
    /// wrapping, no scroll. For mouse coordinates of a `Paragraph` widget
    /// (with scroll/wrap/alignment), use [`Self::element_at_screen`].
    pub fn element_at(&self, row: u16, column: u16) -> Option<&Element> {
        let ranges = self.line_annotations.get(row as usize)?;
        let range = ranges
            .iter()
            .find(|r| r.start <= column && column < r.end)?;
        Some(self.element(range.element))
    }

    /// Return the innermost element at the given **screen** position
    /// (e.g. a crossterm mouse event's `column`/`row`), given how the
    /// `Paragraph` widget renders the text (see [`ParagraphView`]).
    ///
    /// Handles vertical scroll, horizontal scroll (no-wrap, left-aligned
    /// only — matching ratatui), word wrapping (replicating ratatui's
    /// wrapping behaviour), and left/center/right alignment. Returns `None`
    /// when the position is outside the paragraph area or over padding.
    pub fn element_at_screen(
        &self,
        row: u16,
        column: u16,
        view: &ParagraphView,
    ) -> Option<&Element> {
        let area = view.area;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        if row < area.y || row >= area.y.saturating_add(area.height) {
            return None;
        }
        if column < area.x || column >= area.x.saturating_add(area.width) {
            return None;
        }
        let y = row - area.y;
        let x = column - area.x;

        match view.wrap {
            None => {
                let line_idx = y as usize + view.scroll.0 as usize;
                let line = self.text.lines.get(line_idx)?;
                let alignment = line.alignment.unwrap_or(view.alignment);
                let row_map = wrap::truncated_row(line, area.width, view.scroll.1, alignment);
                let src = wrap::hit_src_col(&row_map, x, area.width, alignment)?;
                self.element_at(line_idx as u16, src)
            }
            Some(trim) => {
                let mut remaining = y as usize + view.scroll.0 as usize;
                for (idx, line) in self.text.lines.iter().enumerate() {
                    let rows = wrap::wrapped_rows(line, area.width, trim);
                    if remaining < rows.len() {
                        let alignment = line.alignment.unwrap_or(view.alignment);
                        let src =
                            wrap::hit_src_col(&rows[remaining], x, area.width, alignment)?;
                        return self.element_at(idx as u16, src);
                    }
                    remaining -= rows.len();
                }
                None
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RendererBuilder, into_document, into_document_with_renderer};
    use ratatui_core::text::Span;
    use unicode_width::UnicodeWidthStr;

    /// Find the (row, display-column) of the first occurrence of `needle`.
    fn find_pos(doc: &MarkdownDocument, needle: &str) -> (u16, u16) {
        for (i, line) in doc.text().lines.iter().enumerate() {
            let s: String = line.spans.iter().map(|sp| sp.content.as_ref()).collect();
            if let Some(byte) = s.find(needle) {
                let col = UnicodeWidthStr::width(&s[..byte]) as u16;
                return (i as u16, col);
            }
        }
        panic!("'{needle}' not found in document");
    }

    fn hit_kind<'a>(doc: &'a MarkdownDocument, needle: &str) -> &'a ElementKind {
        let (row, col) = find_pos(doc, needle);
        &doc.element_at(row, col)
            .unwrap_or_else(|| panic!("no element at '{needle}' ({row}, {col})"))
            .kind
    }

    fn ancestor_kinds<'a>(doc: &'a MarkdownDocument, el: &'a Element) -> Vec<&'a ElementKind> {
        doc.ancestors_of(el).map(|e| &e.kind).collect()
    }

    // ── Blocks & inline elements ──────────────────────────────────────────────

    #[test]
    fn paragraph_text() {
        let doc = into_document("Hello, world.");
        assert!(matches!(hit_kind(&doc, "world"), ElementKind::Paragraph));
    }

    #[test]
    fn heading_with_level() {
        let doc = into_document("## Title");
        assert!(
            matches!(hit_kind(&doc, "Title"), ElementKind::Heading { level: 2 }),
            "got {:?}",
            hit_kind(&doc, "Title")
        );
        // The "## " prefix belongs to the heading too.
        assert!(matches!(hit_kind(&doc, "##"), ElementKind::Heading { level: 2 }));
    }

    #[test]
    fn strong_and_emphasis() {
        let doc = into_document("a **bold** and _it_");
        assert!(matches!(hit_kind(&doc, "bold"), ElementKind::Strong));
        assert!(matches!(hit_kind(&doc, "it"), ElementKind::Emphasis));
        let (row, col) = find_pos(&doc, "bold");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            ancestor_kinds(&doc, hit)
                .iter()
                .any(|k| matches!(k, ElementKind::Paragraph))
        );
    }

    #[test]
    fn inline_code() {
        let doc = into_document("Use `code` here.");
        assert!(matches!(hit_kind(&doc, "code"), ElementKind::InlineCode));
    }

    #[test]
    fn code_block_with_lang() {
        let doc = into_document("```rust\nlet x = 1;\n```");
        assert!(matches!(
            hit_kind(&doc, "let x"),
            ElementKind::CodeBlock { lang: Some(l) } if l == "rust"
        ));
        // The [rust] language label belongs to the code block as well.
        assert!(matches!(
            hit_kind(&doc, "[rust]"),
            ElementKind::CodeBlock { lang: Some(l) } if l == "rust"
        ));
    }

    #[test]
    fn block_quote() {
        let doc = into_document("> quoted text");
        let (row, col) = find_pos(&doc, "quoted");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            ancestor_kinds(&doc, hit).iter().any(
                |k| matches!(k, ElementKind::BlockQuote { kind: None })
            ),
            "ancestors: {:?}",
            ancestor_kinds(&doc, hit)
        );
        // The "▌" prefix maps to the block quote itself.
        assert!(matches!(
            hit_kind(&doc, "▌"),
            ElementKind::BlockQuote { kind: None }
        ));
    }

    #[test]
    fn gfm_alert_quote_kind() {
        let doc = into_document("> [!NOTE]\n> pay attention");
        let (row, col) = find_pos(&doc, "pay attention");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            ancestor_kinds(&doc, hit).iter().any(
                |k| matches!(k, ElementKind::BlockQuote { kind: Some(QuoteKind::Note) })
            ),
            "ancestors: {:?}",
            ancestor_kinds(&doc, hit)
        );
    }

    // ── Lists ─────────────────────────────────────────────────────────────────

    #[test]
    fn list_item_and_marker() {
        let doc = into_document("- Apple\n- Banana");
        let (row, col) = find_pos(&doc, "Apple");
        let hit = doc.element_at(row, col).unwrap();
        // Tight lists have no Paragraph element: the item text itself maps
        // to the list item, with the list as ancestor.
        let mut kinds = vec![&hit.kind];
        kinds.extend(ancestor_kinds(&doc, hit));
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, ElementKind::ListItem { checked: None })),
            "kinds: {kinds:?}"
        );
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, ElementKind::List { ordered: false })),
            "kinds: {kinds:?}"
        );
        // The bullet marker maps to the list item.
        assert!(matches!(
            hit_kind(&doc, "•"),
            ElementKind::ListItem { checked: None }
        ));
    }

    #[test]
    fn ordered_list() {
        let doc = into_document("1. First\n2. Second");
        let (row, col) = find_pos(&doc, "Second");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            ancestor_kinds(&doc, hit)
                .iter()
                .any(|k| matches!(k, ElementKind::List { ordered: true }))
        );
    }

    #[test]
    fn task_list_checked_state() {
        let doc = into_document("- [x] Done\n- [ ] Todo");
        let (row, col) = find_pos(&doc, "[x]");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            matches!(hit.kind, ElementKind::ListItem { checked: Some(true) }),
            "got {:?}",
            hit.kind
        );
        let (row, col) = find_pos(&doc, "[ ]");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            matches!(hit.kind, ElementKind::ListItem { checked: Some(false) }),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn nested_list_indent_columns() {
        let doc = into_document("- parent\n  - child");
        let (row, col) = find_pos(&doc, "child");
        // "  • child": child starts at column 4.
        assert_eq!(col, 4);
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            ancestor_kinds(&doc, hit)
                .iter()
                .any(|k| matches!(k, ElementKind::ListItem { .. }))
        );
    }

    // ── Links & images ────────────────────────────────────────────────────────

    #[test]
    fn link_alt_and_url_suffix() {
        let doc = into_document("see [click](https://example.com)!");
        let (row, col) = find_pos(&doc, "click");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { alt, url } if alt == "click" && url == "https://example.com"),
            "got {:?}",
            hit.kind
        );
        // The "(url)" suffix also maps to the link.
        let (row, col) = find_pos(&doc, "https://example.com");
        let hit = doc.element_at(row, col).unwrap();
        assert!(matches!(hit.kind, ElementKind::Link { .. }));
    }

    #[test]
    fn link_with_custom_renderer() {
        let renderer = RendererBuilder::new()
            .with_link(|alt, url| vec![Span::raw(format!("[{alt}→{url}]"))])
            .build();
        let doc = into_document_with_renderer("go [docs](https://docs.rs) now", &renderer);
        let (row, col) = find_pos(&doc, "[docs→");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { alt, url } if alt == "docs" && url == "https://docs.rs"),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn multiple_links_keep_their_urls() {
        let doc = into_document("[a](l1) foo [b](l2)");
        let (row, col) = find_pos(&doc, "(l2)");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { alt, url } if alt == "b" && url == "l2"),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn image() {
        let doc = into_document("a ![cat](cat.png) b");
        let (row, col) = find_pos(&doc, "cat.png");
        let hit = doc.element_at(row, col).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Image { alt, url } if alt == "cat" && url == "cat.png"),
            "got {:?}",
            hit.kind
        );
    }

    // ── Tables ────────────────────────────────────────────────────────────────

    #[test]
    fn table_cells_and_separator() {
        let doc = into_document("| Name | Age |\n|------|-----|\n| Alice | 30 |");
        assert!(
            matches!(
                hit_kind(&doc, "Name"),
                ElementKind::TableCell { row: 0, col: 0, header: true }
            ),
            "got {:?}",
            hit_kind(&doc, "Name")
        );
        assert!(
            matches!(
                hit_kind(&doc, "Age"),
                ElementKind::TableCell { row: 0, col: 1, header: true }
            ),
            "got {:?}",
            hit_kind(&doc, "Age")
        );
        assert!(
            matches!(
                hit_kind(&doc, "Alice"),
                ElementKind::TableCell { row: 1, col: 0, header: false }
            ),
            "got {:?}",
            hit_kind(&doc, "Alice")
        );
        assert!(
            matches!(
                hit_kind(&doc, "30"),
                ElementKind::TableCell { row: 1, col: 1, header: false }
            ),
            "got {:?}",
            hit_kind(&doc, "30")
        );
        // The ─┼─ separator row maps to the table element.
        let (row, _) = find_pos(&doc, "Alice");
        let sep = doc.element_at(row - 1, 1).expect("separator hit");
        assert!(matches!(sep.kind, ElementKind::Table));
    }

    // ── Footnotes ─────────────────────────────────────────────────────────────

    #[test]
    fn footnote_ref_and_def() {
        let doc = into_document("See[^1].\n\n[^1]: The note.");
        assert!(
            matches!(
                hit_kind(&doc, "[^1]"),
                ElementKind::FootnoteRef { label } if label == "1"
            ),
            "got {:?}",
            hit_kind(&doc, "[^1]")
        );
        assert!(
            matches!(
                hit_kind(&doc, "The note"),
                ElementKind::FootnoteDef { label } if label == "1"
            ),
            "got {:?}",
            hit_kind(&doc, "The note")
        );
    }

    // ── Miscellaneous ─────────────────────────────────────────────────────────

    #[test]
    fn thematic_break() {
        let doc = into_document("Before\n\n---\n\nAfter");
        assert!(matches!(hit_kind(&doc, "────"), ElementKind::Rule));
    }

    #[test]
    fn inline_math() {
        let doc = into_document("formula $x^2$ here");
        assert!(
            matches!(hit_kind(&doc, "x^2"), ElementKind::Math { display: false }),
            "got {:?}",
            hit_kind(&doc, "x^2")
        );
    }

    #[test]
    fn definition_list() {
        let doc = into_document("Term\n:   A definition");
        assert!(matches!(hit_kind(&doc, "Term"), ElementKind::DefinitionTitle));
        assert!(matches!(
            hit_kind(&doc, "A definition"),
            ElementKind::Definition
        ));
    }

    // ── Unicode widths ────────────────────────────────────────────────────────

    #[test]
    fn cjk_wide_chars_column_math() {
        let doc = into_document("你好 **世界**");
        let (row, col) = find_pos(&doc, "世界");
        // "你好 " is 2 + 2 + 1 = 5 display cells.
        assert_eq!(col, 5);
        let hit = doc.element_at(row, col).unwrap();
        assert!(matches!(hit.kind, ElementKind::Strong));
        // The second cell of a wide char still hits the same element.
        let hit = doc.element_at(row, col + 1).unwrap();
        assert!(matches!(hit.kind, ElementKind::Strong));
        let hit = doc.element_at(row, 1).unwrap();
        assert!(matches!(hit.kind, ElementKind::Paragraph));
    }

    // ── Misses ────────────────────────────────────────────────────────────────

    #[test]
    fn blank_line_returns_none() {
        let doc = into_document("First.\n\nSecond.");
        assert!(doc.element_at(1, 0).is_none());
    }

    #[test]
    fn past_end_of_line_returns_none() {
        let doc = into_document("short");
        assert!(doc.element_at(0, 100).is_none());
    }

    #[test]
    fn out_of_bounds_row_returns_none() {
        let doc = into_document("one line");
        assert!(doc.element_at(42, 0).is_none());
    }

    // ── Screen-space hit-testing against a real ratatui Paragraph ─────────────
    //
    // These tests render `doc.text()` with an actual `ratatui::Paragraph`
    // into a `TestBackend` buffer and compare against our own layout
    // prediction — pinning our reimplementation of ratatui's (private) word
    // wrapping to the real thing.

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::{Paragraph, Wrap};

    /// Render `text` with a real `Paragraph` and extract the buffer rows.
    fn render_rows(
        text: &Text,
        width: u16,
        height: u16,
        trim: bool,
        scroll: (u16, u16),
        alignment: Alignment,
    ) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let p = Paragraph::new(text.clone())
                    .wrap(Wrap { trim })
                    .scroll(scroll)
                    .alignment(alignment);
                f.render_widget(p, f.area());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                // Walk cells by display width so the padding cells of wide
                // graphemes (which contain " ") are skipped.
                let mut row = String::new();
                let mut x = 0;
                while x < width {
                    let sym = buf.cell((x, y)).unwrap().symbol();
                    row.push_str(sym);
                    x += (UnicodeWidthStr::width(sym) as u16).max(1);
                }
                row
            })
            .collect()
    }

    /// Our own prediction of the wrapped layout (row strings incl. alignment
    /// padding), after applying vertical scroll.
    fn predicted_rows(
        text: &Text,
        width: u16,
        trim: bool,
        scroll_y: u16,
        alignment: Alignment,
    ) -> Vec<String> {
        let mut rows = Vec::new();
        for line in &text.lines {
            let align = line.alignment.unwrap_or(alignment);
            for r in crate::wrap::wrapped_rows(line, width, trim) {
                let mut s = " ".repeat(crate::wrap::line_offset(r.width, width, align) as usize);
                for seg in &r.segs {
                    s.push_str(seg.sym);
                }
                rows.push(s);
            }
        }
        rows.into_iter().skip(scroll_y as usize).collect()
    }

    /// Assert our wrapping prediction matches a real `Paragraph` render.
    fn assert_layout_matches(
        text: &Text,
        width: u16,
        height: u16,
        trim: bool,
        scroll: (u16, u16),
        alignment: Alignment,
    ) {
        let buffer = render_rows(text, width, height, trim, scroll, alignment);
        let predicted = predicted_rows(text, width, trim, scroll.0, alignment);
        for (y, pred) in predicted.iter().enumerate().take(height as usize) {
            assert_eq!(
                buffer[y].trim_end(),
                pred.trim_end(),
                "row {y} mismatch (width={width}, trim={trim}, scroll={scroll:?}, {alignment:?})\nbuffer: {:?}",
                buffer
            );
        }
        for (y, row) in buffer.iter().enumerate().skip(predicted.len()) {
            assert!(
                row.trim().is_empty(),
                "row {y} should be blank, got {row:?} (predicted only {} rows)",
                predicted.len()
            );
        }
    }

    const LONG_TEXT: &str = "The quick brown fox jumps over the lazy dog. \
        Pack my box with five dozen liquor jugs. How vexingly quick daft \
        zebras jump!";

    fn long_text() -> Text<'static> {
        crate::into_text(LONG_TEXT)
    }

    #[test]
    fn layout_wrap_trim_true() {
        let text = long_text();
        for width in [10, 20, 25, 40] {
            assert_layout_matches(&text, width, 20, true, (0, 0), Alignment::Left);
        }
    }

    #[test]
    fn layout_wrap_trim_false() {
        let text = long_text();
        for width in [10, 20, 25, 40] {
            assert_layout_matches(&text, width, 20, false, (0, 0), Alignment::Left);
        }
    }

    #[test]
    fn layout_long_word_hard_split() {
        let text = crate::into_text("supercalifragilisticexpialidocious ok");
        for trim in [true, false] {
            assert_layout_matches(&text, 10, 8, trim, (0, 0), Alignment::Left);
        }
    }

    #[test]
    fn layout_cjk_wrap() {
        let text = crate::into_text("你好世界，这是一个测试。再来一行。");
        for trim in [true, false] {
            for width in [10, 11, 12] {
                assert_layout_matches(&text, width, 10, trim, (0, 0), Alignment::Left);
            }
        }
    }

    #[test]
    fn layout_wrap_with_scroll() {
        let text = long_text();
        assert_layout_matches(&text, 20, 6, true, (3, 0), Alignment::Left);
        assert_layout_matches(&text, 20, 6, false, (5, 0), Alignment::Left);
    }

    #[test]
    fn layout_wrap_with_alignment() {
        let text = crate::into_text("short words here and there");
        for alignment in [Alignment::Center, Alignment::Right] {
            assert_layout_matches(&text, 12, 8, true, (0, 0), alignment);
            assert_layout_matches(&text, 12, 8, false, (0, 0), alignment);
        }
    }

    #[test]
    fn layout_blank_lines() {
        let text = crate::into_text("First.\n\nSecond paragraph that is long enough to wrap.");
        assert_layout_matches(&text, 15, 10, true, (0, 0), Alignment::Left);
        assert_layout_matches(&text, 15, 10, false, (0, 0), Alignment::Left);
    }

    // ── element_at_screen ─────────────────────────────────────────────────────

    /// Render `doc` full-screen and find the screen rect of `needle`
    /// (must fit on one screen row). Returns (row, col) of its first cell.
    fn find_on_screen(
        doc: &MarkdownDocument,
        width: u16,
        height: u16,
        trim: bool,
        scroll: (u16, u16),
        alignment: Alignment,
        needle: &str,
    ) -> (u16, u16) {
        let rows = render_rows(doc.text(), width, height, trim, scroll, alignment);
        for (y, row) in rows.iter().enumerate() {
            if let Some(x) = row.find(needle) {
                return (y as u16, x as u16);
            }
        }
        panic!("'{needle}' not visible on screen: {rows:?}");
    }

    #[test]
    fn screen_wrapped_link_hit() {
        let doc = into_document(
            "Intro text here [examplelink](https://example.com) and more \
             text to make the paragraph wrap around the link.",
        );
        let view = ParagraphView::new(Rect::new(0, 0, 25, 20)).wrap(true);
        // The link alt fits on one screen row at width 25.
        let (row, col) = find_on_screen(&doc, 25, 20, true, (0, 0), Alignment::Left, "examplelink");
        for x in col..col + "examplelink".len() as u16 {
            let hit = doc
                .element_at_screen(row, x, &view)
                .unwrap_or_else(|| panic!("no hit at ({row}, {x})"));
            assert!(
                matches!(&hit.kind, ElementKind::Link { alt, url } if alt == "examplelink" && url == "https://example.com"),
                "at ({row}, {x}): {:?}",
                hit.kind
            );
        }
        // A non-link cell maps to the paragraph.
        let (row, col) = find_on_screen(&doc, 25, 20, true, (0, 0), Alignment::Left, "Intro");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(matches!(hit.kind, ElementKind::Paragraph));
    }

    #[test]
    fn screen_every_content_cell_hits() {
        let doc = into_document(
            "Words [a link](https://a.b) words words words words words words \
             words words words words words.",
        );
        let (w, h) = (18, 12);
        let view = ParagraphView::new(Rect::new(0, 0, w, h)).wrap(true);
        let rows = render_rows(doc.text(), w, h, true, (0, 0), Alignment::Left);
        for (y, row) in rows.iter().enumerate() {
            for (x, _) in row.char_indices().filter(|(_, c)| *c != ' ') {
                let hit = doc.element_at_screen(y as u16, x as u16, &view);
                assert!(
                    hit.is_some(),
                    "non-space cell ({y}, {x}) of {row:?} should hit an element"
                );
            }
        }
    }

    #[test]
    fn screen_wrapped_link_with_scroll() {
        let doc = into_document(
            "Some leading text that takes up space [examplelink](https://example.com) \
             and trailing text that wraps further down the screen.",
        );
        let scroll = (2u16, 0u16);
        let view = ParagraphView::new(Rect::new(0, 0, 20, 20)).wrap(true).scroll(2, 0);
        let (row, col) = find_on_screen(&doc, 20, 20, true, scroll, Alignment::Left, "examplelink");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { url, .. } if url == "https://example.com"),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn screen_no_wrap_table_hit() {
        let doc = into_document("| Name | Age |\n|------|-----|\n| Alice | 30 |");
        let view = ParagraphView::new(Rect::new(0, 0, 30, 10));
        let (row, col) = find_on_screen(&doc, 30, 10, true, (0, 0), Alignment::Left, "Alice");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(
            matches!(hit.kind, ElementKind::TableCell { row: 1, col: 0, header: false }),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn screen_center_aligned_hit() {
        let doc = into_document("go [examplelink](https://example.com) now");
        let view =
            ParagraphView::new(Rect::new(0, 0, 40, 10)).alignment(Alignment::Center);
        let (row, col) = find_on_screen(&doc, 40, 10, true, (0, 0), Alignment::Center, "examplelink");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { url, .. } if url == "https://example.com"),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn screen_right_aligned_wrapped_hit() {
        let doc = into_document("go [examplelink](https://example.com) now with more words");
        let view =
            ParagraphView::new(Rect::new(0, 0, 20, 10)).wrap(true).alignment(Alignment::Right);
        let (row, col) = find_on_screen(&doc, 20, 10, true, (0, 0), Alignment::Right, "examplelink");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { url, .. } if url == "https://example.com"),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn screen_horizontal_scroll_no_wrap() {
        let doc = into_document("see [docs](https://docs.rs) here");
        let view = ParagraphView::new(Rect::new(0, 0, 30, 10)).scroll(0, 4);
        let (row, col) = find_on_screen(&doc, 30, 10, true, (0, 4), Alignment::Left, "docs");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(
            matches!(&hit.kind, ElementKind::Link { url, .. } if url == "https://docs.rs"),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn screen_offset_area() {
        let doc = into_document("[examplelink](https://example.com)");
        let view = ParagraphView::new(Rect::new(5, 3, 30, 10));
        // Inside the offset area, the link starts at (3, 5).
        let hit = doc.element_at_screen(3, 5, &view).unwrap();
        assert!(matches!(hit.kind, ElementKind::Link { .. }));
        // Outside the area: no hit.
        assert!(doc.element_at_screen(2, 5, &view).is_none());
        assert!(doc.element_at_screen(3, 4, &view).is_none());
        assert!(doc.element_at_screen(0, 0, &view).is_none());
    }

    #[test]
    fn screen_per_line_alignment_override() {
        // A custom renderer returns a right-aligned line; the paragraph-level
        // alignment (Left) must not apply to it.
        use ratatui_core::text::Line;
        let renderer = RendererBuilder::new()
            .with_heading(|_, spans| vec![Line::from(spans).alignment(Alignment::Right)])
            .build();
        let doc = into_document_with_renderer("# Head\n\nbody text here", &renderer);
        assert_layout_matches(doc.text(), 20, 6, true, (0, 0), Alignment::Left);
        let view = ParagraphView::new(Rect::new(0, 0, 20, 6));
        let (row, col) = find_on_screen(&doc, 20, 6, true, (0, 0), Alignment::Left, "Head");
        let hit = doc.element_at_screen(row, col, &view).unwrap();
        assert!(
            matches!(hit.kind, ElementKind::Heading { level: 1 }),
            "got {:?}",
            hit.kind
        );
    }

    #[test]
    fn screen_padding_returns_none() {
        let doc = into_document("tiny");
        let view = ParagraphView::new(Rect::new(0, 0, 30, 10));
        // Past the end of the text.
        assert!(doc.element_at_screen(0, 10, &view).is_none());
        // Below all content.
        assert!(doc.element_at_screen(5, 0, &view).is_none());
    }
}
