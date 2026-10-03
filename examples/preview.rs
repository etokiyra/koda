//! A tiny preview harness.
//!
//! Renders Koda into an in-memory terminal and prints it as text, so the layout
//! and ASCII art can be inspected from any environment.
//!
//! ```text
//! cargo run --example preview -- [width] [height] [mode]
//! mode: welcome | file | empty | palette | quick | find | filter | tabs | diagnostics | symbols | completion | hover | help | setup | split | toast | newproject | newproject-name | newproject-lang
//! ```

use koda::app::App;
use koda::commands::ids;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Build an app with `path` open, as the editor previews expect.
fn file_app(path: &std::path::Path) -> App {
    let mut app = App::new(Some(path)).expect("app");
    app.open_path(path.to_path_buf());
    app.pump_background(std::time::Duration::from_millis(300));
    // Detection runs behind git/tool probes; wait for it to settle.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while app
        .editor
        .active_document()
        .is_some_and(|doc| doc.buffer.language == koda::language::LanguageId::Unknown)
        && std::time::Instant::now() < deadline
    {
        app.pump_background(std::time::Duration::from_millis(20));
    }
    app
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let width: u16 = args.first().and_then(|s| s.parse().ok()).unwrap_or(110);
    let height: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let mode = args.get(2).map(String::as_str).unwrap_or("welcome");

    let root = std::env::current_dir().expect("cwd");

    let mut app = match mode {
        "empty-picker" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.execute_command(ids::QUICK_OPEN);
            if let koda::app::overlay::Overlay::Picker(picker) = &mut app.overlay {
                picker.query = "zzzz".to_string();
                picker.refilter();
            }
            app
        }
        "scene-starry" | "scene-cozy" | "scene-rainy" | "scene-sakura" | "scene-study" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.welcome_scene = match mode {
                "scene-cozy" => koda::ui::art::WelcomeScene::Cozy,
                "scene-rainy" => koda::ui::art::WelcomeScene::Rainy,
                "scene-sakura" => koda::ui::art::WelcomeScene::Sakura,
                "scene-study" => koda::ui::art::WelcomeScene::Study,
                _ => koda::ui::art::WelcomeScene::Starry,
            };
            app
        }
        "html" => {
            let path = std::env::temp_dir().join("koda-preview.html");
            std::fs::write(
                &path,
                "<!DOCTYPE html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <title>Koda</title>\n  </head>\n  <body>\n    <main id=\"app\" class=\"card\">\n      <h1>Hello</h1>\n    </main>\n  </body>\n</html>\n",
            )
            .unwrap();
            file_app(&path)
        }
        "css" => {
            let path = std::env::temp_dir().join("koda-preview.css");
            std::fs::write(
                &path,
                ":root {\n  color-scheme: light dark;\n}\n\n.card {\n  color: #ffcc00;\n  padding: 2rem;\n  display: flex;\n}\n",
            )
            .unwrap();
            file_app(&path)
        }
        "cfile" => {
            let path = std::env::temp_dir().join("koda-preview.c");
            std::fs::write(
                &path,
                "#include <stdio.h>\n\nstruct Point { int x; int y; };\n\nint add(int a, int b) {\n    // sum the pair\n    return a + b;\n}\n\nint main(void) {\n    printf(\"%d\\n\", add(1, 2));\n    return 0;\n}\n",
            )
            .unwrap();
            file_app(&path)
        }
        "file" => file_app(&root.join("src/main.rs")),
        "palette" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.execute_command(ids::PALETTE);
            app
        }
        "quick" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.execute_command(ids::QUICK_OPEN);
            app
        }
        "split" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.open_path(root.join("src/ui/mod.rs"));
            app.execute_command(ids::SPLIT);
            app
        }
        "toast" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.push_toast(koda::app::ToastKind::Success, "Formatted src/main.rs");
            app.push_toast(
                koda::app::ToastKind::Error,
                "Language server stopped; using built-in intelligence",
            );
            app
        }
        "find" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.execute_command(ids::FIND);
            app
        }
        "replace" => {
            let mut app = file_app(&root.join("src/main.rs"));
            app.execute_command(ids::REPLACE);
            app
        }
        "bare" => {
            let dir = std::env::temp_dir().join("koda-preview-bare");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            App::new(Some(&dir)).expect("app")
        }
        "newproject" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.execute_command(ids::NEW_PROJECT);
            app
        }
        "newproject-name" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.execute_command(ids::NEW_PROJECT);
            if let koda::app::overlay::Overlay::NewProject(flow) = &mut app.overlay {
                flow.step = koda::app::overlay::NewProjectStep::Name;
                flow.name = "my-app".to_string();
            }
            app
        }
        "newproject-lang" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.execute_command(ids::NEW_PROJECT);
            if let koda::app::overlay::Overlay::NewProject(flow) = &mut app.overlay {
                flow.step = koda::app::overlay::NewProjectStep::Language;
                flow.name = "my-app".to_string();
                flow.parent = root.clone();
            }
            app
        }
        "bracket" => {
            let path = std::env::temp_dir().join("koda-preview-bracket.rs");
            std::fs::write(&path, "fn main() {\n    let x = (1 + 2);\n}\n").unwrap();
            let mut app = file_app(&path);
            app.editor
                .active_document_mut()
                .unwrap()
                .move_to(koda::editor::Position::new(1, 13));
            app
        }
        "tabs" => {
            let mut app = file_app(&root.join("src/main.rs"));
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
            file_app(&path)
        }
        "diagnostics" => {
            let path = std::env::temp_dir().join("koda-preview-diagnostics.rs");
            std::fs::write(
                &path,
                "fn main() {\n    let values = [1, 2, 3;\n    println!(\"hi\");\n}\n",
            )
            .unwrap();
            let mut app = file_app(&path);
            let text = app.editor.active_document().unwrap().buffer.text();
            let language = app.editor.active_document().unwrap().buffer.language;
            let diagnostics = app.language.provider(language).diagnostics(&text);
            if let Some(doc) = app.editor.active_document_mut() {
                doc.set_diagnostics_revision(1);
                doc.apply_diagnostics(1, diagnostics);
                doc.move_to(koda::editor::Position::new(1, 17));
            }
            app.clear_status();
            app
        }
        "symbols" => {
            let mut app = App::new(Some(&root.join("src/ui/editor.rs"))).expect("app");
            app.execute_command(ids::SHOW_SYMBOLS);
            app
        }
        "completion" => {
            let mut app = App::new(Some(&root.join("src/app/mod.rs"))).expect("app");
            app.execute_command(ids::COMPLETE);
            if let Some(state) = app.completion.as_mut() {
                state.set_prefix("ren".to_string());
            }
            app
        }
        "hover" => {
            let path = std::env::temp_dir().join("koda-preview-hover.rs");
            std::fs::write(
                &path,
                "fn main() {\n    helper();\n    helper();\n}\n\nfn helper() {}\n",
            )
            .unwrap();
            let mut app = file_app(&path);
            app.editor
                .active_document_mut()
                .unwrap()
                .move_to(koda::editor::Position::new(1, 4));
            app.execute_command(ids::HOVER);
            app
        }
        "help" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.execute_command(ids::HELP);
            app
        }
        "setup" => {
            let mut app = App::new(Some(&root)).expect("app");
            app.execute_command(ids::SETUP);
            app
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
