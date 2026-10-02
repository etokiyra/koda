//! Rendering smoke tests using ratatui's `TestBackend`.
//!
//! These prove the full UI composes real application state without a terminal.

use std::fs;
use std::path::{Path, PathBuf};

use koda::app::App;
use koda::commands::ids;
use koda::ui::theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn temp_project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("koda-render-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n    println!(\"hi\");\n}\n",
    )
    .unwrap();
    dir
}

fn draw(app: &mut App) -> String {
    draw_at(app, 120, 30)
}

fn draw_at(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| koda::ui::render(frame, app)).unwrap();

    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let symbol = buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ");
            out.push_str(symbol);
        }
        out.push('\n');
    }
    out
}

fn cleanup(dir: &Path) {
    fs::remove_dir_all(dir).ok();
}

#[test]
fn renders_project_tree_and_rust_file() {
    let dir = temp_project("editor");
    let file = dir.join("src/main.rs");
    let mut app = App::new(Some(&file)).unwrap();

    let screen = draw(&mut app);
    assert!(screen.contains("koda"), "header missing:\n{screen}");
    assert!(screen.contains("files"), "sidebar missing:\n{screen}");
    assert!(screen.contains("main.rs"), "file name missing:\n{screen}");
    assert!(screen.contains("fn main"), "code missing:\n{screen}");
    assert!(screen.contains("Rust"), "language missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn renders_welcome_when_no_file_open() {
    let dir = temp_project("welcome");
    let mut app = App::new(Some(&dir)).unwrap();

    let screen = draw(&mut app);
    assert!(screen.contains("K O D A"), "welcome missing:\n{screen}");
    assert!(
        screen.contains("quick open"),
        "shortcuts missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn command_palette_renders_overlay() {
    let dir = temp_project("palette");
    let file = dir.join("src/main.rs");
    let mut app = App::new(Some(&file)).unwrap();
    app.execute_command(ids::PALETTE);

    let screen = draw(&mut app);
    assert!(
        screen.contains("Command Palette"),
        "palette missing:\n{screen}"
    );
    assert!(screen.contains("Save"), "commands missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn statusline_uses_mellow_panel_background() {
    let dir = temp_project("statusbg");
    let file = dir.join("src/main.rs");
    let mut app = App::new(Some(&file)).unwrap();
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| koda::ui::render(frame, &mut app))
        .unwrap();

    let buffer = terminal.backend().buffer();
    let y = buffer.area.height - 1;
    let has_panel = (0..buffer.area.width).any(|x| {
        buffer
            .cell((x, y))
            .is_some_and(|cell| cell.bg == theme::PANEL_BG)
    });
    assert!(
        has_panel,
        "statusline should use the Mellow panel background"
    );
    cleanup(&dir);
}

#[test]
fn bracket_match_is_highlighted() {
    let dir = temp_project("bracket");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    let x = (1 + 2);\n}\n").unwrap();
    let mut app = App::new(Some(&file)).unwrap();
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(koda::editor::Position::new(1, 13));

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| koda::ui::render(frame, &mut app))
        .unwrap();

    let buffer = terminal.backend().buffer();
    let highlighted = (0..buffer.area.height).any(|y| {
        (0..buffer.area.width).any(|x| {
            buffer
                .cell((x, y))
                .is_some_and(|cell| cell.bg == theme::BRACKET_BG)
        })
    });
    assert!(highlighted, "matching brackets should be highlighted");
    cleanup(&dir);
}

#[test]
fn editor_background_stays_transparent() {
    let dir = temp_project("transparent");
    let file = dir.join("src/main.rs");
    let mut app = App::new(Some(&file)).unwrap();
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| koda::ui::render(frame, &mut app))
        .unwrap();

    let buffer = terminal.backend().buffer();
    let cell = buffer.cell((buffer.area.width - 1, 8)).unwrap();
    assert_eq!(
        cell.bg,
        Color::Reset,
        "editor background must stay transparent"
    );
    cleanup(&dir);
}

#[test]
fn tabs_keep_the_active_tab_visible_when_overflowing() {
    let dir = temp_project("tabs");
    let names = [
        "lib.rs",
        "engine.rs",
        "buffer.rs",
        "widgets.rs",
        "theme.rs",
        "art.rs",
    ];
    for name in names {
        fs::write(dir.join("src").join(name), "fn f() {}\n").unwrap();
    }

    let mut app = App::new(Some(&dir)).unwrap();
    for name in names {
        app.open_path(dir.join("src").join(name));
    }

    let screen = draw_at(&mut app, 56, 16);
    assert!(
        screen.contains("art.rs"),
        "the active tab should stay visible:\n{screen}"
    );
    assert!(
        screen.contains('‹') || screen.contains('›'),
        "overflow should be signalled:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn long_file_names_are_truncated_in_the_tree() {
    let dir = temp_project("longnames");
    let long = "a_very_long_file_name_that_should_be_truncated.rs";
    fs::write(dir.join(long), "fn f() {}\n").unwrap();

    let mut app = App::new(Some(&dir)).unwrap();
    let screen = draw_at(&mut app, 100, 20);
    assert!(
        screen.contains('…'),
        "long names should be clipped with an ellipsis:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn tree_filter_lists_matching_files() {
    let dir = temp_project("filter");
    fs::write(dir.join("src/engine.rs"), "fn f() {}\n").unwrap();

    let mut app = App::new(Some(&dir)).unwrap();
    app.execute_command(ids::FILTER_TREE);
    assert!(app.tree_filter.is_some());

    let filter = app.tree_filter.as_mut().unwrap();
    filter.push_char('e');
    filter.push_char('n');

    let screen = draw_at(&mut app, 100, 20);
    assert!(
        screen.contains("engine.rs"),
        "the filter should list matching files:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn statusline_shows_selection_size() {
    let dir = temp_project("selection-status");
    let file = dir.join("src/main.rs");
    let mut app = App::new(Some(&file)).unwrap();
    {
        let doc = app.editor.active_document_mut().unwrap();
        doc.selection = Some(koda::editor::Selection::new(koda::editor::Position::new(
            0, 0,
        )));
        doc.cursor = koda::editor::Position::new(0, 5);
    }

    let screen = draw_at(&mut app, 100, 20);
    assert!(
        screen.contains("sel"),
        "the selection size should be visible:\n{screen}"
    );
    cleanup(&dir);
}
