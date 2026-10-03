//! Rendering smoke tests using ratatui's `TestBackend`.
//!
//! These prove the full UI composes real application state without a terminal.

use std::fs;
use std::path::{Path, PathBuf};

use koda::app::App;
use koda::app::ToastKind;
use koda::app::overlay::{DiffState, Overlay};
use koda::commands::ids;
use koda::language::diagnostics::{Diagnostic, Severity, TextPos};
use koda::ui::theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

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

/// Build an app and open `file`, settling detection, as these editor-rendering
/// tests expect a document to be active. Detection runs on the background worker
/// behind queued git/tool probes, so wait for the result rather than assuming a
/// fixed delay.
fn app_with_file(file: &Path) -> App {
    let mut app = App::new(Some(file)).unwrap();
    app.open_path(file.to_path_buf());
    let expected_known = std::fs::read(file)
        .map(|bytes| bytes.starts_with(b"#!"))
        .unwrap_or(false)
        || file
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| app.language.language_for_extension(extension).is_some());
    if expected_known {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app
            .editor
            .active_document()
            .is_some_and(|doc| doc.buffer.language == koda::language::LanguageId::Unknown)
            && std::time::Instant::now() < deadline
        {
            app.pump_background(std::time::Duration::from_millis(20));
        }
    } else {
        app.pump_background(std::time::Duration::from_millis(300));
    }
    app
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
    let mut app = app_with_file(&file);

    let screen = draw(&mut app);
    assert!(screen.contains("koda"), "header missing:\n{screen}");
    assert!(screen.contains("files"), "sidebar missing:\n{screen}");
    assert!(screen.contains("main.rs"), "file name missing:\n{screen}");
    assert!(screen.contains("fn main"), "code missing:\n{screen}");
    assert!(screen.contains("Rust"), "language missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn renders_indent_guides_at_the_detected_width() {
    let dir = temp_project("indent-guides");
    let file = dir.join("app.ts");
    fs::write(&file, "function f() {\n  if (x) {\n    y();\n  }\n}\n").unwrap();
    let mut app = app_with_file(&file);

    let screen = draw(&mut app);
    assert!(screen.contains('│'), "indent guide missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn renders_the_signature_help_popup() {
    use koda::app::overlay::SignatureState;
    use koda::language::lsp::convert::{Signature, SignatureHelp};

    let dir = temp_project("signature-popup");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.signature = Some(SignatureState {
        help: SignatureHelp {
            signatures: vec![Signature {
                label: "fn add(a: i32, b: i32) -> i32".to_string(),
                parameters: vec!["a: i32".to_string(), "b: i32".to_string()],
                documentation: Some("Adds two numbers".to_string()),
            }],
            active: 0,
            parameter: 1,
        },
        anchor: Some((10, 5)),
    });

    let screen = draw(&mut app);
    assert!(screen.contains("fn add"), "signature missing:\n{screen}");
    assert!(
        screen.contains("Adds two numbers"),
        "signature documentation missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn renders_a_unified_diff_overlay() {
    let dir = temp_project("diff-view");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.overlay = Overlay::Diff(DiffState::from_unified(
        "working tree · src/main.rs",
        "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1,2 @@\n fn main() {}\n+// added\n",
    ));

    let screen = draw(&mut app);
    assert!(
        screen.contains("working tree"),
        "diff title missing:\n{screen}"
    );
    assert!(screen.contains("@@"), "hunk marker missing:\n{screen}");
    assert!(screen.contains("added"), "added line missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn renders_inline_diagnostics() {
    let dir = temp_project("inline-diag");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    {
        let doc = app.editor.active_document_mut().unwrap();
        doc.set_lsp_diagnostics(vec![Diagnostic::new(
            TextPos::new(1, 4),
            TextPos::new(1, 9),
            Severity::Warning,
            "unused variable",
        )]);
    }

    let screen = draw(&mut app);
    assert!(
        screen.contains("unused variable"),
        "inline diagnostic missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn renders_welcome_when_no_file_open() {
    let dir = temp_project("welcome");
    let mut app = App::new(Some(&dir)).unwrap();

    let screen = draw(&mut app);
    assert!(screen.contains("K O D A"), "welcome missing:\n{screen}");
    assert!(
        screen.contains("Create a new project"),
        "welcome menu missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn renders_a_welcome_scene() {
    let dir = temp_project("scene");
    let mut app = App::new(Some(&dir)).unwrap();
    app.welcome_scene = koda::ui::art::WelcomeScene::Cozy;

    let screen = draw(&mut app);
    assert!(screen.contains('ω'), "familiar missing:\n{screen}");
    assert!(screen.contains("K O D A"), "wordmark missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn renders_every_welcome_scene() {
    let dir = temp_project("all-scenes");
    for scene in koda::ui::art::WelcomeScene::ALL {
        let mut app = App::new(Some(&dir)).unwrap();
        app.welcome_scene = scene;
        let screen = draw(&mut app);
        assert!(
            screen.contains('ω'),
            "{scene:?} should show the familiar:\n{screen}"
        );
        assert!(
            screen.contains("K O D A"),
            "{scene:?} should show the wordmark"
        );
    }
    cleanup(&dir);
}

#[test]
fn welcome_degrades_on_a_small_terminal() {
    let dir = temp_project("small-welcome");
    let mut app = App::new(Some(&dir)).unwrap();
    app.tree_visible = false;
    app.welcome_scene = koda::ui::art::WelcomeScene::Study;

    // Too short for the scene, so it falls back to the wordmark and menu.
    let screen = draw_at(&mut app, 44, 14);
    assert!(
        screen.contains("K O D A"),
        "the welcome should stay usable when small:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn frozen_motion_still_renders_the_welcome() {
    let dir = temp_project("frozen");
    let mut app = App::new(Some(&dir)).unwrap();
    app.motion = false;

    let screen = draw(&mut app);
    assert!(screen.contains("K O D A"), "welcome missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn command_palette_renders_overlay() {
    let dir = temp_project("palette");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::PALETTE);

    let screen = draw(&mut app);
    assert!(
        screen.contains("Command Palette"),
        "palette missing:\n{screen}"
    );
    assert!(screen.contains("Save"), "commands missing:\n{screen}");
    assert!(
        screen.contains("Esc close"),
        "picker footer missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn picker_empty_state_is_personable() {
    let dir = temp_project("picker-empty");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::QUICK_OPEN);
    if let Overlay::Picker(picker) = &mut app.overlay {
        picker.query = "zzzzzzzz".to_string();
        picker.refilter();
    }

    let screen = draw(&mut app);
    assert!(
        screen.contains('ω'),
        "the familiar should greet an empty result:\n{screen}"
    );
    assert!(screen.contains("no matches"), "missing message:\n{screen}");

    cleanup(&dir);
}

#[test]
fn prompt_shows_confirm_hint() {
    let dir = temp_project("prompt-hint");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::GOTO_LINE);

    let screen = draw(&mut app);
    assert!(
        screen.contains("Esc cancel"),
        "prompt hint missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn split_renders_both_documents() {
    let dir = temp_project("split-render");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&a, "fn main() {}\n").unwrap();
    fs::write(&b, "pub fn koda_split() {}\n").unwrap();
    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    app.execute_command(ids::SPLIT);

    let screen = draw(&mut app);
    assert!(screen.contains("fn main"), "left pane missing:\n{screen}");
    assert!(
        screen.contains("koda_split"),
        "right pane missing:\n{screen}"
    );
    assert!(screen.contains("│"), "separator missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn renders_toast_notification() {
    let dir = temp_project("toast-render");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.push_toast(ToastKind::Success, "Formatted main.rs");

    let screen = draw(&mut app);
    assert!(
        screen.contains("Formatted main.rs"),
        "toast missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn welcome_menu_lists_home_actions() {
    let dir = temp_project("welcome-menu");
    let mut app = App::new(Some(&dir)).unwrap();

    let screen = draw(&mut app);
    assert!(screen.contains("Open a file"), "menu missing:\n{screen}");
    assert!(
        screen.contains("Create a new project"),
        "create action missing:\n{screen}"
    );
    assert!(
        screen.contains("↑↓ choose"),
        "navigation hint missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn welcome_renders_on_a_small_terminal() {
    let dir = temp_project("welcome-small");
    let mut app = App::new(Some(&dir)).unwrap();

    let screen = draw_at(&mut app, 40, 10);
    assert!(screen.contains("K O D A"), "wordmark missing:\n{screen}");

    cleanup(&dir);
}

#[test]
fn new_project_overlay_renders_its_first_step() {
    let dir = temp_project("new-project-overlay");
    let mut app = App::new(Some(&dir)).unwrap();
    app.execute_command(ids::NEW_PROJECT);

    let screen = draw(&mut app);
    assert!(screen.contains("New Project"), "title missing:\n{screen}");
    assert!(
        screen.contains("use this folder"),
        "folder choice missing:\n{screen}"
    );

    cleanup(&dir);
}

#[test]
fn statusline_uses_mellow_panel_background() {
    let dir = temp_project("statusbg");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
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
    let mut app = app_with_file(&file);
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
            buffer.cell((x, y)).is_some_and(|cell| {
                cell.fg == theme::BRACKET_MATCH && cell.modifier.contains(Modifier::UNDERLINED)
            })
        })
    });
    assert!(highlighted, "matching brackets should be highlighted");
    cleanup(&dir);
}

#[test]
fn editor_background_stays_transparent() {
    let dir = temp_project("transparent");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
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
    let mut app = app_with_file(&file);
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

#[test]
fn gutter_and_statusline_show_diagnostics() {
    let dir = temp_project("diagnostics-ui");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    {
        let doc = app.editor.active_document_mut().unwrap();
        doc.set_diagnostics_revision(1);
        doc.apply_diagnostics(
            1,
            vec![Diagnostic::new(
                TextPos::new(0, 10),
                TextPos::new(0, 11),
                Severity::Error,
                "unclosed `{`; expected `}`",
            )],
        );
        doc.move_to(koda::editor::Position::new(0, 10));
    }
    // Let the diagnostic message show instead of the transient "Opened" status.
    app.clear_status();

    let screen = draw_at(&mut app, 100, 20);
    assert!(
        screen.contains('●'),
        "the gutter should mark the error:\n{screen}"
    );
    assert!(
        screen.contains("1✖"),
        "the statusline should count the error:\n{screen}"
    );
    assert!(
        screen.contains("unclosed"),
        "the statusline should explain the problem under the cursor:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn diagnostics_underline_the_affected_characters() {
    let dir = temp_project("diagnostics-underline");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    {
        let doc = app.editor.active_document_mut().unwrap();
        doc.set_diagnostics_revision(1);
        doc.apply_diagnostics(
            1,
            vec![Diagnostic::new(
                TextPos::new(0, 0),
                TextPos::new(0, 2),
                Severity::Error,
                "bad",
            )],
        );
    }

    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| koda::ui::render(frame, &mut app))
        .unwrap();

    // Collect every underlined grapheme; the diagnostic covers `fn`.
    let buffer = terminal.backend().buffer();
    let mut underlined = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y))
                && cell.modifier.contains(Modifier::UNDERLINED)
            {
                underlined.push(cell.symbol().to_string());
            }
        }
    }
    assert_eq!(
        underlined,
        vec!["f".to_string(), "n".to_string()],
        "diagnostic text should be underlined"
    );
    cleanup(&dir);
}

#[test]
fn completion_popup_is_drawn() {
    let dir = temp_project("completion-ui");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::COMPLETE);

    let screen = draw_at(&mut app, 100, 24);
    assert!(
        screen.contains("complete"),
        "the completion popup should be visible:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn hover_popup_is_drawn() {
    let dir = temp_project("hover-ui");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(koda::editor::Position::new(0, 3));
    app.execute_command(ids::HOVER);

    let screen = draw_at(&mut app, 100, 24);
    assert!(
        screen.contains("fn main"),
        "the hover popup should be visible:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn editor_keeps_context_below_the_cursor() {
    let dir = temp_project("scrolloff");
    let file = dir.join("src/main.rs");
    let source: String = (0..40).map(|line| format!("line {line}\n")).collect();
    fs::write(&file, source).unwrap();

    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(koda::editor::Position::new(30, 0));

    let screen = draw_at(&mut app, 100, 20);
    assert!(
        screen.contains("line 31"),
        "a scroll margin should keep context below the cursor:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn statusline_shows_the_line_ending() {
    let dir = temp_project("line-ending");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\r\n}\r\n").unwrap();

    let mut app = app_with_file(&file);
    let screen = draw_at(&mut app, 100, 20);
    assert!(
        screen.contains("CRLF"),
        "the line ending should appear in the statusline:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn welcome_mascot_blinks_on_later_frames() {
    let dir = temp_project("blink");
    // No file open, so the welcome scene is shown.
    let mut app = App::new(Some(&dir)).unwrap();

    app.anim_phase = 0;
    let calm = draw_at(&mut app, 90, 24);
    assert!(
        calm.contains("･ω･"),
        "the familiar should be awake and content by default:\n{calm}"
    );

    app.anim_phase = 7;
    let blink = draw_at(&mut app, 90, 24);
    assert!(
        blink.contains("-ω-"),
        "the familiar should blink on the blink frame:\n{blink}"
    );
    cleanup(&dir);
}

#[test]
fn help_overlay_lists_shortcuts() {
    let dir = temp_project("help");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::HELP);

    let screen = draw_at(&mut app, 100, 30);
    assert!(
        screen.contains("keyboard shortcuts"),
        "the help cheatsheet should be visible:\n{screen}"
    );
    assert!(
        screen.contains("Ctrl+S"),
        "the help cheatsheet should list real shortcuts:\n{screen}"
    );
    cleanup(&dir);
}

#[test]
fn command_palette_shows_a_no_matches_state() {
    let dir = temp_project("palette-empty");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::PALETTE);
    if let Overlay::Picker(picker) = &mut app.overlay {
        for c in "zzzzzz".chars() {
            picker.push_char(c);
        }
    }

    let screen = draw_at(&mut app, 100, 30);
    assert!(
        screen.contains("no matches"),
        "an empty filter should explain itself:\n{screen}"
    );
    cleanup(&dir);
}
