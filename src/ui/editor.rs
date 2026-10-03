//! The editor pane: gutter, syntax, cursorline, indent guides and cursor.
//!
//! Koda's editor is intentionally bare — no box, no chrome. Code dominates.
//! A subtle cursorline (`ui.cursorline.primary`) and faint indent guides add
//! warmth without fighting the code.
//!
//! When soft wrap is on, one logical line may occupy several visual rows. The
//! character ↔ display-column mapping and the wrap boundaries come from
//! [`crate::editor::layout`], the same code the cursor movement uses, so the
//! renderer and the editor never disagree about where a line breaks.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::overlay::Search;
use crate::editor::layout::{LineLayout, segment_index, wrap_segments};
use crate::editor::{Document, Position};
use crate::language::diagnostics::{Diagnostic, Severity};
use crate::language::provider::{LanguageProvider, TokenKind};
use crate::ui::{art, theme};

/// Keep a little context visible around the cursor so it never sits glued to an
/// edge (scrolloff, as in Helix).
const SCROLL_OFF: usize = 3;

/// Render the active document, returning the screen position of the cursor when
/// it is visible (used to anchor the completion popup).
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    doc: &mut Document,
    provider: &dyn LanguageProvider,
    search: &Search,
    focused: bool,
    inline_diagnostics: bool,
    wrap: bool,
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

    doc.wrap_width = if wrap { text_width } else { 0 };
    scroll_viewport(doc, text_width, view_height, SCROLL_OFF);

    let cursor = doc.clamped_cursor();
    let cursor_display = doc.display_col_of(cursor.row, cursor.col);
    let brackets = doc.matching_brackets(provider);
    // Secondary cursors: their selections share the selection background, and
    // each caret gets a solid accent block (the terminal can only place one
    // real cursor, so extra carets are drawn as cells).
    let secondary_selections: Vec<(Position, Position)> = doc
        .cursors
        .iter()
        .map(|cursor| cursor.range())
        .filter(|(start, end)| start != end)
        .collect();
    let secondary_carets: std::collections::HashSet<Position> =
        doc.cursors.iter().map(|cursor| cursor.cursor).collect();

    let mut lines: Vec<Line> = Vec::with_capacity(view_height);
    let mut cursor_screen = None;

    if wrap {
        let mut row = doc.scroll_top;
        let mut subline = doc.scroll_subline;
        while lines.len() < view_height && row < total_lines {
            let segments = doc.row_segments(row, text_width);
            if subline >= segments.len() {
                subline = 0;
                row += 1;
                continue;
            }
            let context = build_row(
                doc,
                provider,
                search,
                row,
                &secondary_selections,
                &secondary_carets,
            );
            let cursor_segment =
                (focused && row == cursor.row).then(|| segment_index(&segments, cursor_display));
            for (index, &(start, end)) in segments.iter().enumerate().skip(subline) {
                if lines.len() >= view_height {
                    break;
                }
                if cursor_segment == Some(index) {
                    let x = area.x + gutter + (cursor_display.saturating_sub(start)) as u16;
                    let y = area.y + lines.len() as u16;
                    if x < area.x + area.width {
                        cursor_screen = Some((x, y));
                    }
                }
                let first = index == 0;
                let last = index + 1 == segments.len();
                lines.push(render_segment(
                    &context,
                    row,
                    start,
                    end,
                    first,
                    last,
                    gutter,
                    area.width,
                    brackets,
                    inline_diagnostics,
                    true,
                ));
            }
            row += 1;
            subline = 0;
        }
    } else {
        for row in doc.scroll_top..(doc.scroll_top + view_height).min(total_lines) {
            let context = build_row(
                doc,
                provider,
                search,
                row,
                &secondary_selections,
                &secondary_carets,
            );
            let start = doc.scroll_left.min(context.layout.len());
            let end = (start + text_width).min(context.layout.len());
            if focused && row == cursor.row {
                let x = area.x + gutter + (cursor_display.saturating_sub(doc.scroll_left)) as u16;
                let y = area.y + (row - doc.scroll_top) as u16;
                if x < area.x + area.width && y < area.y + area.height {
                    cursor_screen = Some((x, y));
                }
            }
            lines.push(render_segment(
                &context,
                row,
                start,
                end,
                true,
                true,
                gutter,
                area.width,
                brackets,
                inline_diagnostics,
                false,
            ));
        }
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    if let Some((x, y)) = cursor_screen {
        frame.set_cursor_position((x, y));
    }
    cursor_screen
}

