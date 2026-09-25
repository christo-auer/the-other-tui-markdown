//! Core event-driven converter: walks `pulldown-cmark` events and builds
//! `ratatui_core::text::Text`.

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui_core::style::Style;
use ratatui_core::text::{Line, Span, Text};
use unicode_width::UnicodeWidthStr;

use crate::Renderer;
use crate::document::{Element, ElementId, ElementKind, MarkdownDocument, SpanRange};

/// The root element (kind [`ElementKind::Document`]) at arena index 0.
const ROOT_ELEMENT: ElementId = ElementId(0);

// ── Block context stack ───────────────────────────────────────────────────────

/// Describes the block-level context we are currently inside.
#[derive(Debug, Clone)]
enum BlockCtx {
    Paragraph,
    Heading(HeadingLevel),
    CodeBlock { lang: String, content: String },
    BlockQuote(Option<BlockQuoteKind>),
    OrderedList(u64),
    BulletList,
    Item,
    /// Buffering a table so we can compute column widths before emitting.
    Table(TableBuf),
    TableHead,
    TableRow,
    TableCell,
    FootnoteDef(String),
    DefinitionList,
    DefinitionListTitle,
    DefinitionListDefinition,
    MetadataBlock,
    HtmlBlock,
}

// ── Table buffering ───────────────────────────────────────────────────────────

/// A fully-buffered table (all rows collected before rendering).
#[derive(Debug, Clone)]
struct TableBuf {
    /// Header cells (plain strings).
    header: Vec<String>,
    /// Body rows, each a Vec of plain strings.
    rows: Vec<Vec<String>>,
    /// Current cell being accumulated.
    current_cell: String,
    /// Whether we are currently in the header section.
    in_header: bool,
    /// Current partial row being accumulated.
    current_row: Vec<String>,
}

impl TableBuf {
    fn new() -> Self {
        Self {
            header: Vec::new(),
            rows: Vec::new(),
            current_cell: String::new(),
            in_header: false,
            current_row: Vec::new(),
        }
    }
}

// ── Footnote definition buffering ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct FootnoteDef {
    label: String,
    content: String,
    /// The element created for this footnote definition.
    element: ElementId,
}

// ── Converter ────────────────────────────────────────────────────────────────

pub(crate) struct Converter<'r> {
    renderer: &'r Renderer,
    /// Completed output lines.
    lines: Vec<Line<'static>>,
    /// Spans accumulating for the line currently being built.
    current_spans: Vec<Span<'static>>,
    /// Block-level context stack.
    block_stack: Vec<BlockCtx>,
    /// Inline modifier style stack (bold, italic, link …).
    inline_stack: Vec<Style>,
    /// Nesting depth of list items (for indentation).
    item_depth: usize,
    /// URL stashed when we enter `Tag::Link`.
    pending_link_url: Option<String>,
    /// Index into `current_spans` at which the current link's content begins.
    /// Used to extract only the link's own spans when the link closes.
    link_span_start: Option<usize>,
    /// Alt text accumulated while inside `Tag::Image`.
    pending_image_alt: Option<String>,
    /// URL stashed when we enter `Tag::Image`.
    pending_image_url: Option<String>,
    /// Whether we are currently inside an image (so text goes to alt buffer).
    in_image: bool,
    /// Footnote definitions collected during the parse (rendered at the end).
    footnote_defs: Vec<FootnoteDef>,
    // ── Hit-test annotation state ─────────────────────────────────────────────
    /// Arena of all elements encountered so far (indexed by [`ElementId`]).
    elements: Vec<Element>,
    /// Stack of currently open elements (innermost last). Never empty.
    element_stack: Vec<ElementId>,
    /// Per-line span annotations, parallel to `lines`.
    line_annotations: Vec<Vec<SpanRange>>,
    /// Ranges accumulating for the line currently being built.
    current_ranges: Vec<SpanRange>,
    /// Display-cell column the next span of the current line starts at.
    current_col: u16,
    /// Display-cell column at which the current link's content begins.
    link_col_start: Option<u16>,
}

