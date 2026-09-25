//! Wrap/truncation mapping: compute how a [`ratatui_core::text::Line`] is
//! laid out on screen by ratatui's `Paragraph` widget, and map screen columns
//! back to source (display-cell) columns within the original line.
//!
//! ratatui's reflow implementation (`ratatui_widgets::reflow`) is private, so
//! this module replicates its behaviour (greedy word wrap with `trim`,
//! mid-word hard splits, truncation with horizontal offset). The logic is a
//! faithful port of `WordWrapper::process_input` and `LineTruncator` from
//! ratatui-widgets 0.3, and is pinned by tests that compare against a real
//! `Paragraph` rendered into a `TestBackend` buffer.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use ratatui_core::layout::Alignment;
use ratatui_core::text::Line;

/// Zero-width space — treated as whitespace by ratatui.
const ZWSP: &str = "\u{200B}";
/// Non-breaking space — NOT treated as whitespace by ratatui.
const NBSP: &str = "\u{A0}";

/// One visible grapheme within a screen row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RowSeg<'a> {
    /// The grapheme text (used by tests to reconstruct the rendered layout).
    #[allow(dead_code)]
    pub sym: &'a str,
    /// Screen column within the row (before alignment offset).
    pub screen: u16,
    /// Display width in cells.
    pub width: u16,
    /// Source column (display cells) within the original [`Line`].
    pub src: u16,
}

/// How one screen row maps back to the original line.
#[derive(Debug, Clone)]
pub(crate) struct RowMap<'a> {
    /// Total visible width of the row (used for alignment offsets).
    pub width: u16,
    /// Visible graphemes, ordered left to right.
    pub segs: Vec<RowSeg<'a>>,
}

/// A grapheme of the source line with its display properties.
#[derive(Debug, Clone, Copy)]
struct Sym<'a> {
    sym: &'a str,
    width: u16,
    ws: bool,
    src: u16,
}

/// Mirror of ratatui's `StyledGrapheme::is_whitespace`.
fn is_whitespace(g: &str) -> bool {
    g == ZWSP || (g.chars().all(char::is_whitespace) && g != NBSP)
}

/// Segment a line into graphemes (per-span, exactly like ratatui's
/// `Line::styled_graphemes`) and compute display widths and source columns.
fn line_symbols<'l>(line: &'l Line) -> Vec<Sym<'l>> {
    let mut out = Vec::new();
    let mut col = 0u16;
    for span in &line.spans {
        for g in UnicodeSegmentation::graphemes(span.content.as_ref(), true) {
            let width = UnicodeWidthStr::width(g) as u16;
            out.push(Sym {
                sym: g,
                width,
                ws: is_whitespace(g),
                src: col,
            });
            col = col.saturating_add(width);
        }
    }
    out
}

/// Build a [`RowMap`] from one emitted (wrapped or truncated) row.
fn row_map(symbols: Vec<Sym<'_>>) -> RowMap<'_> {
    let mut segs = Vec::new();
    let mut width = 0u16;
    for s in symbols {
        // Zero-width graphemes are not rendered (ratatui skips them).
        if s.width == 0 {
            continue;
        }
        segs.push(RowSeg {
            sym: s.sym,
            screen: width,
            width: s.width,
            src: s.src,
        });
        width += s.width;
    }
    RowMap { width, segs }
}

// ── Word wrapping (port of `WordWrapper::process_input`) ─────────────────────

/// Split one source line into wrapped screen rows, replicating ratatui's
/// `WordWrapper` for the given `max_width` and `trim` flag.
///
/// Always returns at least one row (an empty line wraps to one empty row).
pub(crate) fn wrapped_rows<'l>(line: &'l Line, max_width: u16, trim: bool) -> Vec<RowMap<'l>> {
    process_input(line_symbols(line), max_width, trim)
        .into_iter()
        .map(row_map)
        .collect()
}

