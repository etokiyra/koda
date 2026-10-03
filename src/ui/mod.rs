//! Rendering for every Koda surface.

pub mod art;
pub mod editor;
pub mod file_tree;
pub mod header;
pub mod overlay;
pub mod status_bar;
pub mod tabs;
pub mod theme;
pub mod welcome;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::overlay::{Overlay, Search};
use crate::app::{App, Focus, Pane};
use crate::editor::Document;
use crate::language::LanguageService;

/// Draw the whole application.
pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let search_height = if app.search.open {
        if app.search.replace_mode { 2 } else { 1 }
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(1),    // body
            Constraint::Length(search_height),
            Constraint::Length(1), // status bar
        ])
        .split(area);

    header::render(frame, chunks[0], app);
    render_body(frame, chunks[1], app);
    if app.search.open {
        overlay::render_search(frame, chunks[2], &app.search);
    }
    status_bar::render(frame, chunks[3], app);
    // Notifications sit just above the statusline; overlays draw over them.
    overlay::render_toasts(frame, area, &app.toasts);

    match &app.overlay {
        Overlay::None => {}
        Overlay::Picker(picker) => overlay::render_picker(frame, area, picker),
        Overlay::Prompt(prompt) => overlay::render_prompt(frame, area, prompt),
        Overlay::Help(help) => {
            overlay::render_help(frame, area, help, &app.commands, app.anim_phase)
        }
        Overlay::DirPicker(picker) => overlay::render_dir_picker(frame, area, picker),
        Overlay::NewProject(flow) => overlay::render_new_project(frame, area, flow),
        Overlay::Diff(diff) => overlay::render_diff(frame, area, diff),
    }

    if app.overlay.is_none() {
        if let Some(hover) = &app.hover {
            overlay::render_hover(frame, area, hover, app.cursor_screen);
        } else if let Some(completion) = &app.completion {
            overlay::render_completion(frame, area, completion, app.cursor_screen);
        }
    }
}

fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.tree_visible {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(1)])
            .split(area);
        if let Some(filter) = &app.tree_filter {
            file_tree::render_filter(frame, chunks[0], filter, app.focus == Focus::FileTree);
        } else {
            file_tree::render(
                frame,
                chunks[0],
                &app.workspace.tree,
                &app.workspace.git,
                app.focus == Focus::FileTree,
            );
        }
        render_editor_area(frame, chunks[1], app);
    } else {
        render_editor_area(frame, area, app);
    }
}

fn render_editor_area(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.editor.is_empty() {
        welcome::render(frame, area, app);
        app.viewport_height = area.height.saturating_sub(2) as usize;
        app.cursor_screen = None;
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    tabs::render(frame, chunks[0], &app.editor);
    let body = chunks[1];
    app.viewport_height = body.height.saturating_sub(2) as usize;
    app.cursor_screen = None;

    let editor_focused = app.focus == Focus::Editor;
    if app.split && app.pane_right_index().is_some() && body.width >= 34 {
        render_split(frame, body, app, editor_focused);
    } else {
        let index = app.editor.active_index();
        app.cursor_screen = render_pane(
            frame,
            body,
            &app.language,
            &mut app.editor.documents,
            &app.search,
            index,
            editor_focused,
            app.inline_diagnostics,
        );
    }
}

/// Draw the two panes and the focused-pane rule between them.
fn render_split(frame: &mut Frame, area: Rect, app: &mut App, editor_focused: bool) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 2),
            Constraint::Length(1),
            Constraint::Ratio(1, 2),
        ])
        .split(area);

    let rule: Vec<Line> = (0..columns[1].height)
        .map(|_| Line::from(Span::styled("│", theme::border(false))))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(rule)), columns[1]);

    let idle = Search::default();
    let left_focused = editor_focused && app.focus_pane == Pane::Primary;
    let right_focused = editor_focused && app.focus_pane == Pane::Secondary;
    let left_search = if left_focused { &app.search } else { &idle };
    let right_search = if right_focused { &app.search } else { &idle };
    let left_index = app.pane_left_index();
    let right_index = app.pane_right_index().unwrap_or(left_index);

    let left = render_pane(
        frame,
        columns[0],
        &app.language,
        &mut app.editor.documents,
        left_search,
        left_index,
        left_focused,
        app.inline_diagnostics,
    );
    if left_focused {
        app.cursor_screen = left;
    }

    let right = render_pane(
        frame,
        columns[2],
        &app.language,
        &mut app.editor.documents,
        right_search,
        right_index,
        right_focused,
        app.inline_diagnostics,
    );
    if right_focused {
        app.cursor_screen = right;
    }
}

/// Render one document into a pane, returning the cursor screen position when
/// the pane is focused and the cursor is visible.
#[allow(clippy::too_many_arguments)]
fn render_pane(
    frame: &mut Frame,
    area: Rect,
    language: &LanguageService,
    documents: &mut [Document],
    search: &Search,
    index: usize,
    focused: bool,
    inline_diagnostics: bool,
) -> Option<(u16, u16)> {
    let language_id = documents.get(index)?.buffer.language;
    let provider = language.provider(language_id);
    let doc = documents.get_mut(index)?;
    editor::render(
        frame,
        area,
        doc,
        provider,
        search,
        focused,
        inline_diagnostics,
    )
}

/// Center a rectangle of the given size within `area`.
pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}