impl<'r> Converter<'r> {
    pub(crate) fn new(renderer: &'r Renderer) -> Self {
        Self {
            renderer,
            lines: Vec::new(),
            current_spans: Vec::new(),
            block_stack: Vec::new(),
            inline_stack: Vec::new(),
            item_depth: 0,
            pending_link_url: None,
            link_span_start: None,
            pending_image_alt: None,
            pending_image_url: None,
            in_image: false,
            footnote_defs: Vec::new(),
            elements: vec![Element {
                kind: ElementKind::Document,
                parent: None,
            }],
            element_stack: vec![ROOT_ELEMENT],
            line_annotations: Vec::new(),
            current_ranges: Vec::new(),
            current_col: 0,
            link_col_start: None,
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn theme(&self) -> &crate::Theme {
        &self.renderer.theme
    }

    /// Effective inline style: block base style patched with every active inline modifier.
    fn current_style(&self) -> Style {
        let base = self.current_block_style();
        self.inline_stack.iter().fold(base, |acc, s| acc.patch(*s))
    }

    /// The base style from the innermost relevant block context.
    fn current_block_style(&self) -> Style {
        for ctx in self.block_stack.iter().rev() {
            match ctx {
                BlockCtx::Heading(level) => return self.heading_style(*level),
                BlockCtx::CodeBlock { .. } => return self.theme().code_block,
                BlockCtx::BlockQuote(kind) => return self.blockquote_style(*kind),
                BlockCtx::HtmlBlock => return self.theme().html,
                BlockCtx::FootnoteDef(_) => return self.theme().footnote_def,
                _ => {}
            }
        }
        Style::default()
    }

    fn heading_style(&self, level: HeadingLevel) -> Style {
        match level {
            HeadingLevel::H1 => self.theme().h1,
            HeadingLevel::H2 => self.theme().h2,
            HeadingLevel::H3 => self.theme().h3,
            HeadingLevel::H4 => self.theme().h4,
            HeadingLevel::H5 => self.theme().h5,
            HeadingLevel::H6 => self.theme().h6,
        }
    }

    fn blockquote_style(&self, kind: Option<BlockQuoteKind>) -> Style {
        match kind {
            None => self.theme().block_quote,
            Some(BlockQuoteKind::Note) => self.theme().block_quote_note,
            Some(BlockQuoteKind::Tip) => self.theme().block_quote_tip,
            Some(BlockQuoteKind::Warning) => self.theme().block_quote_warning,
            Some(BlockQuoteKind::Caution) => self.theme().block_quote_caution,
            Some(BlockQuoteKind::Important) => self.theme().block_quote_important,
        }
    }

    fn heading_prefix(level: HeadingLevel) -> &'static str {
        match level {
            HeadingLevel::H1 => "# ",
            HeadingLevel::H2 => "## ",
            HeadingLevel::H3 => "### ",
            HeadingLevel::H4 => "#### ",
            HeadingLevel::H5 => "##### ",
            HeadingLevel::H6 => "###### ",
        }
    }

    fn heading_level_u8(level: HeadingLevel) -> u8 {
        match level {
            HeadingLevel::H1 => 1,
            HeadingLevel::H2 => 2,
            HeadingLevel::H3 => 3,
            HeadingLevel::H4 => 4,
            HeadingLevel::H5 => 5,
            HeadingLevel::H6 => 6,
        }
    }

    fn list_indent(&self) -> String {
        "  ".repeat(self.item_depth.saturating_sub(1))
    }

    // ── Element stack ─────────────────────────────────────────────────────────

    /// The innermost open element (or the root).
    fn current_element(&self) -> ElementId {
        *self.element_stack.last().unwrap_or(&ROOT_ELEMENT)
    }

    /// Open a new element as a child of the current one and return its id.
    fn push_element(&mut self, kind: ElementKind) -> ElementId {
        let parent = match self.element_stack.last() {
            Some(&id) if id != ROOT_ELEMENT => Some(id),
            _ => None,
        };
        let id = ElementId(self.elements.len() as u32);
        self.elements.push(Element { kind, parent });
        self.element_stack.push(id);
        id
    }

    /// Close the innermost element (keeping the root).
    fn pop_element(&mut self) {
        if self.element_stack.len() > 1 {
            self.element_stack.pop();
        }
    }

    /// Mutate the kind of the innermost element (e.g. to fill in link alt
    /// text once the closing tag is reached).
    fn patch_current_element(&mut self, f: impl FnOnce(&mut ElementKind)) {
        let id = self.current_element();
        f(&mut self.elements[id.0 as usize].kind);
    }

    // ── Span/line emission with hit-test annotation ───────────────────────────

    /// Append a span to the current line and record its column range.
    fn emit_span(&mut self, span: Span<'static>) {
        let width = UnicodeWidthStr::width(span.content.as_ref()) as u16;
        if width > 0 {
            let start = self.current_col;
            let end = start.saturating_add(width);
            self.current_ranges.push(SpanRange {
                start,
                end,
                element: self.current_element(),
            });
            self.current_col = end;
        }
        self.current_spans.push(span);
    }

    fn push_span(&mut self, content: impl Into<String>, style: Style) {
        let content = content.into();
        if !content.is_empty() {
            self.emit_span(Span::styled(content, style));
        }
    }

    fn commit_line(&mut self) {
        let spans = std::mem::take(&mut self.current_spans);
        self.lines.push(Line::from(spans));
        let ranges = std::mem::take(&mut self.current_ranges);
        self.line_annotations.push(ranges);
        self.current_col = 0;
    }

    fn push_blank_line(&mut self) {
        self.lines.push(Line::default());
        self.line_annotations.push(Vec::new());
    }

    fn commit_line_and_blank(&mut self) {
        self.commit_line();
        self.push_blank_line();
    }

    /// Take the accumulated current spans (to hand to a custom renderer),
    /// discarding their ranges — the lines built from them are re-annotated
    /// when appended via [`Self::extend_annotated_lines`].
    fn take_current_spans(&mut self) -> Vec<Span<'static>> {
        self.current_ranges.clear();
        self.current_col = 0;
        std::mem::take(&mut self.current_spans)
    }

    /// Append an already-built line, annotating every span with `element`.
    fn push_annotated_line(&mut self, line: Line<'static>, element: ElementId) {
        let mut col = 0u16;
        let mut ranges = Vec::new();
        for span in &line.spans {
            let width = UnicodeWidthStr::width(span.content.as_ref()) as u16;
            if width > 0 {
                ranges.push(SpanRange {
                    start: col,
                    end: col + width,
                    element,
                });
                col += width;
            }
        }
        self.lines.push(line);
        self.line_annotations.push(ranges);
    }

    /// Append already-built lines, annotating every span with `element`.
    fn extend_annotated_lines(&mut self, lines: Vec<Line<'static>>, element: ElementId) {
        for line in lines {
            self.push_annotated_line(line, element);
        }
    }

