//! Rendering smoke tests using ratatui's `TestBackend`.
//!
//! These prove the full UI composes real application state without a terminal.

use std::fs;
use std::path::{Path, PathBuf};

use koda::app::App;
use koda::commands::ids;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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
    assert!(screen.contains("PROJECT"), "sidebar missing:\n{screen}");
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
    assert!(screen.contains("k o d a"), "welcome missing:\n{screen}");
    assert!(
        screen.contains("Quick open"),
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
