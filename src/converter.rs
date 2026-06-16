//! Core event-driven converter: walks `pulldown-cmark` events and builds
//! `ratatui_core::text::Text`.

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui_core::style::Style;
use ratatui_core::text::{Line, Span, Text};
use unicode_width::UnicodeWidthStr;

use crate::Renderer;

// ── Block context stack ───────────────────────────────────────────────────────

/// Describes the block-level context we are currently inside.
#[derive(Debug, Clone)]
enum BlockCtx {
    Paragraph,
    Heading(HeadingLevel),
    CodeBlock {
        lang: String,
        content: String,
    },
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
    /// Header cells, each a `Vec<Span>` holding styled inline content.
    header: Vec<Vec<Span<'static>>>,
    /// Body rows, each a Vec of cells, each cell a Vec of styled spans.
    rows: Vec<Vec<Vec<Span<'static>>>>,
    /// Current cell being accumulated.
    current_cell: Vec<Span<'static>>,
    /// Current partial row being accumulated.
    current_row: Vec<Vec<Span<'static>>>,
}

impl TableBuf {
    fn new() -> Self {
        Self {
            header: Vec::new(),
            rows: Vec::new(),
            current_cell: Vec::new(),
            current_row: Vec::new(),
        }
    }
}

// ── Footnote definition buffering ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct FootnoteDef {
    label: String,
    content: String,
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
    /// Index into the active table cell at which the current link's content
    /// begins. Used to extract only the link's own spans when the link closes.
    table_link_span_start: Option<usize>,
    /// Alt text accumulated while inside `Tag::Image`.
    pending_image_alt: Option<String>,
    /// URL stashed when we enter `Tag::Image`.
    pending_image_url: Option<String>,
    /// Whether we are currently inside an image (so text goes to alt buffer).
    in_image: bool,
    /// Footnote definitions collected during the parse (rendered at the end).
    footnote_defs: Vec<FootnoteDef>,
    markdown: String,
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
            table_link_span_start: None,
            pending_image_alt: None,
            pending_image_url: None,
            in_image: false,
            footnote_defs: Vec::new(),
            markdown: String::new(),
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

    fn push_span(&mut self, content: impl Into<String>, style: Style) {
        let content = content.into();
        if !content.is_empty() {
            self.current_spans.push(Span::styled(content, style));
        }
    }

    fn commit_line(&mut self) {
        let spans = std::mem::take(&mut self.current_spans);
        self.lines.push(Line::from(spans));
    }

    fn commit_line_and_blank(&mut self) {
        self.commit_line();
        self.lines.push(Line::default());
    }

    /// True if the innermost block context is an active table cell.
    fn is_table_cell(&self) -> bool {
        matches!(self.block_stack.last(), Some(BlockCtx::TableCell))
    }

    /// True if we're inside a block context that produces no visible output.
    fn is_metadata(&self) -> bool {
        self.block_stack
            .last()
            .map_or(false, |b| matches!(b, BlockCtx::MetadataBlock))
    }

    fn push_table_cell_span(&mut self, span: Span<'static>) {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::Table(buf) = ctx {
                buf.current_cell.push(span);
                return;
            }
        }
    }

    // ── Table rendering ───────────────────────────────────────────────────────

    /// Sum display widths of spans in a cell.
    fn cell_width(cell: &[Span<'static>]) -> usize {
        cell.iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum()
    }

    fn render_table(buf: &TableBuf, theme: &crate::Theme) -> Vec<Line<'static>> {
        let ncols = buf
            .header
            .len()
            .max(buf.rows.iter().map(|r| r.len()).max().unwrap_or(0));

        if ncols == 0 {
            return Vec::new();
        }

        // Compute column widths (display width).
        let mut col_widths: Vec<usize> = (0..ncols)
            .map(|i| buf.header.get(i).map(|h| Self::cell_width(h)).unwrap_or(0))
            .collect();
        for row in &buf.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < ncols {
                    col_widths[i] = col_widths[i].max(Self::cell_width(cell));
                }
            }
        }
        // Minimum column width of 1.
        for w in &mut col_widths {
            *w = (*w).max(1);
        }

        let mut out: Vec<Line<'static>> = Vec::new();

        // Header row.
        {
            let mut line_spans: Vec<Span<'static>> = Vec::new();
            for (i, w) in col_widths.iter().enumerate() {
                if i > 0 {
                    line_spans.push(Span::styled(" │ ", theme.table_separator));
                }
                let cell = buf.header.get(i).map(Vec::as_slice).unwrap_or_default();
                let padded = Self::pad_cell_spans(cell, *w, theme.table_header);
                line_spans.extend(padded);
            }
            out.push(Line::from(line_spans));
        }

        // Separator row: ─────┼───── for each column.
        {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (i, w) in col_widths.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled("─┼─", theme.table_separator));
                }
                spans.push(Span::styled("─".repeat(*w), theme.table_separator));
            }
            out.push(Line::from(spans));
        }

        // Body rows.
        for row in &buf.rows {
            let mut line_spans: Vec<Span<'static>> = Vec::new();
            for (i, w) in col_widths.iter().enumerate() {
                if i > 0 {
                    line_spans.push(Span::styled(" │ ", theme.table_separator));
                }
                let cell = row.get(i).map(Vec::as_slice).unwrap_or_default();
                let padded = Self::pad_cell_spans(cell, *w, theme.table_cell);
                line_spans.extend(padded);
            }
            out.push(Line::from(line_spans));
        }

        out
    }

    /// Pad a cell (a sequence of spans) to `width` display width by appending a
    /// whitespace span with the supplied `base` style. The base style is patched
    /// onto every span so header cells, for example, become bold without
    /// losing inline foreground colors.
    fn pad_cell_spans(cell: &[Span<'static>], width: usize, base: Style) -> Vec<Span<'static>> {
        let mut out: Vec<Span<'static>> = cell
            .iter()
            .map(|s| Span {
                content: s.content.clone(),
                style: s.style.patch(base),
            })
            .collect();
        let cell_w = Self::cell_width(&out);
        if cell_w < width {
            out.push(Span::styled(" ".repeat(width - cell_w), base));
        }
        out
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

    fn extract_orig_list_number(&self, offset: usize) -> Option<u64> {
        let src = &self.markdown;
        if offset >= src.len() {
            return None;
        }
        let line_start = src[..offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line = src[line_start..].lines().next()?;
        let mut trimmed = line.trim_start();
        while let Some(rest) = trimmed.strip_prefix('>') {
            trimmed = rest.trim_start();
        }
        let digits_end = trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len());
        if digits_end == 0 {
            return None;
        }
        let digits = &trimmed[..digits_end];
        let after = &trimmed[digits_end..];
        if after.starts_with(". ")
            || after.starts_with(".\t")
            || after == "."
            || after.starts_with(") ")
            || after.starts_with(")\t")
            || after == ")"
        {
            digits.parse().ok()
        } else {
            None
        }
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
        self.lines.push(Line::default());
        self.lines
            .push(Line::from(Span::styled("─".repeat(40), self.theme().rule)));
        let defs = std::mem::take(&mut self.footnote_defs);
        for def in &defs {
            let label_span = Span::styled(format!("[{}]: ", def.label), self.theme().footnote_def);
            let content_span = Span::styled(def.content.clone(), self.theme().footnote_def);
            self.lines.push(Line::from(vec![label_span, content_span]));
        }
    }

    // ── Main convert entry-point ──────────────────────────────────────────────

    pub(crate) fn convert(&mut self, markdown: &str) -> Text<'static> {
        self.markdown = markdown.to_owned();

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

        for (event, range) in parser.into_offset_iter() {
            self.handle_event(event, range.start);
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
        }

        let mut text = Text::from(std::mem::take(&mut self.lines));
        text.style = self.theme().base;
        text
    }

    // ── Event dispatch ────────────────────────────────────────────────────────

    fn handle_event(&mut self, event: Event<'_>, offset: usize) {
        match event {
            Event::Start(tag) => self.handle_start(tag, offset),
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
                // Inside a table cell: push a styled span into the cell buffer.
                if self.is_table_cell() {
                    let span = Span::styled(s, self.current_style());
                    self.push_table_cell_span(span);
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
                if self.is_metadata() {
                    return;
                }
                let code_str = code.into_string();
                let spans = if let Some(f) = &self.renderer.inline_code {
                    f(&code_str)
                } else {
                    let style = self.current_style().patch(self.theme().inline_code);
                    vec![Span::styled(code_str, style)]
                };
                if self.is_table_cell() {
                    for span in spans {
                        self.push_table_cell_span(span);
                    }
                    return;
                }
                for span in spans {
                    self.current_spans.push(span);
                }
            }

            Event::InlineMath(math) | Event::DisplayMath(math) => {
                if self.is_metadata() {
                    return;
                }
                let style = self.current_style().patch(self.theme().math);
                let span = Span::styled(math.into_string(), style);
                if self.is_table_cell() {
                    self.push_table_cell_span(span);
                    return;
                }
                self.push_span(span.content, span.style);
            }

            Event::SoftBreak => {
                if self.is_metadata() || self.in_image {
                    return;
                }
                if self.is_table_cell() {
                    let span = Span::styled(" ".to_string(), self.current_style());
                    self.push_table_cell_span(span);
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
                if self.is_metadata() || self.in_image {
                    return;
                }
                if self.is_table_cell() {
                    let span = Span::styled(" ".to_string(), self.current_style());
                    self.push_table_cell_span(span);
                    return;
                }
                self.commit_line();
            }

            Event::Rule => {
                self.commit_line();
                let lines = if let Some(f) = &self.renderer.rule {
                    f()
                } else {
                    vec![Line::from(Span::styled("─".repeat(40), self.theme().rule))]
                };
                self.lines.extend(lines);
                self.lines.push(Line::default());
            }

            Event::Html(html) | Event::InlineHtml(html) => {
                if self.is_metadata() {
                    return;
                }
                let style = self.theme().html;
                let s = html.into_string();
                let mut iter = s.split('\n').peekable();
                while let Some(line) = iter.next() {
                    let span = Span::styled(line.trim_end_matches('\r').to_string(), style);
                    if self.is_table_cell() {
                        self.push_table_cell_span(span);
                    } else {
                        self.push_span(span.content, span.style);
                    }
                    if iter.peek().is_some() && !self.is_table_cell() {
                        self.commit_line();
                    }
                }
            }

            Event::TaskListMarker(checked) => {
                if self.is_metadata() {
                    return;
                }
                let marker = if checked { "[x] " } else { "[ ] " };
                if self.is_table_cell() {
                    let span = Span::styled(marker.to_string(), self.current_style());
                    self.push_table_cell_span(span);
                    return;
                }
                self.push_span(marker, self.current_style());
            }

            Event::FootnoteReference(label) => {
                let label = label.into_string();
                let spans = if let Some(f) = &self.renderer.footnote_ref {
                    f(&label)
                } else {
                    vec![Span::styled(
                        format!("[^{}]", label),
                        self.theme().footnote_ref,
                    )]
                };
                if self.is_table_cell() {
                    for span in spans {
                        self.push_table_cell_span(span);
                    }
                    return;
                }
                for span in spans {
                    self.current_spans.push(span);
                }
            }
        }
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

    fn handle_start(&mut self, tag: Tag<'_>, offset: usize) {
        match tag {
            Tag::Paragraph => {
                self.block_stack.push(BlockCtx::Paragraph);
            }

            Tag::Heading { level, .. } => {
                self.block_stack.push(BlockCtx::Heading(level));
                // If there's no custom heading renderer, push the prefix span immediately.
                if self.renderer.heading.is_none() {
                    let style = self.heading_style(level);
                    self.push_span(Self::heading_prefix(level), style);
                }
            }

            Tag::BlockQuote(kind) => {
                self.block_stack.push(BlockCtx::BlockQuote(kind));
                let style = self.blockquote_style(kind);
                self.push_span("▌ ", style);
            }

            Tag::CodeBlock(kind) => {
                let lang = match &kind {
                    CodeBlockKind::Fenced(l) => l.trim().to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.block_stack.push(BlockCtx::CodeBlock {
                    lang,
                    content: String::new(),
                });
            }

            Tag::List(start) => {
                if !self.current_spans.is_empty() {
                    self.commit_line();
                }
                if let Some(n) = start {
                    self.block_stack.push(BlockCtx::OrderedList(n));
                } else {
                    self.block_stack.push(BlockCtx::BulletList);
                }
            }

            Tag::Item => {
                self.item_depth += 1;
                self.block_stack.push(BlockCtx::Item);
                if self
                    .block_stack
                    .iter()
                    .rev()
                    .any(|b| matches!(b, BlockCtx::OrderedList(_)))
                    && let Some(orig) = self.extract_orig_list_number(offset)
                {
                    for ctx in self.block_stack.iter_mut().rev() {
                        if let BlockCtx::OrderedList(n) = ctx {
                            *n = orig;
                            break;
                        }
                    }
                }
                let indent = self.list_indent();
                let marker = self.make_item_marker();
                let style = self.theme().list_marker;
                self.push_span(format!("{}{}", indent, marker), style);
                self.advance_list_counter();
            }

            Tag::Table(_) => {
                self.block_stack.push(BlockCtx::Table(TableBuf::new()));
            }
            Tag::TableHead => {
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
                self.footnote_defs.push(FootnoteDef {
                    label: label.clone(),
                    content: String::new(),
                });
                self.block_stack.push(BlockCtx::FootnoteDef(label));
            }

            Tag::DefinitionList => {
                self.block_stack.push(BlockCtx::DefinitionList);
            }
            Tag::DefinitionListTitle => {
                self.block_stack.push(BlockCtx::DefinitionListTitle);
                // Render title as bold.
                self.inline_stack.push(self.theme().strong);
            }
            Tag::DefinitionListDefinition => {
                self.block_stack.push(BlockCtx::DefinitionListDefinition);
                self.push_span("  ", Style::default());
            }

            Tag::MetadataBlock(_) => {
                self.block_stack.push(BlockCtx::MetadataBlock);
            }

            Tag::HtmlBlock => {
                self.block_stack.push(BlockCtx::HtmlBlock);
            }

            // Inline tags.
            Tag::Emphasis => self.inline_stack.push(self.theme().emphasis),
            Tag::Strong => self.inline_stack.push(self.theme().strong),
            Tag::Strikethrough => self.inline_stack.push(self.theme().strikethrough),
            Tag::Superscript => self.inline_stack.push(self.theme().superscript),
            Tag::Subscript => self.inline_stack.push(self.theme().subscript),

            Tag::Link { dest_url, .. } => {
                self.pending_link_url = Some(dest_url.into_string());
                self.link_span_start = Some(self.current_spans.len());
                if self.is_table_cell() {
                    for ctx in self.block_stack.iter_mut().rev() {
                        if let BlockCtx::Table(buf) = ctx {
                            self.table_link_span_start = Some(buf.current_cell.len());
                            break;
                        }
                    }
                }
                self.inline_stack.push(self.theme().link);
            }

            Tag::Image { dest_url, .. } => {
                self.pending_image_url = Some(dest_url.into_string());
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
                self.commit_line_and_blank();
            }

            TagEnd::Heading(level) => {
                self.block_stack.pop();
                if let Some(f) = &self.renderer.heading {
                    // Hand the accumulated spans to the custom renderer.
                    let spans = std::mem::take(&mut self.current_spans);
                    let lines = f(Self::heading_level_u8(level), spans);
                    self.lines.extend(lines);
                    self.lines.push(Line::default());
                } else {
                    self.commit_line_and_blank();
                }
            }

            TagEnd::BlockQuote(_) => {
                self.block_stack.pop();
                if !self.current_spans.is_empty() {
                    self.commit_line_and_blank();
                }
            }

            TagEnd::CodeBlock => {
                // Pop and extract the buffered CodeBlock context.
                let ctx = self.block_stack.pop();
                if let Some(BlockCtx::CodeBlock { lang, content }) = ctx {
                    let lines = if let Some(f) = &self.renderer.code_block {
                        f(&lang, &content)
                    } else {
                        Self::default_code_block_lines(&lang, &content, self.renderer.theme())
                    };
                    self.lines.extend(lines);
                    self.lines.push(Line::default());
                }
            }

            TagEnd::List(_) => {
                self.block_stack.pop();
                if !self.block_stack.iter().any(|b| matches!(b, BlockCtx::Item)) {
                    self.lines.push(Line::default());
                }
            }

            TagEnd::Item => {
                self.block_stack.pop();
                self.item_depth = self.item_depth.saturating_sub(1);
                if !self.current_spans.is_empty() {
                    self.commit_line();
                }
            }

            TagEnd::Table => {
                let ctx = self.block_stack.pop();
                if let Some(BlockCtx::Table(buf)) = ctx {
                    let lines = if let Some(f) = &self.renderer.table {
                        f(&buf.header, &buf.rows, self.renderer.theme())
                    } else {
                        Self::render_table(&buf, self.renderer.theme())
                    };
                    self.lines.extend(lines);
                    self.lines.push(Line::default());
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
            }

            TagEnd::DefinitionList => {
                self.block_stack.pop();
                self.lines.push(Line::default());
            }
            TagEnd::DefinitionListTitle => {
                self.block_stack.pop();
                self.inline_stack.pop();
                self.commit_line();
            }
            TagEnd::DefinitionListDefinition => {
                self.block_stack.pop();
                self.commit_line();
            }

            TagEnd::MetadataBlock(_) => {
                self.block_stack.pop();
            }

            TagEnd::HtmlBlock => {
                self.block_stack.pop();
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
            }

            TagEnd::Link => {
                self.inline_stack.pop();
                let url = self.pending_link_url.take().unwrap_or_default();
                if self.is_table_cell() {
                    let link_style = self.theme().link;
                    for ctx in self.block_stack.iter_mut().rev() {
                        if let BlockCtx::Table(buf) = ctx {
                            let start = self
                                .table_link_span_start
                                .take()
                                .unwrap_or(buf.current_cell.len());
                            if let Some(f) = &self.renderer.link {
                                let alt: String = buf
                                    .current_cell
                                    .iter()
                                    .skip(start)
                                    .map(|s| s.content.as_ref())
                                    .collect();
                                let new_spans = f(&alt, &url);
                                buf.current_cell.truncate(start);
                                buf.current_cell.extend(new_spans);
                            } else {
                                buf.current_cell
                                    .push(Span::styled(format!("({})", url), link_style));
                            }
                            break;
                        }
                    }
                    return;
                }
                // Non-table path.
                if let Some(f) = &self.renderer.link {
                    let start = self.link_span_start.take().unwrap_or(0);
                    let drained: Vec<_> = self.current_spans.drain(start..).collect();
                    let alt: String = drained.iter().map(|s| s.content.as_ref()).collect();
                    let new_spans = f(&alt, &url);
                    self.current_spans.extend(new_spans);
                } else {
                    self.link_span_start = None;
                    let style = self.theme().link;
                    self.push_span(format!("({})", url), style);
                }
            }

            TagEnd::Image => {
                self.in_image = false;
                let alt = self.pending_image_alt.take().unwrap_or_default();
                let url = self.pending_image_url.take().unwrap_or_default();
                let spans = if let Some(f) = &self.renderer.image {
                    f(&alt, &url)
                } else {
                    let style = self.theme().image;
                    vec![Span::styled(format!("🖼 {}({})", alt, url), style)]
                };
                if self.is_table_cell() {
                    for span in spans {
                        self.push_table_cell_span(span);
                    }
                } else {
                    for span in spans {
                        self.current_spans.push(span);
                    }
                }
            }
        }
    }

    // ── Table cell/row helpers ────────────────────────────────────────────────

    /// Drain the `current_cell` buffer from the innermost Table context.
    fn flush_table_cell(&mut self) -> Vec<Span<'static>> {
        for ctx in self.block_stack.iter_mut().rev() {
            if let BlockCtx::Table(buf) = ctx {
                return std::mem::take(&mut buf.current_cell);
            }
        }
        Vec::new()
    }

    /// Drain the `current_row` buffer from the innermost Table context.
    fn flush_table_row(&mut self) -> Vec<Vec<Span<'static>>> {
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
        c.convert(md)
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
        let heading: Vec<_> = spans.iter().filter(|(c, _)| c == "Bold heading").collect();
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
            .with_inline_code(|code| vec![Span::raw(format!("`{code}`"))])
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
            .with_code_block(|lang, content| vec![Line::raw(format!("LANG={lang} CODE={content}"))])
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
        assert!(
            !p.contains("[a](l1) foo [b](l2)".replace('(', "").as_str()),
            "alt text must not bleed across links: {p}"
        );
        // Stricter: the second link's alt must be exactly "b", not "a(l1) foo b".
        assert!(
            !p.contains("a(l1)"),
            "first link content leaked into second: {p}"
        );
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
        assert!(
            p.contains("IMAGE:kitten@kitten.jpg"),
            "custom image renderer not applied: {p}"
        );
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
        let text = convert_with("| Col |\n|-----|\n| val |", &renderer);
        let spans = all_spans(&text);
        // The padded "Col" span plus padding spaces share the header style.
        let header: Vec<_> = spans.iter().filter(|(c, _)| c.trim() == "Col").collect();
        assert!(
            !header.is_empty(),
            "header span not found, spans: {:?}",
            spans
        );
        for (_, s) in header {
            assert!(
                s.add_modifier.contains(Modifier::BOLD),
                "table header should be BOLD, got {:?}",
                s
            );
        }
    }

    #[test]
    fn table_cell_inline_code_in_header_and_body() {
        let renderer = RendererBuilder::new().build();
        let md = "| `cmd` | Name |\n|-------|------|\n| `x` | value |";
        let text = convert_with(md, &renderer);
        let spans = all_spans(&text);

        let code_spans: Vec<_> = spans
            .iter()
            .filter(|(c, _)| c == "cmd" || c == "x")
            .collect();
        assert!(
            !code_spans.is_empty(),
            "inline code spans missing, got {:?}",
            spans
        );
        for (_, s) in &code_spans {
            assert_eq!(s.fg, renderer.theme().inline_code.fg, "code span fg wrong");
        }
    }

    #[test]
    fn table_cell_bold_and_italic() {
        let renderer = RendererBuilder::new().build();
        let md = "| Text |\n|------|\n| **bold** _italic_ |";
        let text = convert_with(md, &renderer);
        let spans = all_spans(&text);

        let bold: Vec<_> = spans.iter().filter(|(c, _)| c == "bold").collect();
        assert!(!bold.is_empty(), "bold span missing");
        for (_, s) in &bold {
            assert!(s.add_modifier.contains(Modifier::BOLD));
        }

        let italic: Vec<_> = spans.iter().filter(|(c, _)| c == "italic").collect();
        assert!(!italic.is_empty(), "italic span missing");
        for (_, s) in &italic {
            assert!(s.add_modifier.contains(Modifier::ITALIC));
        }
    }

    #[test]
    fn table_cell_link_default() {
        let text = convert("| A |\n|---|\n| [click](https://example.com) |");
        let p = plain_text(&text);
        assert!(p.contains("click"), "link alt missing: {p}");
        assert!(p.contains("https://example.com"), "link url missing: {p}");
    }

    #[test]
    fn table_cell_link_surrounded_by_text() {
        let text = convert("| A |\n|---|\n| pre [click](https://example.com) post |");
        let p = plain_text(&text);
        assert!(p.contains("pre"), "pre-link text missing: {p}");
        assert!(p.contains("click"), "link alt missing: {p}");
        assert!(p.contains("https://example.com"), "link url missing: {p}");
        assert!(p.contains("post"), "post-link text missing: {p}");
    }

    #[test]
    fn table_cell_link_custom_renderer_receives_spans() {
        use std::cell::RefCell;
        thread_local! {
            static CAPTURED: RefCell<Vec<String>> = RefCell::new(Vec::new());
        }
        CAPTURED.with(|c| c.borrow_mut().clear());
        let renderer = RendererBuilder::new()
            .with_link(|alt, url| {
                CAPTURED.with(|c| c.borrow_mut().push(format!("{alt}@{url}")));
                vec![Span::raw(format!("[{alt}]({url})"))]
            })
            .build();
        let text = convert_with("| A |\n|---|\n| [x](u) |", &renderer);
        let p = plain_text(&text);
        assert!(p.contains("[x](u)"), "custom link not rendered: {p}");
        CAPTURED.with(|c| {
            assert!(
                c.borrow().iter().any(|s| s == "x@u"),
                "custom link not invoked"
            );
        });
    }

    #[test]
    fn table_cell_image_default() {
        let text = convert("| A |\n|---|\n| ![cat](cat.png) |");
        let p = plain_text(&text);
        assert!(p.contains("cat"), "image alt missing: {p}");
        assert!(p.contains("cat.png"), "image url missing: {p}");
    }

    #[test]
    fn table_cell_width_with_inline_code() {
        let renderer = RendererBuilder::new().build();
        let md = "| `abc` |\n|-------|\n| x |";
        let text = convert_with(md, &renderer);
        let plain = plain_text(&text);
        // Separator line for a single-column table has only ─ characters.
        let sep_line = text
            .lines
            .iter()
            .find(|l| {
                !l.spans.is_empty() && l.spans.iter().all(|s| s.content.chars().all(|c| c == '─'))
            })
            .expect("separator line");
        let sep_text: String = sep_line.spans.iter().map(|s| s.content.as_ref()).collect();
        // The code cell separator should be at least 3 chars of ─.
        assert!(sep_text.len() >= 3, "separator too short: {sep_text}");
        assert!(plain.contains("abc"), "code content missing: {plain}");
    }

    #[test]
    fn table_custom_renderer_receives_styled_spans() {
        let renderer = RendererBuilder::new()
            .with_table(|header, rows, _theme| {
                assert!(!header.is_empty());
                // Inspect the first header cell: it should contain the inline
                // spans accumulated while parsing the cell.
                let first_cell = &header[0];
                assert!(!first_cell.is_empty());
                // The combined content should be "Name".
                let content: String = first_cell.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(content, "Name");
                // One row should be present.
                assert_eq!(rows.len(), 1);
                vec![Line::raw(format!("ROWS={}", rows.len()))]
            })
            .build();
        let text = convert_with("| Name |\n|------|\n| Alice |", &renderer);
        let p = plain_text(&text);
        assert!(
            p.contains("ROWS=1"),
            "custom table renderer output missing: {p}"
        );
    }

    #[test]
    fn table_cell_other_inline_elements() {
        let renderer = RendererBuilder::new().build();
        let md =
            "| A | B | C | D | E |\n|---|---|---|---|---|\n| ~~s~~ | ^sup^ | ~sub~ | $x$ | [^1] |";
        let text = convert_with(md, &renderer);
        let p = plain_text(&text);
        assert!(p.contains('s'), "strikethrough text missing: {p}");
        assert!(p.contains("sup"), "superscript text missing: {p}");
        assert!(p.contains("sub"), "subscript text missing: {p}");
        assert!(p.contains('x'), "math text missing: {p}");
        assert!(p.contains("[^1]"), "footnote ref missing: {p}");
    }

    #[test]
    fn table_cell_width_with_empty_cell() {
        let renderer = RendererBuilder::new().build();
        let md = "| A | B |\n|---|---|\n| | x |";
        let text = convert_with(md, &renderer);
        // Should render without panicking; separator exists.
        assert!(
            text.lines
                .iter()
                .any(|l| { l.spans.iter().any(|s| s.content.contains('─')) })
        );
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
        assert!(
            p.contains("(note abc)"),
            "custom footnote ref not applied: {p}"
        );
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
        assert!(
            p.contains("H1: My Title"),
            "custom heading not applied: {p}"
        );
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
            !lines
                .iter()
                .any(|l| l.contains("parent") && l.contains("child")),
            "parent and child on same line: {lines:?}"
        );
        // The two sibling nested items must be on different lines.
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("child") && l.contains("sibling")),
            "child and sibling on same line: {lines:?}"
        );
    }
    #[test]
    fn custom_table_renderer_receives_cells() {
        let renderer = RendererBuilder::new()
            .with_table(|header, rows, _theme| {
                let mut lines = Vec::new();
                let header_text: Vec<String> = header
                    .iter()
                    .map(|cell| cell.iter().map(|s| s.content.as_ref()).collect())
                    .collect();
                lines.push(Line::raw(format!("HDR={}", header_text.join(","))));
                for row in rows {
                    let row_text: Vec<String> = row
                        .iter()
                        .map(|cell| cell.iter().map(|s| s.content.as_ref()).collect())
                        .collect();
                    lines.push(Line::raw(format!("ROW={}", row_text.join(","))));
                }
                lines
            })
            .build();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let text = convert_with(md, &renderer);
        let p = plain_text(&text);
        assert!(p.contains("HDR=A,B"), "custom header not rendered: {p}");
        assert!(p.contains("ROW=1,2"), "custom body not rendered: {p}");
    }

    #[test]
    fn custom_table_renderer_overrides_default() {
        let renderer = RendererBuilder::new()
            .with_table(|_header, _rows, _theme| vec![Line::raw("CUSTOM_TABLE")])
            .build();
        let md = "| X | Y |\n|---|---|\n| 1 | 2 |";
        let text = convert_with(md, &renderer);
        let p = plain_text(&text);
        assert!(
            p.contains("CUSTOM_TABLE"),
            "custom table renderer should override default: {p}"
        );
        // Should NOT contain default separator characters
        assert!(!p.contains('┼'), "default separator should not appear: {p}");
    }

    // ── Ordered list number preservation ─────────────────────────────────────

    #[test]
    fn non_sequential_ordered_list_preserves_numbers() {
        let md = "2. alpha\n4. beta\n8. gamma";
        let p = plain_text(&convert(md));
        assert_eq!(p, "2. alpha\n4. beta\n8. gamma");
    }

    #[test]
    fn sequential_ordered_list_unchanged() {
        let md = "1. first\n2. second\n3. third";
        let p = plain_text(&convert(md));
        assert_eq!(p, "1. first\n2. second\n3. third");
    }

    #[test]
    fn start_number_greater_than_one() {
        let md = "5. alpha\n6. beta";
        let p = plain_text(&convert(md));
        assert_eq!(p, "5. alpha\n6. beta");
    }

    #[test]
    fn nested_non_sequential_ordered_lists() {
        let md = "2. outer-a\n4. outer-b\n   3. inner-x\n   5. inner-y\n8. outer-c";
        let p = plain_text(&convert(md));
        assert!(p.contains("2. outer-a"), "outer item 2: {p}");
        assert!(p.contains("4. outer-b"), "outer item 4: {p}");
        assert!(p.contains("3. inner-x"), "inner item 3: {p}");
        assert!(p.contains("5. inner-y"), "inner item 5: {p}");
        assert!(p.contains("8. outer-c"), "outer item 8: {p}");
        assert!(!p.contains("3. outer"), "should not renumber: {p}");
    }

    #[test]
    fn mixed_ordered_and_unordered_nesting() {
        let md = "2. first\n4. second\n   - bullet-a\n   - bullet-b\n8. third";
        let text = convert(md);
        let lines: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .filter(|s: &String| !s.trim().is_empty())
            .collect();
        assert!(lines[0].contains("2. first"), "line 0: {:?}", lines);
        assert!(lines[1].contains("4. second"), "line 1: {:?}", lines);
        assert!(lines[2].contains("• bullet-a"), "line 2: {:?}", lines);
        assert!(lines[3].contains("• bullet-b"), "line 3: {:?}", lines);
        assert!(lines[4].contains("8. third"), "line 4: {:?}", lines);
    }

    #[test]
    fn paren_ordered_list_marker() {
        let md = "2) alpha\n4) beta\n8) gamma";
        let p = plain_text(&convert(md));
        assert!(p.contains("2. "), "expected '2. ': {p}");
        assert!(p.contains("4. "), "expected '4. ': {p}");
        assert!(p.contains("8. "), "expected '8. ': {p}");
        assert!(
            !p.contains("3. "),
            "sequential number should not appear: {p}"
        );
    }

    #[test]
    fn ordered_list_after_code_block() {
        let md =
            "```\n10. not a list\n20. still not a list\n```\n\n2. real\n4. also real\n8. third";
        let p = plain_text(&convert(md));
        assert!(p.contains("2. real"), "expected '2. real': {p}");
        assert!(p.contains("4. also real"), "expected '4. also real': {p}");
        assert!(p.contains("8. third"), "expected '8. third': {p}");
        assert!(
            !p.contains("1. real") && !p.contains("3. also"),
            "code-block numbers should not corrupt list: {p}"
        );
    }

    #[test]
    fn two_ordered_lists_same_depth() {
        let md = "2. a\n4. b\n\n3. x\n5. y";
        let p = plain_text(&convert(md));
        assert!(p.contains("2. "), "first list item 2: {p}");
        assert!(p.contains("4. "), "first list item 4: {p}");
        assert!(p.contains("3. "), "second list item 3: {p}");
        assert!(p.contains("5. "), "second list item 5: {p}");
    }

    #[test]
    fn ordered_list_in_blockquote() {
        let md = "> 2. alpha\n> 4. beta\n> 8. gamma";
        let p = plain_text(&convert(md));
        assert!(p.contains("2. alpha"), "item 2: {p}");
        assert!(p.contains("4. beta"), "item 4: {p}");
        assert!(p.contains("8. gamma"), "item 8: {p}");
        assert!(!p.contains("3. "), "should not renumber: {p}");
    }
}