    /// True if the innermost block context should swallow text silently.
    fn is_table_context(&self) -> bool {
        for ctx in self.block_stack.iter().rev() {
            match ctx {
                BlockCtx::Table(_) | BlockCtx::TableHead | BlockCtx::TableRow | BlockCtx::TableCell => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// True if we're inside a block context that produces no visible output.
    fn is_metadata(&self) -> bool {
        self.block_stack
            .last()
            .map_or(false, |b| matches!(b, BlockCtx::MetadataBlock))
    }

    /// If inside a table cell, append text to the current cell buffer.
    fn try_append_table_cell_text(&mut self, s: &str) {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::Table(buf) = ctx {
                buf.current_cell.push_str(s);
                return;
            }
        }
    }

    // ── Table rendering ───────────────────────────────────────────────────────

    /// Create a [`ElementKind::TableCell`] element under the given table.
    fn new_table_cell(&mut self, table: ElementId, row: u16, col: u16, header: bool) -> ElementId {
        let id = ElementId(self.elements.len() as u32);
        self.elements.push(Element {
            kind: ElementKind::TableCell { row, col, header },
            parent: Some(table),
        });
        id
    }

    /// Render the buffered table, annotating header/body cells with
    /// [`ElementKind::TableCell`] and separators with the table element.
    fn render_table(&mut self, buf: &TableBuf, table: ElementId) {
        let ncols = buf
            .header
            .len()
            .max(buf.rows.iter().map(|r| r.len()).max().unwrap_or(0));

        if ncols == 0 {
            return;
        }

        let theme = self.renderer.theme();

        // Compute column widths (display width).
        let mut col_widths: Vec<usize> = (0..ncols)
            .map(|i| buf.header.get(i).map(|h| UnicodeWidthStr::width(h.as_str())).unwrap_or(0))
            .collect();
        for row in &buf.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < ncols {
                    col_widths[i] = col_widths[i].max(UnicodeWidthStr::width(cell.as_str()));
                }
            }
        }
        // Minimum column width of 1.
        for w in &mut col_widths {
            *w = (*w).max(1);
        }

        // Header row.
        let header_cells: Vec<(String, Style, ElementId)> = (0..ncols)
            .map(|i| {
                let cell = buf.header.get(i).cloned().unwrap_or_default();
                let el = self.new_table_cell(table, 0, i as u16, true);
                (cell, theme.table_header, el)
            })
            .collect();
        let (line, ranges) = build_table_row(&header_cells, &col_widths, theme.table_separator, table);
        self.lines.push(line);
        self.line_annotations.push(ranges);

        // Separator row: ─────┼───── for each column.
        {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut ranges: Vec<SpanRange> = Vec::new();
            let mut col = 0u16;
            for (i, w) in col_widths.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled("─┼─", theme.table_separator));
                    ranges.push(SpanRange {
                        start: col,
                        end: col + 3,
                        element: table,
                    });
                    col += 3;
                }
                let w = *w as u16;
                spans.push(Span::styled("─".repeat(w as usize), theme.table_separator));
                ranges.push(SpanRange {
                    start: col,
                    end: col + w,
                    element: table,
                });
                col += w;
            }
            self.lines.push(Line::from(spans));
            self.line_annotations.push(ranges);
        }

        // Body rows.
        for (r, row) in buf.rows.iter().enumerate() {
            let cells: Vec<(String, Style, ElementId)> = (0..ncols)
                .map(|i| {
                    let cell = row.get(i).cloned().unwrap_or_default();
                    let el = self.new_table_cell(table, (r + 1) as u16, i as u16, false);
                    (cell, theme.table_cell, el)
                })
                .collect();
            let (line, ranges) =
                build_table_row(&cells, &col_widths, theme.table_separator, table);
            self.lines.push(line);
            self.line_annotations.push(ranges);
        }
    }

    // ── Item marker ───────────────────────────────────────────────────────────

    fn make_item_marker(&self) -> String {
        for ctx in self.block_stack.iter().rev() {
            match ctx {
                BlockCtx::OrderedList(n) => return format!("{}. ", n),
                BlockCtx::BulletList => return "• ".to_string(),
                _ => {}
            }
        }
        "• ".to_string()
    }

    fn advance_list_counter(&mut self) {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::OrderedList(n) = ctx {
                *n += 1;
                return;
            }
            if matches!(ctx, BlockCtx::BulletList) {
                return;
            }
        }
    }

    // ── Footnote definitions ──────────────────────────────────────────────────

    fn current_footnote_def_label(&self) -> Option<String> {
        for ctx in self.block_stack.iter().rev() {
            if let BlockCtx::FootnoteDef(label) = ctx {
                return Some(label.clone());
            }
        }
        None
    }

    fn append_to_current_footnote(&mut self, s: &str) {
        if let Some(label) = self.current_footnote_def_label() {
            if let Some(def) = self.footnote_defs.iter_mut().find(|d| d.label == label) {
                def.content.push_str(s);
            }
        }
    }

    fn render_footnote_defs(&mut self) {
        if self.footnote_defs.is_empty() {
            return;
        }
        self.push_blank_line();
        let rule_id = self.push_element(ElementKind::Rule);
        let rule_line = Line::from(Span::styled("─".repeat(40), self.theme().rule));
        self.push_annotated_line(rule_line, rule_id);
        self.pop_element();
        let defs = std::mem::take(&mut self.footnote_defs);
        for def in &defs {
            let label_span = Span::styled(
                format!("[{}]: ", def.label),
                self.theme().footnote_def,
            );
            let content_span = Span::styled(def.content.clone(), self.theme().footnote_def);
            self.push_annotated_line(Line::from(vec![label_span, content_span]), def.element);
        }
    }

    // ── Main convert entry-point ──────────────────────────────────────────────

    pub(crate) fn convert(&mut self, markdown: &str) -> MarkdownDocument {
        let options = Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TABLES
            | Options::ENABLE_GFM
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_MATH
            | Options::ENABLE_SUPERSCRIPT
            | Options::ENABLE_SUBSCRIPT
            | Options::ENABLE_DEFINITION_LIST;

        let parser = Parser::new_ext(markdown, options);

        for event in parser {
            self.handle_event(event);
        }

        // Flush dangling spans.
        if !self.current_spans.is_empty() {
            self.commit_line();
        }

        // Append footnote definitions at the end.
        self.render_footnote_defs();

        // Remove trailing blank line.
        if self.lines.last().map_or(false, |l| l.spans.is_empty()) {
            self.lines.pop();
            self.line_annotations.pop();
        }

        let mut text = Text::from(std::mem::take(&mut self.lines));
        text.style = self.theme().base;
        MarkdownDocument::from_parts(
            text,
            std::mem::take(&mut self.elements),
            std::mem::take(&mut self.line_annotations),
        )
    }

    // ── Event dispatch ────────────────────────────────────────────────────────

    fn handle_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.handle_start(tag),
            Event::End(tag) => self.handle_end(tag),

            Event::Text(text) => {
                if self.is_metadata() {
                    return;
                }
                let s = text.into_string();
                // Inside an image: accumulate alt text.
                if self.in_image {
                    if let Some(alt) = &mut self.pending_image_alt {
                        alt.push_str(&s);
                    }
                    return;
                }
                // Inside a table cell: buffer the text.
                if self.is_table_context() {
                    self.try_append_table_cell_text(&s);
                    return;
                }
                // Inside a footnote def: buffer content.
                if self.current_footnote_def_label().is_some() {
                    self.append_to_current_footnote(&s);
                    return;
                }
                let style = self.current_style();
                // Code block text may contain embedded newlines.
                // For code blocks specifically, we buffer into the BlockCtx.
                if self.is_code_block_context() {
                    self.append_to_code_block(&s);
                    return;
                }
                let mut lines = s.split('\n').peekable();
                while let Some(line) = lines.next() {
                    self.push_span(line, style);
                    if lines.peek().is_some() {
                        self.commit_line();
                    }
                }
            }

            Event::Code(code) => {
                if self.is_metadata() || self.is_table_context() {
                    return;
                }
                self.push_element(ElementKind::InlineCode);
                let code_str = code.into_string();
                // Custom inline_code renderer?
                let spans = if let Some(f) = &self.renderer.inline_code {
                    f(&code_str)
                } else {
                    let style = self.current_style().patch(self.theme().inline_code);
                    vec![Span::styled(code_str, style)]
                };
                for span in spans {
                    self.emit_span(span);
                }
                self.pop_element();
            }

            Event::InlineMath(math) => self.handle_math(math.into_string(), false),
            Event::DisplayMath(math) => self.handle_math(math.into_string(), true),

            Event::SoftBreak => {
                if self.is_metadata() || self.in_image || self.is_table_context() {
                    return;
                }
                if self.current_footnote_def_label().is_some() {
                    self.append_to_current_footnote(" ");
                    return;
                }
                let style = self.current_style();
                self.push_span(" ", style);
            }

            Event::HardBreak => {
                if self.is_metadata() || self.in_image || self.is_table_context() {
                    return;
                }
                self.commit_line();
            }

            Event::Rule => {
                self.commit_line();
                let lines = if let Some(f) = &self.renderer.rule {
                    f()
                } else {
                    vec![
                        Line::from(Span::styled("─".repeat(40), self.theme().rule)),
                    ]
                };
                let rule = self.push_element(ElementKind::Rule);
                self.extend_annotated_lines(lines, rule);
                self.pop_element();
                self.push_blank_line();
            }

            Event::Html(html) | Event::InlineHtml(html) => {
                if self.is_metadata() {
                    return;
                }
                self.push_element(ElementKind::Html);
                let style = self.theme().html;
                let s = html.into_string();
                let mut iter = s.split('\n').peekable();
                while let Some(line) = iter.next() {
                    self.push_span(line.trim_end_matches('\r'), style);
                    if iter.peek().is_some() {
                        self.commit_line();
                    }
                }
                self.pop_element();
            }

            Event::TaskListMarker(checked) => {
                if self.is_metadata() {
                    return;
                }
                // Record the checkbox state on the enclosing list item.
                for &id in self.element_stack.iter().rev() {
                    if let ElementKind::ListItem { checked: c } =
                        &mut self.elements[id.0 as usize].kind
                    {
                        *c = Some(checked);
                        break;
                    }
                }
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_span(marker, self.current_style());
            }

            Event::FootnoteReference(label) => {
                let label = label.into_string();
                self.push_element(ElementKind::FootnoteRef {
                    label: label.clone(),
                });
                let spans = if let Some(f) = &self.renderer.footnote_ref {
                    f(&label)
                } else {
                    vec![Span::styled(
                        format!("[^{}]", label),
                        self.theme().footnote_ref,
                    )]
                };
                for span in spans {
                    self.emit_span(span);
                }
                self.pop_element();
            }

        }
    }

    // ── Inline math ───────────────────────────────────────────────────────────

    fn handle_math(&mut self, math: String, display: bool) {
        if self.is_metadata() {
            return;
        }
        self.push_element(ElementKind::Math { display });
        let style = self.current_style().patch(self.theme().math);
        self.push_span(math, style);
        self.pop_element();
    }

    // ── Code block helpers ────────────────────────────────────────────────────

    fn is_code_block_context(&self) -> bool {
        for ctx in self.block_stack.iter().rev() {
            if matches!(ctx, BlockCtx::CodeBlock { .. }) {
                return true;
            }
        }
        false
    }

    fn append_to_code_block(&mut self, s: &str) {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::CodeBlock { content, .. } = ctx {
                content.push_str(s);
                return;
            }
        }
    }

    // ── Start tag handling ────────────────────────────────────────────────────

    fn handle_start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.block_stack.push(BlockCtx::Paragraph);
                self.push_element(ElementKind::Paragraph);
            }

            Tag::Heading { level, .. } => {
                self.block_stack.push(BlockCtx::Heading(level));
                self.push_element(ElementKind::Heading {
                    level: Self::heading_level_u8(level),
                });
                // If there's no custom heading renderer, push the prefix span immediately.
                if self.renderer.heading.is_none() {
                    let style = self.heading_style(level);
                    self.push_span(Self::heading_prefix(level), style);
                }
            }

            Tag::BlockQuote(kind) => {
                self.block_stack.push(BlockCtx::BlockQuote(kind));
                self.push_element(ElementKind::BlockQuote {
                    kind: kind.map(crate::QuoteKind::from),
                });
                let style = self.blockquote_style(kind);
                self.push_span("▌ ", style);
            }

            Tag::CodeBlock(kind) => {
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) => l.trim().to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.push_element(ElementKind::CodeBlock {
                    lang: if lang.is_empty() { None } else { Some(lang.clone()) },
                });
                self.block_stack.push(BlockCtx::CodeBlock {
                    lang,
                    content: String::new(),
                });
            }

            Tag::List(start) => {
                // If we're inside a list item that already has text spans,
                // commit them as a line before starting the nested list.
                if !self.current_spans.is_empty() {
                    self.commit_line();
                }
                self.push_element(ElementKind::List {
                    ordered: start.is_some(),
                });
                let ctx = if let Some(n) = start {
                    BlockCtx::OrderedList(n)
                } else {
                    BlockCtx::BulletList
                };
                self.block_stack.push(ctx);
            }

            Tag::Item => {
                self.item_depth += 1;
                self.block_stack.push(BlockCtx::Item);
                self.push_element(ElementKind::ListItem { checked: None });
                let indent = self.list_indent();
                let marker = self.make_item_marker();
                let style = self.theme().list_marker;
                self.push_span(format!("{}{}", indent, marker), style);
                self.advance_list_counter();
            }

            Tag::Table(_) => {
                self.block_stack.push(BlockCtx::Table(TableBuf::new()));
                self.push_element(ElementKind::Table);
            }
            Tag::TableHead => {
                // Mark the table buffer as being in the header.
                for ctx in self.block_stack.iter_mut().rev() {
                    if let BlockCtx::Table(buf) = ctx {
                        buf.in_header = true;
                        break;
                    }
                }
                self.block_stack.push(BlockCtx::TableHead);
            }
            Tag::TableRow => {
                self.block_stack.push(BlockCtx::TableRow);
            }
            Tag::TableCell => {
                self.block_stack.push(BlockCtx::TableCell);
            }

            Tag::FootnoteDefinition(label) => {
                let label = label.into_string();
                let element = self.push_element(ElementKind::FootnoteDef {
                    label: label.clone(),
                });
                self.footnote_defs.push(FootnoteDef {
                    label: label.clone(),
                    content: String::new(),
                    element,
                });
                self.block_stack.push(BlockCtx::FootnoteDef(label));
            }

            Tag::DefinitionList => {
                self.block_stack.push(BlockCtx::DefinitionList);
                self.push_element(ElementKind::DefinitionList);
            }
            Tag::DefinitionListTitle => {
                self.block_stack.push(BlockCtx::DefinitionListTitle);
                self.push_element(ElementKind::DefinitionTitle);
                // Render title as bold.
                self.inline_stack.push(self.theme().strong);
            }
            Tag::DefinitionListDefinition => {
                self.block_stack.push(BlockCtx::DefinitionListDefinition);
                self.push_element(ElementKind::Definition);
                self.push_span("  ", Style::default());
            }

            Tag::MetadataBlock(_) => {
                self.block_stack.push(BlockCtx::MetadataBlock);
            }

            Tag::HtmlBlock => {
                self.block_stack.push(BlockCtx::HtmlBlock);
                self.push_element(ElementKind::Html);
            }

            // Inline tags.
            Tag::Emphasis => {
                self.inline_stack.push(self.theme().emphasis);
                self.push_element(ElementKind::Emphasis);
            }
            Tag::Strong => {
                self.inline_stack.push(self.theme().strong);
                self.push_element(ElementKind::Strong);
            }
            Tag::Strikethrough => {
                self.inline_stack.push(self.theme().strikethrough);
                self.push_element(ElementKind::Strikethrough);
            }
            Tag::Superscript => {
                self.inline_stack.push(self.theme().superscript);
                self.push_element(ElementKind::Superscript);
            }
            Tag::Subscript => {
                self.inline_stack.push(self.theme().subscript);
                self.push_element(ElementKind::Subscript);
            }

            Tag::Link { dest_url, .. } => {
                let url = dest_url.into_string();
                self.push_element(ElementKind::Link {
                    alt: String::new(),
                    url: url.clone(),
                });
                self.pending_link_url = Some(url);
                self.link_span_start = Some(self.current_spans.len());
                self.link_col_start = Some(self.current_col);
                self.inline_stack.push(self.theme().link);
            }

            Tag::Image { dest_url, .. } => {
                let url = dest_url.into_string();
                self.push_element(ElementKind::Image {
                    alt: String::new(),
                    url: url.clone(),
                });
                self.pending_image_url = Some(url);
                self.pending_image_alt = Some(String::new());
                self.in_image = true;
                // Don't push to inline_stack yet; we render everything in End.
            }
        }
    }

    // ── End tag handling ──────────────────────────────────────────────────────

    fn handle_end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.block_stack.pop();
                self.pop_element();
                self.commit_line_and_blank();
            }

            TagEnd::Heading(level) => {
                self.block_stack.pop();
                if let Some(f) = &self.renderer.heading {
                    // Hand the accumulated spans to the custom renderer.
                    let spans = self.take_current_spans();
                    let heading = self.current_element();
                    let lines = f(Self::heading_level_u8(level), spans);
                    self.extend_annotated_lines(lines, heading);
                    self.push_blank_line();
                } else {
                    self.commit_line_and_blank();
                }
                self.pop_element();
            }

            TagEnd::BlockQuote(_) => {
                self.block_stack.pop();
                self.pop_element();
                if !self.current_spans.is_empty() {
                    self.commit_line_and_blank();
                }
            }

            TagEnd::CodeBlock => {
                // Pop and extract the buffered CodeBlock context.
                let ctx = self.block_stack.pop();
                if let Some(BlockCtx::CodeBlock { lang, content }) = ctx {
                    let code_block = self.current_element();
                    let lines = if let Some(f) = &self.renderer.code_block {
                        f(&lang, &content)
                    } else {
                        Self::default_code_block_lines(&lang, &content, self.renderer.theme())
                    };
                    self.extend_annotated_lines(lines, code_block);
                    self.push_blank_line();
                }
                self.pop_element();
            }

            TagEnd::List(_) => {
                self.block_stack.pop();
                self.pop_element();
                // Blank line after the outermost list.
                if !self.block_stack.iter().any(|b| matches!(b, BlockCtx::Item)) {
                    self.push_blank_line();
                }
            }

            TagEnd::Item => {
                self.block_stack.pop();
                self.pop_element();
                self.item_depth = self.item_depth.saturating_sub(1);
                if !self.current_spans.is_empty() {
                    self.commit_line();
                }
            }

            TagEnd::Table => {
                let ctx = self.block_stack.pop();
                let table = self.current_element();
                self.pop_element();
                if let Some(BlockCtx::Table(buf)) = ctx {
                    self.render_table(&buf, table);
                    self.push_blank_line();
                }
            }
            TagEnd::TableHead => {
                // Commit the current row as the header.
                let row_cells = self.flush_table_row();
                for ctx in self.block_stack.iter_mut().rev() {
                    if let BlockCtx::Table(buf) = ctx {
                        if !row_cells.is_empty() {
                            buf.header = row_cells;
                        }
                        buf.in_header = false;
                        break;
                    }
                }
                self.block_stack.pop(); // pop TableHead
            }
            TagEnd::TableRow => {
                let row_cells = self.flush_table_row();
                for ctx in self.block_stack.iter_mut().rev() {
                    if let BlockCtx::Table(buf) = ctx {
                        if !row_cells.is_empty() {
                            buf.rows.push(row_cells);
                        }
                        break;
                    }
                }
                self.block_stack.pop(); // pop TableRow
            }
            TagEnd::TableCell => {
                // Commit the current cell into the parent row accumulator.
                let cell = self.flush_table_cell();
                // We'll actually store it in TableRow end; for now just put it
                // into a current_row buffer on the Table.
                for ctx in self.block_stack.iter_mut().rev() {
                    if let BlockCtx::Table(buf) = ctx {
                        buf.current_row.push(cell);
                        break;
                    }
                }
                self.block_stack.pop(); // pop TableCell
            }

            TagEnd::FootnoteDefinition => {
                self.block_stack.pop();
                self.pop_element();
            }

            TagEnd::DefinitionList => {
                self.block_stack.pop();
                self.pop_element();
                self.push_blank_line();
            }
            TagEnd::DefinitionListTitle => {
                self.block_stack.pop();
                self.pop_element();
                self.inline_stack.pop();
                self.commit_line();
            }
            TagEnd::DefinitionListDefinition => {
                self.block_stack.pop();
                self.pop_element();
                self.commit_line();
            }

            TagEnd::MetadataBlock(_) => {
                self.block_stack.pop();
            }

            TagEnd::HtmlBlock => {
                self.block_stack.pop();
                self.pop_element();
                if !self.current_spans.is_empty() {
                    self.commit_line();
                }
            }

            // Inline ends.
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript => {
                self.inline_stack.pop();
                self.pop_element();
            }

            TagEnd::Link => {
                self.inline_stack.pop();
                let url = self.pending_link_url.take().unwrap_or_default();
                let col_start = self.link_col_start.take().unwrap_or(0);
                // Collect the alt-text spans pushed to current_spans since
                // Tag::Link opened (not spans from earlier in the same line).
                if let Some(f) = &self.renderer.link {
                    let start = self.link_span_start.take().unwrap_or(0);
                    // Drain only the spans belonging to this link.
                    let link_spans: Vec<_> = self.current_spans.drain(start..).collect();
                    let alt: String = link_spans.iter().map(|s| s.content.as_ref()).collect();
                    self.patch_current_element(|kind| {
                        if let ElementKind::Link { alt: a, .. } = kind {
                            *a = alt.clone();
                        }
                    });
                    // Drop the drained spans' ranges; the replacement spans
                    // are re-annotated below.
                    let first_dropped = self
                        .current_ranges
                        .iter()
                        .position(|r| r.start >= col_start)
                        .unwrap_or(self.current_ranges.len());
                    self.current_ranges.truncate(first_dropped);
                    self.current_col = col_start;
                    let new_spans = f(&alt, &url);
                    for span in new_spans {
                        self.emit_span(span);
                    }
                } else {
                    let start = self.link_span_start.take().unwrap_or(0);
                    let alt: String = self.current_spans[start..]
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    self.patch_current_element(|kind| {
                        if let ElementKind::Link { alt: a, .. } = kind {
                            *a = alt;
                        }
                    });
                    // Default: append `(url)` after the alt text spans.
                    let style = self.theme().link;
                    self.push_span(format!("({})", url), style);
                }
                self.pop_element();
            }

            TagEnd::Image => {
                self.in_image = false;
                let alt = self.pending_image_alt.take().unwrap_or_default();
                let url = self.pending_image_url.take().unwrap_or_default();
                self.patch_current_element(|kind| {
                    if let ElementKind::Image { alt: a, .. } = kind {
                        *a = alt.clone();
                    }
                });
                let spans = if let Some(f) = &self.renderer.image {
                    f(&alt, &url)
                } else {
                    let style = self.theme().image;
                    vec![Span::styled(format!("🖼 {}({})", alt, url), style)]
                };
                for span in spans {
                    self.emit_span(span);
                }
                self.pop_element();
            }
        }
    }

    // ── Table cell/row helpers ────────────────────────────────────────────────

    /// Drain the `current_cell` buffer from the innermost Table context.
    fn flush_table_cell(&mut self) -> String {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::Table(buf) = ctx {
                let cell = std::mem::take(&mut buf.current_cell);
                return cell;
            }
        }
        String::new()
    }

    /// Drain the `current_row` buffer from the innermost Table context.
    fn flush_table_row(&mut self) -> Vec<String> {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::Table(buf) = ctx {
                return std::mem::take(&mut buf.current_row);
            }
        }
        Vec::new()
    }

    // ── Default code block renderer ──────────────────────────────────────────

    fn default_code_block_lines(
        lang: &str,
        content: &str,
        theme: &crate::Theme,
    ) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::new();
        if !lang.is_empty() {
            out.push(Line::from(Span::styled(
                format!("[{}]", lang),
                theme.code_block_lang,
            )));
        }
        // Split content on newlines; trailing newline from fenced blocks yields
        // an empty final segment — skip it.
        let mut lines = content.split('\n').peekable();
        while let Some(line) = lines.next() {
            // Skip a single empty trailing segment produced by the parser.
            if lines.peek().is_none() && line.is_empty() {
                break;
            }
            out.push(Line::from(Span::styled(line.to_owned(), theme.code_block)));
        }
        out
    }
}