/// Scroll the viewport so the cursor is visible, in visual rows when wrapping
/// and in logical rows otherwise.
///
/// `doc.wrap_width` must already be set. With wrapping the top is kept at or
/// before the cursor's logical line, which bounds the work to the visible rows
/// instead of scanning the whole document.
fn scroll_viewport(doc: &mut Document, width: usize, height: usize, scroll_off: usize) {
    let width = width.max(1);
    let height = height.max(1);
    let cursor = doc.clamped_cursor();

    if doc.wrap_width == 0 {
        doc.scroll_subline = 0;
        let total = doc.buffer.len_lines();
        if cursor.row < doc.scroll_top + scroll_off {
            doc.scroll_top = cursor.row.saturating_sub(scroll_off);
        }
        if cursor.row + scroll_off >= doc.scroll_top + height {
            doc.scroll_top = (cursor.row + scroll_off + 1).saturating_sub(height);
        }
        let max_top = total.saturating_sub(height);
        doc.scroll_top = doc.scroll_top.min(max_top);

        // Horizontal scroll is driven by the cursor line, with the same margin.
        let layout = doc.line_layout(cursor.row);
        let cursor_col = layout.display_col(cursor.col);
        if cursor_col < doc.scroll_left + scroll_off {
            doc.scroll_left = cursor_col.saturating_sub(scroll_off);
        }
        if cursor_col + scroll_off >= doc.scroll_left + width {
            doc.scroll_left = (cursor_col + scroll_off + 1).saturating_sub(width);
        }
        doc.scroll_left = doc.scroll_left.min(layout.len().saturating_sub(width));
        return;
    }

    // Wrapping: no horizontal scrolling.
    doc.scroll_left = 0;
    if cursor.row < doc.scroll_top {
        doc.scroll_top = cursor.row;
        doc.scroll_subline = 0;
    }
    // If the cursor is far below, recenter on its line before measuring, so the
    // visual-offset walk stays bounded by the viewport height.
    if cursor.row > doc.scroll_top.saturating_add(height) {
        doc.scroll_top = cursor.row;
        doc.scroll_subline = 0;
    }

    let cursor_segment = segment_of(doc, cursor.row, cursor.col, width);
    // A subline beyond the cursor's segment would make the offset negative.
    if cursor.row == doc.scroll_top && cursor_segment < doc.scroll_subline {
        doc.scroll_subline = cursor_segment;
    }

    let offset = visual_offset(
        doc,
        width,
        doc.scroll_top,
        doc.scroll_subline,
        cursor.row,
        cursor_segment,
    );
    if offset < scroll_off {
        let (row, subline) = walk_back(doc, width, cursor.row, cursor_segment, scroll_off);
        doc.scroll_top = row;
        doc.scroll_subline = subline;
    } else if offset + 1 + scroll_off > height {
        let above = height.saturating_sub(scroll_off + 1);
        let (row, subline) = walk_back(doc, width, cursor.row, cursor_segment, above);
        doc.scroll_top = row;
        doc.scroll_subline = subline;
    }

    // Never point the subline past the row's last visual segment.
    let count = doc.row_visual_count(doc.scroll_top, width).max(1);
    if doc.scroll_subline >= count {
        doc.scroll_subline = count - 1;
    }
    doc.scroll_top = doc.scroll_top.min(doc.buffer.len_lines().saturating_sub(1));
}

/// The visual row of `(row, col)` at `width`.
fn segment_of(doc: &Document, row: usize, col: usize, width: usize) -> usize {
    let layout = doc.line_layout(row);
    let display = layout.display_col(col);
    let segments = wrap_segments(&layout, width);
    segment_index(&segments, display)
}

/// How many visual rows separate `(top_row, top_subline)` from `(row, seg)`.
fn visual_offset(
    doc: &Document,
    width: usize,
    top_row: usize,
    top_subline: usize,
    row: usize,
    seg: usize,
) -> usize {
    let mut offset = 0usize;
    for r in top_row..row {
        let count = doc.row_visual_count(r, width);
        offset += count - if r == top_row { top_subline } else { 0 };
    }
    offset + seg.saturating_sub(if row == top_row { top_subline } else { 0 })
}

