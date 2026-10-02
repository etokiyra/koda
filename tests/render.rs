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
    let backend = TestBackend::new(120, 30);
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