// ── Table row building ────────────────────────────────────────────────────────

/// Build one table row line (cells padded to `col_widths`, joined with
/// `" │ "`) together with its span annotations: cells are tagged with their
/// [`ElementKind::TableCell`] element, separators with the table element.
fn build_table_row(
    cells: &[(String, Style, ElementId)],
    col_widths: &[usize],
    sep_style: Style,
    table: ElementId,
) -> (Line<'static>, Vec<SpanRange>) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut ranges: Vec<SpanRange> = Vec::new();
    let mut col = 0u16;
    for (i, w) in col_widths.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", sep_style));
            ranges.push(SpanRange {
                start: col,
                end: col + 3,
                element: table,
            });
            col += 3;
        }
        let (text, style, el) = &cells[i];
        let padded = pad_cell(text, *w);
        let w = *w as u16;
        spans.push(Span::styled(padded, *style));
        ranges.push(SpanRange {
            start: col,
            end: col + w,
            element: *el,
        });
        col += w;
    }
    (Line::from(spans), ranges)
}

// ── Padding helper ────────────────────────────────────────────────────────────

/// Pad `s` with trailing spaces until its display width equals `width`.
fn pad_cell(s: &str, width: usize) -> String {
    let display_w = UnicodeWidthStr::width(s);
    if display_w < width {
        format!("{}{}", s, " ".repeat(width - display_w))
    } else {
        s.to_owned()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RendererBuilder, Theme};
    use ratatui_core::style::{Color, Modifier, Style};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn plain_text(text: &Text) -> String {
        text.lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn all_spans(text: &Text) -> Vec<(String, Style)> {
        text.lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| (s.content.to_string(), s.style)))
            .collect()
    }

    fn convert(md: &str) -> Text<'static> {
        crate::into_text(md)
    }

    fn convert_with(md: &str, renderer: &Renderer) -> Text<'static> {
        let mut c = Converter::new(renderer);
        c.convert(md).into_text()
    }

    // ── Paragraphs ────────────────────────────────────────────────────────────

    #[test]
    fn plain_paragraph() {
        let text = convert("Hello, world.");
        let spans = all_spans(&text);
        assert!(spans.iter().any(|(c, _)| c == "Hello, world."));
    }

    #[test]
    fn two_paragraphs_have_blank_line_between() {
        let text = convert("First.\n\nSecond.");
        assert!(text.lines.iter().any(|l| l.spans.is_empty()));
    }

    // ── Headings ──────────────────────────────────────────────────────────────

    #[test]
    fn h1_prefix_and_text() {
        let text = convert("# Hello");
        let p = plain_text(&text);
        assert!(p.contains("# "));
        assert!(p.contains("Hello"));
    }

    #[test]
    fn heading_levels_all_prefixes() {
        let md = "# H1\n\n## H2\n\n### H3\n\n#### H4\n\n##### H5\n\n###### H6";
        let p = plain_text(&convert(md));
        for prefix in &["# ", "## ", "### ", "#### ", "##### ", "###### "] {
            assert!(p.contains(prefix), "missing prefix '{prefix}'");
        }
    }

    #[test]
    fn h1_uses_bold() {
        let text = convert("# Bold heading");
        let spans = all_spans(&text);
        let heading: Vec<_> = spans
            .iter()
            .filter(|(c, _)| c == "Bold heading")
            .collect();
        assert!(!heading.is_empty());
        for (_, style) in heading {
            assert!(style.add_modifier.contains(Modifier::BOLD));
        }
    }

    // ── Inline formatting ─────────────────────────────────────────────────────

    #[test]
    fn bold() {
        let spans = all_spans(&convert("Normal **bold** normal."));
        let bold: Vec<_> = spans.iter().filter(|(c, _)| c == "bold").collect();
        assert!(!bold.is_empty());
        for (_, s) in bold {
            assert!(s.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn italic() {
        let spans = all_spans(&convert("Normal _italic_ normal."));
        let it: Vec<_> = spans.iter().filter(|(c, _)| c == "italic").collect();
        assert!(!it.is_empty());
        for (_, s) in it {
            assert!(s.add_modifier.contains(Modifier::ITALIC));
        }
    }

    #[test]
    fn strikethrough() {
        let spans = all_spans(&convert("Normal ~~struck~~ normal."));
        let st: Vec<_> = spans.iter().filter(|(c, _)| c == "struck").collect();
        assert!(!st.is_empty());
        for (_, s) in st {
            assert!(s.add_modifier.contains(Modifier::CROSSED_OUT));
        }
    }

    #[test]
    fn nested_bold_italic() {
        let spans = all_spans(&convert("_italic **bold-italic** italic_"));
        let bi: Vec<_> = spans.iter().filter(|(c, _)| c == "bold-italic").collect();
        assert!(!bi.is_empty());
        for (_, s) in bi {
            assert!(s.add_modifier.contains(Modifier::BOLD));
            assert!(s.add_modifier.contains(Modifier::ITALIC));
        }
    }

    #[test]
    fn superscript_uses_dim() {
        // pulldown-cmark requires the opening ^ to be left-flanking (not
        // immediately preceded by a word character), so "^2^" works but
        // "x^2^" does not — use a space-separated form.
        let spans = all_spans(&convert("footnote ^note^"));
        let sup: Vec<_> = spans.iter().filter(|(c, _)| c == "note").collect();
        assert!(!sup.is_empty(), "no superscript span, got {:?}", spans);
        for (_, s) in sup {
            assert!(s.add_modifier.contains(Modifier::DIM));
        }
    }

    #[test]
    fn subscript_uses_dim() {
        // The closing ~ must also be right-flanking; "~2~ rest" (space after)
        // satisfies both delimiter rules.
        let spans = all_spans(&convert("water ~2~ rest"));
        let sub: Vec<_> = spans.iter().filter(|(c, _)| c == "2").collect();
        assert!(!sub.is_empty(), "no subscript span, got {:?}", spans);
        for (_, s) in sub {
            assert!(s.add_modifier.contains(Modifier::DIM));
        }
    }

    // ── Inline code ───────────────────────────────────────────────────────────

    #[test]
    fn inline_code_theme_color() {
        let renderer = RendererBuilder::new().build();
        let text = convert_with("Use `code` here.", &renderer);
        let spans = all_spans(&text);
        let code: Vec<_> = spans.iter().filter(|(c, _)| c == "code").collect();
        assert!(!code.is_empty());
        for (_, s) in code {
            assert_eq!(s.fg, renderer.theme().inline_code.fg);
        }
    }

    #[test]
    fn inline_code_custom_renderer() {
        let renderer = RendererBuilder::new()
            .with_inline_code(|code| {
                vec![Span::raw(format!("`{code}`"))]
            })
            .build();
        let text = convert_with("Use `foo` here.", &renderer);
        let p = plain_text(&text);
        assert!(p.contains("`foo`"), "custom inline code not applied: {p}");
    }

    // ── Code blocks ───────────────────────────────────────────────────────────

    #[test]
    fn fenced_code_block_content() {
        let p = plain_text(&convert("```\nlet x = 1;\n```"));
        assert!(p.contains("let x = 1;"));
    }

    #[test]
    fn fenced_code_block_language_label() {
        let p = plain_text(&convert("```rust\nlet x = 1;\n```"));
        assert!(p.contains("[rust]"));
        assert!(p.contains("let x = 1;"));
    }

    #[test]
    fn fenced_code_block_custom_renderer() {
        let renderer = RendererBuilder::new()
            .with_code_block(|lang, content| {
                vec![Line::raw(format!("LANG={lang} CODE={content}"))]
            })
            .build();
        let p = plain_text(&convert_with("```python\npass\n```", &renderer));
        assert!(p.contains("LANG=python"), "lang not passed: {p}");
        assert!(p.contains("CODE="), "content not passed: {p}");
    }

    #[test]
    fn code_block_style() {
        let renderer = RendererBuilder::new().build();
        let text = convert_with("```\nhello\n```", &renderer);
        let spans = all_spans(&text);
        let code: Vec<_> = spans.iter().filter(|(c, _)| c == "hello").collect();
        assert!(!code.is_empty());
        for (_, s) in code {
            assert_eq!(s.fg, renderer.theme().code_block.fg);
        }
    }

    // ── Block quotes ──────────────────────────────────────────────────────────

    #[test]
    fn block_quote_prefix() {
        let p = plain_text(&convert("> This is a quote."));
        assert!(p.contains('▌'));
        assert!(p.contains("This is a quote."));
    }

    #[test]
    fn block_quote_style() {
        let renderer = RendererBuilder::new().build();
        let text = convert_with("> quote text", &renderer);
        let spans = all_spans(&text);
        let qs: Vec<_> = spans.iter().filter(|(c, _)| c == "quote text").collect();
        assert!(!qs.is_empty());
        for (_, s) in qs {
            assert_eq!(s.fg, renderer.theme().block_quote.fg);
        }
    }

    // ── Lists ─────────────────────────────────────────────────────────────────

    #[test]
    fn unordered_list() {
        let p = plain_text(&convert("- Apple\n- Banana"));
        assert!(p.contains('•'));
        assert!(p.contains("Apple"));
        assert!(p.contains("Banana"));
    }

    #[test]
    fn ordered_list() {
        let p = plain_text(&convert("1. First\n2. Second\n3. Third"));
        assert!(p.contains("1. "));
        assert!(p.contains("2. "));
        assert!(p.contains("3. "));
    }

    #[test]
    fn task_list() {
        let p = plain_text(&convert("- [x] Done\n- [ ] Todo"));
        assert!(p.contains("[x] "));
        assert!(p.contains("[ ] "));
    }

    // ── Thematic break ────────────────────────────────────────────────────────

    #[test]
    fn thematic_break() {
        let p = plain_text(&convert("Before\n\n---\n\nAfter"));
        assert!(p.contains('─'));
    }

    #[test]
    fn thematic_break_custom_renderer() {
        let renderer = RendererBuilder::new()
            .with_rule(|| vec![Line::raw("=====")])
            .build();
        let p = plain_text(&convert_with("---", &renderer));
        assert!(p.contains("====="), "custom rule not applied: {p}");
    }

    // ── Hard / soft break ─────────────────────────────────────────────────────

    #[test]
    fn hard_break_creates_new_line() {
        let text = convert("Line one  \nLine two");
        let line_contents: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let i1 = line_contents.iter().position(|l| l.contains("Line one"));
        let i2 = line_contents.iter().position(|l| l.contains("Line two"));
        assert!(i1.is_some() && i2.is_some());
        assert_ne!(i1.unwrap(), i2.unwrap());
    }

    #[test]
    fn soft_break_becomes_space() {
        let spans = all_spans(&convert("word1\nword2"));
        let combined: String = spans.iter().map(|(c, _)| c.as_str()).collect();
        assert!(combined.contains("word1") && combined.contains("word2"));
        assert!(spans.iter().any(|(c, _)| c == " "));
    }

    // ── Links ─────────────────────────────────────────────────────────────────

    #[test]
    fn link_default_rendering() {
        let p = plain_text(&convert("[click](https://example.com)"));
        assert!(p.contains("click"), "alt text missing: {p}");
        assert!(p.contains("https://example.com"), "url missing: {p}");
    }

    #[test]
    fn link_alt_text_in_link_style() {
        let renderer = RendererBuilder::new().build();
        let text = convert_with("[click here](https://example.com)", &renderer);
        let spans = all_spans(&text);
        let ls: Vec<_> = spans.iter().filter(|(c, _)| c == "click here").collect();
        assert!(!ls.is_empty());
        for (_, s) in ls {
            assert_eq!(s.fg, renderer.theme().link.fg);
        }
    }

    #[test]
    fn link_custom_renderer_hint_number() {
        let renderer = RendererBuilder::new()
            .with_link(|alt, _url| vec![Span::raw(format!("[{}](42)", alt))])
            .build();
        let p = plain_text(&convert_with("[docs](https://example.com)", &renderer));
        assert!(p.contains("[docs](42)"), "link hint not applied: {p}");
    }

    #[test]
    fn link_custom_renderer_receives_url() {
        let mut received_url = String::new();
        // We can't capture &mut easily in a closure, so use a thread-local.
        use std::cell::RefCell;
        thread_local! {
            static CAPTURED: RefCell<String> = RefCell::new(String::new());
        }
        let renderer = RendererBuilder::new()
            .with_link(|_alt, url| {
                CAPTURED.with(|c| *c.borrow_mut() = url.to_owned());
                vec![Span::raw("link")]
            })
            .build();
        convert_with("[x](https://test.org)", &renderer);
        CAPTURED.with(|c| received_url = c.borrow().clone());
        assert_eq!(received_url, "https://test.org");
    }

    #[test]
    fn multiple_links_in_paragraph_default() {
        // Regression: second link must not absorb the first link's alt text.
        let p = plain_text(&convert("[a](l1) foo [b](l2)"));
        assert!(p.contains("(l1)"), "first url missing: {p}");
        assert!(p.contains("(l2)"), "second url missing: {p}");
        // The text between the links must not be absorbed into a link.
        assert!(p.contains("foo"), "inter-link text missing: {p}");
    }

    #[test]
    fn multiple_links_in_paragraph_custom_renderer() {
        // Regression: with a custom renderer, [a](l1) foo [b](l2) must not
        // produce alt="a(l1) foo b" for the second link.
        let renderer = RendererBuilder::new()
            .with_link(|alt, url| vec![Span::raw(format!("[{alt}]({url})"))])
            .build();
        let p = plain_text(&convert_with("[a](l1) foo [b](l2)", &renderer));
        assert!(p.contains("[a](l1)"), "first link wrong: {p}");
        assert!(p.contains("[b](l2)"), "second link wrong: {p}");
        assert!(!p.contains("[a](l1) foo [b](l2)".replace('(', "").as_str()),
            "alt text must not bleed across links: {p}");
        // Stricter: the second link's alt must be exactly "b", not "a(l1) foo b".
        assert!(!p.contains("a(l1)"), "first link content leaked into second: {p}");
    }

    // ── Images ────────────────────────────────────────────────────────────────

    #[test]
    fn image_default_rendering() {
        let p = plain_text(&convert("![cat](cat.png)"));
        assert!(p.contains("cat"), "alt text missing: {p}");
        assert!(p.contains("cat.png"), "url missing: {p}");
    }

    #[test]
    fn image_custom_renderer() {
        let renderer = RendererBuilder::new()
            .with_image(|alt, url| vec![Span::raw(format!("IMAGE:{alt}@{url}"))])
            .build();
        let p = plain_text(&convert_with("![kitten](kitten.jpg)", &renderer));
        assert!(p.contains("IMAGE:kitten@kitten.jpg"), "custom image renderer not applied: {p}");
    }

    // ── Tables ────────────────────────────────────────────────────────────────

    #[test]
    fn table_header_row_present() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |";
        let p = plain_text(&convert(md));
        assert!(p.contains("Name"), "header 'Name' missing: {p}");
        assert!(p.contains("Age"), "header 'Age' missing: {p}");
    }

    #[test]
    fn table_body_row_present() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |";
        let p = plain_text(&convert(md));
        assert!(p.contains("Alice"), "body cell 'Alice' missing: {p}");
        assert!(p.contains("30"), "body cell '30' missing: {p}");
    }

    #[test]
    fn table_separator_line_present() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let text = convert(md);
        // The separator line should contain '─' and '┼'.
        let has_sep = text.lines.iter().any(|l| {
            l.spans.iter().any(|s| s.content.contains('─'))
                && l.spans.iter().any(|s| s.content.contains('┼'))
        });
        assert!(has_sep, "table separator line missing");
    }

    #[test]
    fn table_header_uses_bold_style() {
        let renderer = RendererBuilder::new().build();
        let text = convert_with(
            "| Col |\n|-----|\n| val |",
            &renderer,
        );
        let spans = all_spans(&text);
        let header: Vec<_> = spans.iter().filter(|(c, _)| c.trim() == "Col").collect();
        assert!(!header.is_empty(), "header span not found, spans: {:?}", spans);
        for (_, s) in header {
            assert!(
                s.add_modifier.contains(Modifier::BOLD),
                "table header should be BOLD, got {:?}", s
            );
        }
    }

    // ── Footnotes ─────────────────────────────────────────────────────────────

    #[test]
    fn footnote_reference_rendered() {
        let md = "See note[^1].\n\n[^1]: The note text.";
        let p = plain_text(&convert(md));
        assert!(p.contains("[^1]"), "footnote ref missing: {p}");
    }

    #[test]
    fn footnote_definition_rendered_at_end() {
        let md = "See[^1].\n\n[^1]: Footnote content.";
        let p = plain_text(&convert(md));
        assert!(p.contains("Footnote content"), "footnote def missing: {p}");
    }

    #[test]
    fn footnote_ref_custom_renderer() {
        let renderer = RendererBuilder::new()
            .with_footnote_ref(|label| vec![Span::raw(format!("(note {label})"))])
            .build();
        let p = plain_text(&convert_with("Text[^abc].\n\n[^abc]: Content.", &renderer));
        assert!(p.contains("(note abc)"), "custom footnote ref not applied: {p}");
    }

    // ── Inline HTML ───────────────────────────────────────────────────────────

    #[test]
    fn inline_html_verbatim() {
        let p = plain_text(&convert("Some <em>html</em> here."));
        assert!(p.contains("html"), "html content missing: {p}");
    }

    // ── Custom theme ──────────────────────────────────────────────────────────

    #[test]
    fn custom_theme_h1_color() {
        let mut theme = Theme::default();
        theme.h1 = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
        let renderer = RendererBuilder::new().with_theme(theme).build();
        let text = convert_with("# Title", &renderer);
        let spans = all_spans(&text);
        let title: Vec<_> = spans.iter().filter(|(c, _)| c == "Title").collect();
        assert!(!title.is_empty());
        for (_, s) in title {
            assert_eq!(s.fg, Some(Color::Red));
        }
    }

    // ── Custom heading renderer ───────────────────────────────────────────────

    #[test]
    fn custom_heading_renderer() {
        let renderer = RendererBuilder::new()
            .with_heading(|level, spans| {
                let content: String = spans.iter().map(|s| s.content.as_ref()).collect();
                vec![Line::raw(format!("H{level}: {content}"))]
            })
            .build();
        let p = plain_text(&convert_with("# My Title", &renderer));
        assert!(p.contains("H1: My Title"), "custom heading not applied: {p}");
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn empty_input() {
        let text = convert("");
        assert!(text.lines.is_empty());
    }

    #[test]
    fn whitespace_only_input() {
        let _ = convert("   \n\n  ");
    }

    #[test]
    fn nested_list_items_on_separate_lines() {
        // Regression: nested items must not appear on the same line as the parent.
        let text = convert("- parent\n  - child\n  - sibling\n- next");
        let lines: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .filter(|s: &String| !s.trim().is_empty())
            .collect();
        // Each item must be on its own line.
        assert!(
            lines.iter().any(|l| l.contains("parent")),
            "parent not found: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("child")),
            "child not found: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("sibling")),
            "sibling not found: {lines:?}"
        );
        // Parent and child must NOT share a line.
        assert!(
            !lines.iter().any(|l| l.contains("parent") && l.contains("child")),
            "parent and child on same line: {lines:?}"
        );
        // The two sibling nested items must be on different lines.
        assert!(
            !lines.iter().any(|l| l.contains("child") && l.contains("sibling")),
            "child and sibling on same line: {lines:?}"
        );
    }
}