fn process_input<'a>(symbols: Vec<Sym<'a>>, max_width: u16, trim: bool) -> Vec<Vec<Sym<'a>>> {
    let mut wrapped: Vec<Vec<Sym<'a>>> = Vec::new();
    let mut pending_line: Vec<Sym<'a>> = Vec::new();
    let mut pending_word: Vec<Sym<'a>> = Vec::new();
    let mut pending_whitespace: std::collections::VecDeque<Sym<'a>> =
        std::collections::VecDeque::new();
    let mut line_width = 0u16;
    let mut word_width = 0u16;
    let mut whitespace_width = 0u16;
    let mut non_whitespace_previous = false;

    for grapheme in symbols {
        let is_ws = grapheme.ws;
        let symbol_width = grapheme.width;

        // Ignore symbols wider than the line limit.
        if symbol_width > max_width {
            continue;
        }

        let word_found = non_whitespace_previous && is_ws;
        // Current word would overflow after removing whitespace.
        let trimmed_overflow =
            pending_line.is_empty() && trim && word_width + symbol_width > max_width;
        // Separated whitespace would overflow on its own.
        let whitespace_overflow =
            pending_line.is_empty() && trim && whitespace_width + symbol_width > max_width;
        // Current full word (including whitespace) would overflow.
        let untrimmed_overflow = pending_line.is_empty()
            && !trim
            && word_width + whitespace_width + symbol_width > max_width;

        // Append finished segment to the current line.
        if word_found || trimmed_overflow || whitespace_overflow || untrimmed_overflow {
            if !pending_line.is_empty() || !trim {
                pending_line.extend(pending_whitespace.drain(..));
                line_width += whitespace_width;
            }
            pending_line.append(&mut pending_word);
            line_width += word_width;

            pending_whitespace.clear();
            whitespace_width = 0;
            word_width = 0;
        }

        // Pending line fills up the limit.
        let line_full = line_width >= max_width;
        // Pending word would overflow the line limit.
        let pending_word_overflow = symbol_width > 0
            && line_width + whitespace_width + word_width >= max_width;

        // Add the finished wrapped line to the remaining lines.
        if line_full || pending_word_overflow {
            let mut remaining_width = max_width.saturating_sub(line_width);
            wrapped.push(std::mem::take(&mut pending_line));
            line_width = 0;

            // Remove whitespace up to the end of the line.
            while let Some(g) = pending_whitespace.front() {
                let width = g.width;
                if width > remaining_width {
                    break;
                }
                whitespace_width -= width;
                remaining_width -= width;
                pending_whitespace.pop_front();
            }

            // Don't count the first whitespace toward the next word.
            if is_ws && pending_whitespace.is_empty() {
                continue;
            }
        }

        // Append the symbol to a pending buffer.
        if is_ws {
            whitespace_width += symbol_width;
            pending_whitespace.push_back(grapheme);
        } else {
            word_width += symbol_width;
            pending_word.push(grapheme);
        }

        non_whitespace_previous = !is_ws;
    }

    // Append remaining text parts.
    if pending_line.is_empty()
        && pending_word.is_empty()
        && !pending_whitespace.is_empty()
        && trim
    {
        wrapped.push(Vec::new());
    }
    if !pending_line.is_empty() || !trim {
        pending_line.extend(pending_whitespace.drain(..));
    }
    pending_line.append(&mut pending_word);

    if !pending_line.is_empty() {
        wrapped.push(pending_line);
    }
    if wrapped.is_empty() {
        wrapped.push(Vec::new());
    }
    wrapped
}

// ── Truncation (port of `LineTruncator`) ─────────────────────────────────────

/// Compute the single screen row for a line rendered without wrapping:
/// truncated at `max_width`, with horizontal scroll `h_offset` applied
/// (only for left-aligned lines, matching ratatui).
pub(crate) fn truncated_row<'a>(
    line: &'a Line,
    max_width: u16,
    h_offset: u16,
    alignment: Alignment,
) -> RowMap<'a> {
    let mut segs = Vec::new();
    let mut cur_w = 0u16;
    let mut h = h_offset;

    for sym in line_symbols(line) {
        // Ignore symbols wider than the total max width.
        if sym.width > max_width {
            continue;
        }
        if cur_w + sym.width > max_width {
            // Truncate the line.
            break;
        }
        if h == 0 || alignment != Alignment::Left {
            segs.push(RowSeg {
                sym: sym.sym,
                screen: cur_w,
                width: sym.width,
                src: sym.src,
            });
            cur_w += sym.width;
        } else if sym.width > h {
            // Partially-scrolled symbol: ratatui keeps it whole.
            h = 0;
            segs.push(RowSeg {
                sym: sym.sym,
                screen: cur_w,
                width: sym.width,
                src: sym.src,
            });
            cur_w += sym.width;
        } else {
            // Fully scrolled-out symbol.
            h -= sym.width;
        }
    }

    RowMap {
        width: cur_w,
        segs,
    }
}

// ── Hit mapping ───────────────────────────────────────────────────────────────

/// Horizontal offset of a rendered row, mirroring ratatui's `get_line_offset`.
pub(crate) fn line_offset(line_width: u16, area_width: u16, alignment: Alignment) -> u16 {
    match alignment {
        Alignment::Center => (area_width / 2).saturating_sub(line_width / 2),
        Alignment::Right => area_width.saturating_sub(line_width),
        Alignment::Left => 0,
    }
}

/// Map an x position within the paragraph area to the source column in the
/// original line, accounting for alignment. Returns `None` when the position
/// is over alignment padding or past the end of the row's content.
pub(crate) fn hit_src_col(
    row: &RowMap,
    x: u16,
    area_width: u16,
    alignment: Alignment,
) -> Option<u16> {
    let offset = line_offset(row.width, area_width, alignment);
    let xr = x.checked_sub(offset)?;
    if xr >= row.width {
        return None;
    }
    let seg = row
        .segs
        .iter()
        .find(|s| s.screen <= xr && xr < s.screen + s.width)?;
    Some(seg.src + (xr - seg.screen))
}
