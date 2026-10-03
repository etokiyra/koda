//! The editor pane: gutter, syntax, cursorline, indent guides and cursor.
//!
//! Koda's editor is intentionally bare — no box, no chrome. Code dominates.
//! A subtle cursorline (`ui.cursorline.primary`) and faint indent guides add
//! warmth without fighting the code.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::overlay::Search;
use crate::editor::{Document, Position};
use crate::language::diagnostics::{Diagnostic, Severity};
use crate::language::provider::{LanguageProvider, TokenKind};
use crate::ui::{art, theme};

/// Render the active document, returning the screen position of the cursor when
/// it is visible (used to anchor the completion popup).
pub fn render(
    frame: &mut Frame,
    area: Rect,
    doc: &mut Document,
    provider: &dyn LanguageProvider,
    search: &Search,
    focused: bool,
    inline_diagnostics: bool,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    if doc.buffer.len_chars() == 0 {
        render_empty(frame, area, focused);
        return if focused {
            Some((area.x, area.y))
        } else {
            None
        };
    }

    let total_lines = doc.buffer.len_lines();
    // One column for the diagnostic marker, the widest line number, and a gap.
    let number_width = total_lines.max(1).to_string().len();
    let gutter = (number_width + 2) as u16;
    let text_width = area.width.saturating_sub(gutter) as usize;
    if text_width == 0 {
        return None;
    }
    let view_height = area.height as usize;
    // Keep a little context visible around the cursor so it never sits glued to
    // an edge (scrolloff, as in Helix).
    const SCROLL_OFF: usize = 3;

    // Keep the cursor inside the viewport, with a margin.
    let cursor = doc.clamped_cursor();
    let max_top = total_lines.saturating_sub(view_height);
    if cursor.row < doc.scroll_top + SCROLL_OFF {
        doc.scroll_top = cursor.row.saturating_sub(SCROLL_OFF);
    }
    if cursor.row + SCROLL_OFF >= doc.scroll_top + view_height {
        doc.scroll_top = (cursor.row + SCROLL_OFF + 1).saturating_sub(view_height);
    }
    doc.scroll_top = doc.scroll_top.min(max_top);

    // Horizontal scroll is driven by the cursor line, with the same margin.
    let indent = doc.indent_width().max(1);
    let cursor_line = doc.buffer.line_text(cursor.row);
    let cursor_layout = LineLayout::new(&cursor_line, indent);
    let cursor_col = cursor_layout.display_col(cursor.col);
    if cursor_col < doc.scroll_left + SCROLL_OFF {
        doc.scroll_left = cursor_col.saturating_sub(SCROLL_OFF);
    }
    if cursor_col + SCROLL_OFF >= doc.scroll_left + text_width {
        doc.scroll_left = (cursor_col + SCROLL_OFF + 1).saturating_sub(text_width);
    }
    doc.scroll_left = doc
        .scroll_left
        .min(cursor_layout.len.saturating_sub(text_width));

    let mut lines: Vec<Line> = Vec::with_capacity(view_height);
    let brackets = doc.matching_brackets(provider);
    for row in doc.scroll_top..(doc.scroll_top + view_height).min(total_lines) {
        lines.push(render_line(
            doc,
            provider,
            search,
            row,
            gutter,
            text_width,
            area.width,
            brackets,
            inline_diagnostics,
        ));
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    // Place the terminal cursor and report where it landed.
    let mut cursor_screen = None;
    if focused && cursor.row >= doc.scroll_top && cursor.row < doc.scroll_top + view_height {
        let x = area.x + gutter + (cursor_col.saturating_sub(doc.scroll_left)) as u16;
        let y = area.y + (cursor.row - doc.scroll_top) as u16;
        if x < area.x + area.width && y < area.y + area.height {
            frame.set_cursor_position((x, y));
            cursor_screen = Some((x, y));
        }
    }
    cursor_screen
}

fn render_empty(frame: &mut Frame, area: Rect, focused: bool) {
    let mut lines: Vec<Line> = vec![Line::from("")];
    lines.extend(art::art_lines(art::CAT, theme::dim()));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("an empty file · start typing", theme::muted())).centered());

    let block_height = lines.len() as u16;
    if area.height > block_height {
        let top = (area.height - block_height) / 2;
        let mut padded: Vec<Line> = (0..top).map(|_| Line::from("")).collect();
        padded.extend(lines);
        lines = padded;
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    if focused {
        frame.set_cursor_position((area.x, area.y));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_line(
    doc: &mut Document,
    provider: &dyn LanguageProvider,
    search: &Search,
    row: usize,
    gutter: u16,
    text_width: usize,
    line_width: u16,
    brackets: Option<(Position, Position)>,
    inline_diagnostics: bool,
) -> Line<'static> {
    let text = doc.buffer.line_text(row);
    let char_count = text.chars().count();
    let current = doc.cursor.row == row;
    let base_bg = if current {
        Some(theme::CURSORLINE_BG)
    } else {
        None
    };

    // Token kinds per original character.
    let mut kinds = vec![TokenKind::Plain; char_count];
    for span in doc.highlight_spans(provider, row) {
        for index in span.range {
            if let Some(kind) = kinds.get_mut(index) {
                *kind = span.kind;
            }
        }
    }

    // Diagnostics overlay per original character (rendered as an underline).
    let mut diagnostic_severity: Vec<Option<Severity>> = vec![None; char_count];
    for diagnostic in doc.diagnostics() {
        if row < diagnostic.start.line || row > diagnostic.end.line {
            continue;
        }
        let from = if row == diagnostic.start.line {
            diagnostic.start.col
        } else {
            0
        };
        let to = if row == diagnostic.end.line {
            diagnostic.end.col
        } else {
            char_count
        };
        for slot in diagnostic_severity
            .iter_mut()
            .take(to.min(char_count))
            .skip(from)
        {
            *slot = Some(diagnostic.severity);
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
    let mut current_match = vec![false; char_count];
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
                current_match[i] = true;
            }
        }
    }

    let indent = doc.indent_width().max(1);
    let layout = LineLayout::new(&text, indent);
    let leading_ws = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let leading_display = layout.display_col(leading_ws.min(char_count));

    let start = doc.scroll_left.min(layout.len);
    let end = (start + text_width).min(layout.len);

    let marker_severity = doc.diagnostic_severity_on_line(row);
    let gutter_style = if current {
        theme::accent_bold()
    } else {
        theme::dim()
    };
    let marker_style = match marker_severity {
        Some(severity) => with_bg(severity_style(severity), base_bg),
        None => with_bg(gutter_style, base_bg),
    };
    let marker = marker_severity.map(Severity::gutter).unwrap_or(' ');
    let mut spans: Vec<Span> = vec![Span::styled(marker.to_string(), marker_style)];
    spans.push(Span::styled(
        format!(
            "{:>width$} ",
            row + 1,
            width = gutter.saturating_sub(2) as usize
        ),
        with_bg(gutter_style, base_bg),
    ));

    // Group consecutive cells that share a style.
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    for cell in start..end {
        let (mut ch, original) = layout.cells[cell];
        let mut style =
            theme::token_style(kinds.get(original).copied().unwrap_or(TokenKind::Plain));

        // Indent guides inside leading whitespace, one per detected level.
        if cell < leading_display && cell > 0 && cell % indent == 0 {
            ch = '│';
            style = theme::dim();
        }
        style = with_bg(style, base_bg);
        if diagnostic_severity
            .get(original)
            .is_some_and(|slot| slot.is_some())
        {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        if let Some((open, close)) = brackets
            && ((open.row == row && open.col == original)
                || (close.row == row && close.col == original))
        {
            // Mellow `ui.cursor.match`: yellow, bold and underlined.
            style = with_bg(theme::bracket_match(), base_bg);
        }
        if selected.get(original).copied().unwrap_or(false) {
            style = style.bg(theme::SELECTION_BG);
        }
        if matched.get(original).copied().unwrap_or(false) {
            style = style.bg(if current_match.get(original).copied().unwrap_or(false) {
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

    // An inline note for the most severe diagnostic on this line. Shown only
    // when the line is not horizontally scrolled and there is room for it, so
    // it never pushes code off the edge.
    let mut note_width = 0usize;
    if inline_diagnostics && doc.scroll_left == 0 {
        let on_line: Vec<&Diagnostic> = doc
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.start.line <= row && diagnostic.end.line >= row)
            .collect();
        if let Some(diagnostic) = on_line.iter().copied().max_by_key(|d| d.severity) {
            let used = gutter as usize + (end - start);
            let available = (line_width as usize).saturating_sub(used);
            if let Some((note, severity)) = inline_note(diagnostic, on_line.len(), available) {
                note_width = note.chars().count();
                spans.push(Span::styled(
                    note,
                    with_bg(
                        severity_style(severity).add_modifier(Modifier::ITALIC),
                        base_bg,
                    ),
                ));
            }
        }
    }

    // Paint the cursorline band across the full width.
    if let Some(bg) = base_bg {
        let used = gutter as usize + (end - start) + note_width;
        let pad = (line_width as usize).saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
        }
    }

    Line::from(spans)
}

/// Format an inline diagnostic note that fits within `available` columns.
///
/// Returns the text to render and the severity to colour it with, or `None`
/// when the line is too narrow to say anything useful.
fn inline_note(
    diagnostic: &Diagnostic,
    count: usize,
    available: usize,
) -> Option<(String, Severity)> {
    const PREFIX: &str = "  ·  ";
    let extra = if count > 1 {
        format!(" (+{})", count - 1)
    } else {
        String::new()
    };
    let fixed = PREFIX.chars().count() + extra.chars().count();
    if available <= fixed + 4 {
        return None;
    }
    let budget = available - fixed;
    let message = diagnostic.message.replace('\n', " ");
    let message = truncate(&message, budget);
    Some((format!("{PREFIX}{message}{extra}"), diagnostic.severity))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn with_bg(style: Style, bg: Option<Color>) -> Style {
    match bg {
        Some(color) => style.bg(color),
        None => style,
    }
}

/// Foreground style for a diagnostic severity.
fn severity_style(severity: Severity) -> Style {
    match severity {
        Severity::Error => theme::error(),
        Severity::Warning => theme::warn(),
        Severity::Info => theme::info(),
        Severity::Hint => theme::hint(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::diagnostics::TextPos;

    fn diagnostic(message: &str, severity: Severity) -> Diagnostic {
        Diagnostic::new(TextPos::new(0, 0), TextPos::new(0, 1), severity, message)
    }

    #[test]
    fn inline_note_fits_or_declines() {
        let diagnostic = diagnostic("unused variable `x`", Severity::Warning);
        let (note, severity) = inline_note(&diagnostic, 1, 40).expect("note");
        assert!(note.contains("unused variable"));
        assert_eq!(severity, Severity::Warning);
        // A line too narrow to say anything useful produces no note.
        assert!(inline_note(&diagnostic, 1, 8).is_none());
    }

    #[test]
    fn inline_note_reports_additional_diagnostics() {
        let diagnostic = diagnostic("boom", Severity::Error);
        let (note, _) = inline_note(&diagnostic, 3, 40).unwrap();
        assert!(note.ends_with("(+2)"), "note was {note}");
    }

    #[test]
    fn inline_note_truncates_and_stays_within_budget() {
        let diagnostic = diagnostic(&"x".repeat(100), Severity::Error);
        let (note, _) = inline_note(&diagnostic, 1, 30).unwrap();
        assert!(note.chars().count() <= 30);
        assert!(note.contains('…'));
    }
}
