//! The editor pane: gutters, syntax highlighting, selection and the cursor.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::app::overlay::Search;
use crate::editor::Document;
use crate::language::provider::{LanguageProvider, TokenKind};
use crate::ui::theme;

const TAB_WIDTH: usize = 4;

/// Render the active document.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    doc: &mut Document,
    provider: &dyn LanguageProvider,
    search: &Search,
    focused: bool,
) {
    let title = if doc.is_dirty() {
        format!(" {} ● ", doc.file_name())
    } else {
        format!(" {} ", doc.file_name())
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border(focused))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let total_lines = doc.buffer.len_lines();
    let gutter = (total_lines.max(1).to_string().len() + 1) as u16;
    let text_width = inner.width.saturating_sub(gutter) as usize;
    if text_width == 0 {
        return;
    }
    let view_height = inner.height as usize;

    // Keep the cursor inside the viewport.
    let cursor = doc.clamped_cursor();
    if cursor.row < doc.scroll_top {
        doc.scroll_top = cursor.row;
    }
    if cursor.row >= doc.scroll_top + view_height {
        doc.scroll_top = cursor.row + 1 - view_height;
    }

    // Horizontal scroll is driven by the cursor line.
    let cursor_line = doc.buffer.line_text(cursor.row);
    let cursor_layout = LineLayout::new(&cursor_line, TAB_WIDTH);
    let cursor_col = cursor_layout.display_col(cursor.col);
    if cursor_col < doc.scroll_left {
        doc.scroll_left = cursor_col;
    }
    if cursor_col >= doc.scroll_left + text_width {
        doc.scroll_left = cursor_col + 1 - text_width;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(view_height);
    for row in doc.scroll_top..(doc.scroll_top + view_height).min(total_lines) {
        lines.push(render_line(doc, provider, search, row, gutter, text_width));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);

    // Place the terminal cursor.
    if focused && cursor.row >= doc.scroll_top && cursor.row < doc.scroll_top + view_height {
        let x = inner.x + gutter + (cursor_col.saturating_sub(doc.scroll_left)) as u16;
        let y = inner.y + (cursor.row - doc.scroll_top) as u16;
        if x < inner.x + inner.width && y < inner.y + inner.height {
            frame.set_cursor_position((x, y));
        }
    }
}

fn render_line(
    doc: &mut Document,
    provider: &dyn LanguageProvider,
    search: &Search,
    row: usize,
    gutter: u16,
    text_width: usize,
) -> Line<'static> {
    let text = doc.buffer.line_text(row);
    let char_count = text.chars().count();

    // Token kinds per original character.
    let mut kinds = vec![TokenKind::Plain; char_count];
    for span in doc.highlight_spans(provider, row) {
        for index in span.range {
            if let Some(kind) = kinds.get_mut(index) {
                *kind = span.kind;
            }
        }
    }

    // Selection and search overlays per original character.
    let mut selected = vec![false; char_count];
    if let Some((from, to)) = selection_columns(doc, row) {
        for flag in selected.iter_mut().take(to.min(char_count)).skip(from) {
            *flag = true;
        }
    }
    let mut matched = vec![false; char_count];
    let mut current = vec![false; char_count];
    for (index, (start, end)) in search.matches.iter().enumerate() {
        if row < start.row || row > end.row {
            continue;
        }
        let from = if row == start.row { start.col } else { 0 };
        let to = if row == end.row { end.col } else { char_count };
        let is_current = search.current == Some(index);
        for i in from..to.min(char_count) {
            matched[i] = true;
            if is_current {
                current[i] = true;
            }
        }
    }

    let layout = LineLayout::new(&text, TAB_WIDTH);
    let start = doc.scroll_left.min(layout.len);
    let end = (start + text_width).min(layout.len);

    let mut spans: Vec<Span> = Vec::new();
    let gutter_style = if doc.cursor.row == row {
        theme::accent()
    } else {
        theme::dim()
    };
    spans.push(Span::styled(
        format!("{:>width$} ", row + 1, width = gutter as usize - 1),
        gutter_style,
    ));

    // Group consecutive cells that share a style.
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    for cell in start..end {
        let (ch, original) = layout.cells[cell];
        let mut style =
            theme::token_style(kinds.get(original).copied().unwrap_or(TokenKind::Plain));
        if selected.get(original).copied().unwrap_or(false) {
            style = style.bg(theme::SELECTION_BG);
        }
        if matched.get(original).copied().unwrap_or(false) {
            style = style.bg(if current.get(original).copied().unwrap_or(false) {
                theme::SEARCH_CURRENT_BG
            } else {
                theme::SEARCH_BG
            });
        }
        if run_style == Some(style) {
            run.push(ch);
        } else {
            if let Some(previous) = run_style.take() {
                spans.push(Span::styled(std::mem::take(&mut run), previous));
            }
            run.push(ch);
            run_style = Some(style);
        }
    }
    if let Some(style) = run_style {
        spans.push(Span::styled(run, style));
    }

    Line::from(spans)
}

/// The selected column range on a row, if the selection covers it.
fn selection_columns(doc: &Document, row: usize) -> Option<(usize, usize)> {
    let (start, end) = doc.selection_range()?;
    if row < start.row || row > end.row {
        return None;
    }
    let from = if row == start.row { start.col } else { 0 };
    let to = if row == end.row {
        end.col
    } else {
        doc.buffer.line_char_len(row)
    };
    Some((from, to))
}

/// A line expanded for display, with a mapping back to original character indices.
struct LineLayout {
    cells: Vec<(char, usize)>,
    /// `map[original_index]` is the display column where that character begins.
    map: Vec<usize>,
    len: usize,
}

impl LineLayout {
    fn new(text: &str, tab_width: usize) -> Self {
        let mut cells = Vec::with_capacity(text.len());
        let mut map = Vec::with_capacity(text.chars().count() + 1);
        let mut column = 0usize;

        for (index, ch) in text.chars().enumerate() {
            map.push(column);
            if ch == '\t' {
                let spaces = tab_width - (column % tab_width);
                for _ in 0..spaces {
                    cells.push((' ', index));
                    column += 1;
                }
            } else {
                cells.push((ch, index));
                column += 1;
            }
        }
        map.push(column);
        LineLayout {
            cells,
            map,
            len: column,
        }
    }

    fn display_col(&self, original: usize) -> usize {
        self.map.get(original).copied().unwrap_or(self.len)
    }
}
