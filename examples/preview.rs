//! A tiny preview harness.
//!
//! Renders Koda into an in-memory terminal and prints it as text, so the layout
//! and ASCII art can be inspected from any environment.
//!
//! ```text
//! cargo run --example preview -- [width] [height] [mode]
//! mode: welcome | file | empty | palette | quick | find
//! ```

use koda::app::App;
use koda::commands::ids;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let width: u16 = args.first().and_then(|s| s.parse().ok()).unwrap_or(110);
    let height: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let mode = args.get(2).map(String::as_str).unwrap_or("welcome");

    let root = std::env::current_dir().expect("cwd");

    let mut app = match mode {
        "file" => App::new(Some(&root.join("src/main.rs"))).expect("app"),
        "palette" => {
            let mut app = App::new(Some(&root.join("src/main.rs"))).expect("app");
            app.execute_command(ids::PALETTE);
            app
        }
        "quick" => {
            let mut app = App::new(Some(&root.join("src/main.rs"))).expect("app");
            app.execute_command(ids::QUICK_OPEN);
            app
        }
        "find" => {
            let mut app = App::new(Some(&root.join("src/main.rs"))).expect("app");
            app.execute_command(ids::FIND);
            app
        }
        "replace" => {
            let mut app = App::new(Some(&root.join("src/main.rs"))).expect("app");
            app.execute_command(ids::REPLACE);
            app
        }
        "bare" => {
            let dir = std::env::temp_dir().join("koda-preview-bare");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            App::new(Some(&dir)).expect("app")
        }
        "bracket" => {
            let path = std::env::temp_dir().join("koda-preview-bracket.rs");
            std::fs::write(&path, "fn main() {\n    let x = (1 + 2);\n}\n").unwrap();
            let mut app = App::new(Some(&path)).expect("app");
            app.editor
                .active_document_mut()
                .unwrap()
                .move_to(koda::editor::Position::new(1, 13));
            app
        }
        "tabs" => {
            let mut app = App::new(Some(&root.join("src/main.rs"))).expect("app");
            for file in [
                "src/app/mod.rs",
                "src/editor/document.rs",
                "src/editor/buffer.rs",
                "src/language/detection/engine.rs",
                "src/ui/tabs.rs",
                "src/ui/file_tree.rs",
                "src/ui/art.rs",
            ] {
                app.open_path(root.join(file));
            }
            app
        }
        "filter" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.execute_command(ids::FILTER_TREE);
            if let Some(filter) = app.tree_filter.as_mut() {
                for c in "doc".chars() {
                    filter.push_char(c);
                }
            }
            app
        }
        "empty" => {
            let path = std::env::temp_dir().join("koda-preview-empty.rs");
            std::fs::write(&path, "").unwrap();
            App::new(Some(&path)).expect("app")
        }
        _ => App::new(Some(&root)).expect("app"),
    };

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| koda::ui::render(frame, &mut app))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(buffer.cell((x, y)).map(|cell| cell.symbol()).unwrap_or(" "));
        }
        println!("{}", line.trim_end());
    }
}
