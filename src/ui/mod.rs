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

use crate::app::overlay::Overlay;
use crate::app::{App, Focus};
use crate::language::LanguageId;

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

    match &app.overlay {
        Overlay::None => {}
        Overlay::Picker(picker) => overlay::render_picker(frame, area, picker),
        Overlay::Prompt(prompt) => overlay::render_prompt(frame, area, prompt),
    }
}

fn render_body(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.tree_visible {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(1)])
            .split(area);
        file_tree::render(
            frame,
            chunks[0],
            &app.workspace.tree,
            &app.workspace.git,
            app.focus == Focus::FileTree,
        );
        render_editor_area(frame, chunks[1], app);
    } else {
        render_editor_area(frame, area, app);
    }
}

fn render_editor_area(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.editor.is_empty() {
        welcome::render(frame, area, app);
        app.viewport_height = area.height.saturating_sub(2) as usize;
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    tabs::render(frame, chunks[0], &app.editor);

    let index = app.editor.active_index();
    let language = app
        .editor
        .documents
        .get(index)
        .map(|doc| doc.buffer.language)
        .unwrap_or(LanguageId::Unknown);
    let focused = app.focus == Focus::Editor;

    if let Some(doc) = app.editor.documents.get_mut(index) {
        let provider = app.language.provider(language);
        editor::render(frame, chunks[1], doc, provider, &app.search, focused);
    }

    app.viewport_height = chunks[1].height.saturating_sub(2) as usize;
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