/// The `(row, subline)` `n` visual rows above `(row, seg)`.
fn walk_back(doc: &Document, width: usize, row: usize, seg: usize, n: usize) -> (usize, usize) {
    let mut r = row;
    let mut s = seg;
    for _ in 0..n {
        if s > 0 {
            s -= 1;
        } else if r > 0 {
            r -= 1;
            s = doc.row_visual_count(r, width).saturating_sub(1);
        } else {
            break;
        }
    }
    (r, s)
}

/// Per-character overlays for one logical line, computed once and reused for
/// every visual row the line occupies.
struct RowContext {
    layout: LineLayout,
    kinds: Vec<TokenKind>,
    diagnostic_severity: Vec<Option<Severity>>,
    selected: Vec<bool>,
    matched: Vec<bool>,
    current_match: Vec<bool>,
    secondary_caret: Vec<bool>,
    trailing_caret: bool,
    leading_display: usize,
    indent: usize,
    current: bool,
    marker_severity: Option<Severity>,
    /// The most severe diagnostic touching this line, with how many touch it,
    /// for the inline note.
    note: Option<(Diagnostic, usize)>,
}

fn build_row(
    doc: &mut Document,
    provider: &dyn LanguageProvider,
    search: &Search,
    row: usize,
    secondary_selections: &[(Position, Position)],
    secondary_carets: &std::collections::HashSet<Position>,
) -> RowContext {
    let text = doc.buffer.line_text(row);
    let char_count = text.chars().count();
    let current = doc.cursor.row == row;

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
    for (start, end) in secondary_selections {
        if row < start.row || row > end.row {
            continue;
        }
        let from = if row == start.row { start.col } else { 0 };
        let to = if row == end.row { end.col } else { char_count };
        for flag in selected.iter_mut().take(to.min(char_count)).skip(from) {
            *flag = true;
        }
    }
    let mut secondary_caret = vec![false; char_count];
    for caret in secondary_carets {
        if caret.row == row && caret.col < char_count {
            secondary_caret[caret.col] = true;
        }
    }
    let trailing_caret = secondary_carets.contains(&Position::new(row, char_count));
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
    let layout = doc.line_layout(row);
    let leading_ws = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let leading_display = layout.display_col(leading_ws.min(char_count));

    // The most severe diagnostic touching this line, plus the count on the line.
    let on_line: Vec<Diagnostic> = doc
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.start.line <= row && diagnostic.end.line >= row)
        .cloned()
        .collect();
    let count = on_line.len();
    let note = on_line
        .into_iter()
        .max_by_key(|diagnostic| diagnostic.severity)
        .map(|diagnostic| (diagnostic, count));

    RowContext {
        layout,
        kinds,
        diagnostic_severity,
        selected,
        matched,
        current_match,
        secondary_caret,
        trailing_caret,
        leading_display,
        indent,
        current,
        marker_severity: doc.diagnostic_severity_on_line(row),
        note,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_segment(
    context: &RowContext,
    row: usize,
    display_start: usize,
    display_end: usize,
    first: bool,
    last: bool,
    gutter: u16,
    line_width: u16,
    brackets: Option<(Position, Position)>,
    inline_diagnostics: bool,
    wrapped: bool,
) -> Line<'static> {
    let base_bg = if context.current {
        Some(theme::CURSORLINE_BG)
    } else {
        None
    };
    let gutter_style = if context.current {
        theme::accent_bold()
    } else {
        theme::dim()
    };
    let mut spans: Vec<Span> = Vec::new();

    if first {
        let marker_style = match context.marker_severity {
            Some(severity) => with_bg(severity_style(severity), base_bg),
            None => with_bg(gutter_style, base_bg),
        };
        let marker = context.marker_severity.map(Severity::gutter).unwrap_or(' ');
        spans.push(Span::styled(marker.to_string(), marker_style));
        spans.push(Span::styled(
            format!(
                "{:>width$} ",
                row + 1,
                width = gutter.saturating_sub(2) as usize
            ),
            with_bg(gutter_style, base_bg),
        ));
    } else {
        // Continuation rows keep the gutter blank so the number reads as one row.
        spans.push(Span::styled(
            " ".repeat(gutter as usize),
            with_bg(gutter_style, base_bg),
        ));
    }

    // Group consecutive cells that share a style.
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    for cell in display_start..display_end {
        let Some((mut ch, original)) = context.layout.cell(cell) else {
            break;
        };
        let mut style = theme::token_style(
            context
                .kinds
                .get(original)
                .copied()
                .unwrap_or(TokenKind::Plain),
        );

        // Indent guides inside leading whitespace, one per detected level.
        if cell < context.leading_display && cell > 0 && cell % context.indent == 0 {
            ch = '│';
            style = theme::dim();
        }
        style = with_bg(style, base_bg);
        if context
            .diagnostic_severity
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
        if context.selected.get(original).copied().unwrap_or(false) {
            style = style.bg(theme::SELECTION_BG);
        }
        if context.matched.get(original).copied().unwrap_or(false) {
            style = style.bg(
                if context
                    .current_match
                    .get(original)
                    .copied()
                    .unwrap_or(false)
                {
                    theme::SEARCH_CURRENT_BG
                } else {
                    theme::SEARCH_BG
                },
            );
        }
        // A secondary caret is a solid accent block over whatever is beneath it.
        if context
            .secondary_caret
            .get(original)
            .copied()
            .unwrap_or(false)
        {
            style = style.bg(theme::MULTI_CURSOR).fg(theme::palette::BG_DARK);
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

    // A secondary caret sitting past the last character is drawn as a block on
    // the final visual row.
    if last && context.trailing_caret {
        spans.push(Span::styled(
            " ",
            Style::default()
                .bg(theme::MULTI_CURSOR)
                .fg(theme::palette::BG_DARK),
        ));
    }

    // An inline note for the most severe diagnostic on this line, on its last
    // visual row. Shown only when there is room, so it never pushes code off.
    let mut note_width = 0usize;
    let note_allowed = inline_diagnostics && last && (wrapped || display_start == 0);
    if note_allowed && let Some((diagnostic, count)) = &context.note {
        let used = gutter as usize + (display_end - display_start);
        let available = (line_width as usize).saturating_sub(used);
        if let Some((note, severity)) = inline_note(diagnostic, *count, available) {
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

    // Paint the cursorline band across the full width.
    if let Some(bg) = base_bg {
        let used = gutter as usize + (display_end - display_start) + note_width;
        let pad = (line_width as usize).saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
        }
    }

    Line::from(spans)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::buffer::Buffer;
    use crate::language::diagnostics::TextPos;

    fn diagnostic(message: &str, severity: Severity) -> Diagnostic {
        Diagnostic::new(TextPos::new(0, 0), TextPos::new(0, 1), severity, message)
    }

    fn doc(text: &str) -> Document {
        Document::new(Buffer::from_text(text, None))
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

    #[test]
    fn wrapped_scroll_keeps_the_cursor_inside_the_viewport() {
        let mut d = doc(&"x".repeat(400));
        d.wrap_width = 20;
        // Put the cursor at the far end of the very long line.
        d.move_to(Position::new(0, 400));
        scroll_viewport(&mut d, 20, 10, SCROLL_OFF);

        let cursor = d.clamped_cursor();
        let seg = segment_of(&d, cursor.row, cursor.col, 20);
        let offset = visual_offset(&d, 20, d.scroll_top, d.scroll_subline, cursor.row, seg);
        assert!(
            offset < 10,
            "cursor visual row {offset} should be inside a 10-row viewport"
        );
        // With scrolloff at the bottom, there is context below the cursor.
        assert!(d.scroll_top <= cursor.row);
    }

    #[test]
    fn wrapped_scroll_backs_up_at_the_top() {
        // The cursor on line 1 with wrapping should leave scrolloff above it.
        let mut d = doc("first wrapped line that is quite long\nsecond\nthird\nfourth\nfifth\n");
        d.wrap_width = 10;
        d.move_to(Position::new(1, 0));
        scroll_viewport(&mut d, 10, 8, SCROLL_OFF);
        let cursor = d.clamped_cursor();
        let seg = segment_of(&d, cursor.row, cursor.col, 10);
        let offset = visual_offset(&d, 10, d.scroll_top, d.scroll_subline, cursor.row, seg);
        assert!(offset < 8);
    }

    #[test]
    fn unwrapped_scroll_matches_logical_lines() {
        let mut d = doc(&(0..100).map(|i| format!("line {i}\n")).collect::<String>());
        d.wrap_width = 0;
        d.move_to(Position::new(90, 0));
        scroll_viewport(&mut d, 40, 10, SCROLL_OFF);
        assert!(d.scroll_top <= 90 && 90 < d.scroll_top + 10);
        assert_eq!(d.scroll_subline, 0);
    }
}
