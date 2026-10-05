use super::*;
use std::fs;

use super::overlay::{SETTINGS_ROWS, SettingsRow};
use crate::settings::{Settings, ThemeId};

fn temp_project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("koda-app-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {\n    let x = 1;\n}\n").unwrap();
    // Resolve symlinks (macOS exposes the temp dir as `/var` → `/private/var`)
    // so the paths a test builds match what Koda stores after canonicalising
    // the workspace root.
    fs::canonicalize(&dir).unwrap()
}

/// Build an app and open `file`, settling detection, as tests expect a
/// document to be active. Koda itself launches into the welcome screen.
///
/// Detection runs on the background worker behind any queued git/tool
/// probes, so wait for the result rather than assuming a fixed delay. Only
/// extensions a provider recognises are waited on; an unknown extension is
/// expected to stay [`LanguageId::Unknown`].
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
        // Detection runs on the background worker behind any queued git and
        // tool probes. Under a fully loaded test machine (many worker
        // threads and subprocess probes) it can take several seconds, so the
        // deadline is generous rather than tight.
        let deadline = Instant::now() + Duration::from_secs(15);
        while app
            .editor
            .active_document()
            .is_some_and(|doc| doc.buffer.language == LanguageId::Unknown)
            && Instant::now() < deadline
        {
            app.pump_background(Duration::from_millis(20));
        }
    } else {
        app.pump_background(Duration::from_millis(300));
    }
    app
}

#[test]
fn opens_rust_file_and_detects_language() {
    let dir = temp_project("open");
    let file = dir.join("src/main.rs");
    let app = app_with_file(&file);
    assert_eq!(app.editor.len(), 1);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Rust
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn saves_and_marks_clean() {
    let dir = temp_project("save");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// hello\n");
    assert!(app.editor.active_document().unwrap().is_dirty());

    app.execute_command(ids::SAVE);
    assert!(!app.editor.active_document().unwrap().is_dirty());
    assert!(fs::read_to_string(&file).unwrap().starts_with("// hello\n"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn revert_discards_edits_and_reloads_from_disk() {
    let dir = temp_project("revert");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// changed\n");
    assert!(app.editor.active_document().unwrap().is_dirty());

    app.execute_command(ids::REVERT);
    let doc = app.editor.active_document().unwrap();
    assert!(!doc.is_dirty());
    assert!(!doc.buffer.text().contains("// changed"));
    assert!(doc.buffer.text().contains("fn main"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_file_creates_and_opens_it() {
    let dir = temp_project("new-file");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::NEW_FILE);
    assert!(matches!(app.overlay, Overlay::Prompt(_)));
    app.submit_prompt(PromptKind::NewFile, "created.rs".to_string());

    let created = dir.join("src/created.rs");
    assert!(created.is_file());
    assert_eq!(
        app.editor.active_document().unwrap().buffer.path.as_deref(),
        Some(created.as_path())
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn duplicate_file_creates_a_copy_and_opens_it() {
    let dir = temp_project("duplicate");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::DUPLICATE_FILE);
    let copy = dir.join("src/main copy.rs");
    assert!(copy.is_file(), "expected {}", copy.display());
    assert_eq!(
        app.editor.active_document().unwrap().buffer.path.as_deref(),
        Some(copy.as_path())
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn copy_file_prompts_and_writes_the_copy() {
    let dir = temp_project("copy-file");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::COPY_FILE);
    assert!(matches!(
        &app.overlay,
        Overlay::Prompt(prompt) if prompt.kind == PromptKind::CopyFile
    ));
    app.submit_prompt(PromptKind::CopyFile, "src/backup.rs".to_string());
    assert!(dir.join("src/backup.rs").is_file());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn rename_updates_the_open_document() {
    let dir = temp_project("rename-file");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::RENAME_FILE);
    assert!(matches!(app.overlay, Overlay::Prompt(_)));
    app.submit_prompt(PromptKind::RenameFile, "renamed.rs".to_string());

    assert!(!file.exists());
    let renamed = dir.join("src/renamed.rs");
    assert!(renamed.is_file());
    assert_eq!(
        app.editor.active_document().unwrap().buffer.path.as_deref(),
        Some(renamed.as_path())
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn delete_removes_the_file_and_closes_its_tab() {
    let dir = temp_project("delete-file");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/extra.rs");
    fs::write(&b, "pub fn extra() {}\n").unwrap();
    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    assert_eq!(app.editor.len(), 2);

    app.delete_path(&b);
    assert!(!b.exists());
    assert_eq!(app.editor.len(), 1);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn kill_ring_supports_yank_pop() {
    let dir = temp_project("kill-ring");
    let file = dir.join("src/main.rs");
    fs::write(&file, "").unwrap();
    let mut app = app_with_file(&file);

    app.push_kill("first");
    app.push_kill("second");
    app.paste();
    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        "second"
    );

    app.yank_pop();
    assert_eq!(app.editor.active_document().unwrap().buffer.text(), "first");

    // Nothing older: the text is left alone.
    app.yank_pop();
    assert_eq!(app.editor.active_document().unwrap().buffer.text(), "first");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn yank_pop_is_refused_after_an_edit() {
    let dir = temp_project("yank-edit");
    let file = dir.join("src/main.rs");
    fs::write(&file, "").unwrap();
    let mut app = app_with_file(&file);
    app.push_kill("first");
    app.push_kill("second");
    app.paste();

    // Any further edit invalidates the yank-pop target.
    app.with_doc(|doc| doc.type_char('!'));
    app.yank_pop();
    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        "second!"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn toggle_comment_adds_prefix() {
    let dir = temp_project("comment");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 0));

    app.execute_command(ids::TOGGLE_COMMENT);

    let doc = app.editor.active_document().unwrap();
    assert!(doc.buffer.line_text(1).contains("// let x = 1;"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn command_palette_opens() {
    let dir = temp_project("palette");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::PALETTE);
    assert!(!app.overlay.is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn split_shows_two_documents_and_focus_switches() {
    let dir = temp_project("split");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&a, "fn main() {}\n").unwrap();
    fs::write(&b, "pub fn lib() {}\n").unwrap();
    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    assert_eq!(app.editor.len(), 2);
    let active = app.editor.active_index();

    app.execute_command(ids::SPLIT);
    assert!(app.split);
    assert_eq!(app.pane_left_index(), active);
    assert_eq!(app.pane_right_index(), Some((active + 1) % 2));
    assert_eq!(app.focus_pane, Pane::Primary);

    app.execute_command(ids::FOCUS_PANE);
    assert_eq!(app.focus_pane, Pane::Secondary);
    assert_eq!(app.editor.active_index(), (active + 1) % 2);

    // Typing edits only the focused (right) document.
    app.with_doc(|doc| doc.insert_text("// right\n"));
    let right = (active + 1) % 2;
    assert!(
        app.editor.documents[right]
            .buffer
            .text()
            .contains("// right")
    );
    assert!(
        !app.editor.documents[active]
            .buffer
            .text()
            .contains("// right")
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn alt_v_splits_and_alt_o_switches_panes() {
    let dir = temp_project("alt-split");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&b, "pub fn lib() {}\n").unwrap();
    let mut app = app_with_file(&a);
    app.open_path(b.clone());

    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    assert!(app.split);

    let before = app.editor.active_index();
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT));
    assert_ne!(app.editor.active_index(), before);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn closing_a_split_document_collapses_the_split() {
    let dir = temp_project("split-close");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&a, "fn main() {}\n").unwrap();
    fs::write(&b, "pub fn lib() {}\n").unwrap();
    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    app.execute_command(ids::SPLIT);
    assert!(app.split);

    // Close the focused (left) document; the split collapses to one pane.
    app.execute_command(ids::CLOSE_TAB);
    assert_eq!(app.editor.len(), 1);
    assert!(!app.split);
    assert!(app.pane_right_index().is_none());
    assert_eq!(app.editor.active_index(), app.pane_left_index());

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn focus_tree_command_toggles_focus() {
    let dir = temp_project("focus");
    let mut app = App::new(Some(&dir)).unwrap();
    assert_eq!(app.focus, Focus::Editor);

    app.execute_command(ids::FOCUS_TREE);
    assert_eq!(app.focus, Focus::FileTree);

    app.execute_command(ids::FOCUS_TREE);
    assert_eq!(app.focus, Focus::Editor);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_b_focuses_the_file_panel_then_hides_it() {
    let dir = temp_project("ctrl-b");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    assert!(app.tree_visible);
    assert_eq!(app.focus, Focus::Editor);

    // First press moves focus into the files.
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert!(app.tree_visible);
    assert_eq!(app.focus, Focus::FileTree);

    // Second press hides it and returns to the editor.
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert!(!app.tree_visible);
    assert_eq!(app.focus, Focus::Editor);

    // Third press reveals and focuses it again.
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert!(app.tree_visible);
    assert_eq!(app.focus, Focus::FileTree);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_shift_p_opens_the_command_palette() {
    let dir = temp_project("palette-shift");
    let file = dir.join("src/main.rs");

    // Terminals that report an uppercase `P` with only Control still open
    // the command palette, not quick open.
    let mut app = app_with_file(&file);
    app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::CONTROL));
    match &app.overlay {
        Overlay::Picker(picker) => assert_eq!(picker.title, "Command Palette"),
        _ => panic!("expected the command palette"),
    }

    // A plain `Ctrl+P` stays quick open.
    let mut app = app_with_file(&file);
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    match &app.overlay {
        Overlay::Picker(picker) => assert_eq!(picker.title, "Quick Open"),
        _ => panic!("expected quick open"),
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn palette_marks_unavailable_commands() {
    let dir = temp_project("palette-avail");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::PALETTE);

    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the command palette");
    };
    let find = |needle: &str| {
        (0..picker.filtered.len())
            .filter_map(|index| picker.item(index))
            .find(|item| item.label.contains(needle))
    };

    assert!(find("File: Save").expect("save").enabled);
    // Formatting availability tracks whether the tool is actually installed.
    let rustfmt_available = app
        .tools
        .as_ref()
        .map(|tools| tools.available(Tool::Rustfmt))
        .unwrap_or_else(|| format::is_available("rustfmt"));
    assert_eq!(
        find("Format Document").expect("format").enabled,
        rustfmt_available
    );
    assert!(
        !find("Rename Symbol").expect("rename").enabled,
        "rename is not available for Rust yet"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn quick_open_lists_recent_files_first() {
    let dir = temp_project("recents");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&b, "pub fn lib() {}\n").unwrap();

    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    app.execute_command(ids::QUICK_OPEN);

    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected quick open");
    };
    let first = picker.item(0).expect("an item");
    match &first.action {
        PickerAction::OpenPath(path) => {
            assert_eq!(path, &b.canonicalize().unwrap());
        }
        _ => panic!("expected a file"),
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn closing_a_dirty_tab_asks_first() {
    let dir = temp_project("close-dirty");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// x\n");

    app.execute_command(ids::CLOSE_TAB);
    assert_eq!(app.editor.len(), 1, "the dirty tab should stay open");

    app.execute_command(ids::CLOSE_TAB);
    assert_eq!(app.editor.len(), 0, "the second press closes it");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_all_writes_every_dirty_file() {
    let dir = temp_project("save-all");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&b, "pub fn f() {}\n").unwrap();

    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    app.editor.documents[0].insert_text("// a\n");
    app.editor.documents[1].insert_text("// b\n");

    app.execute_command(ids::SAVE_ALL);
    assert!(!app.editor.has_unsaved());
    assert!(fs::read_to_string(&a).unwrap().starts_with("// a\n"));
    assert!(fs::read_to_string(&b).unwrap().starts_with("// b\n"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn toggle_hidden_flips_tree_state() {
    let dir = temp_project("hidden");
    let mut app = App::new(Some(&dir)).unwrap();
    assert!(!app.workspace.tree.show_hidden);
    app.execute_command(ids::TOGGLE_HIDDEN);
    assert!(app.workspace.tree.show_hidden);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn refresh_picks_up_new_files() {
    let dir = temp_project("refresh");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    assert!(
        !app.workspace
            .tree
            .entries()
            .iter()
            .any(|entry| entry.name == "added.rs")
    );

    fs::write(dir.join("added.rs"), "pub fn added() {}\n").unwrap();
    app.execute_command(ids::REFRESH);
    assert!(
        app.workspace
            .tree
            .entries()
            .iter()
            .any(|entry| entry.name == "added.rs")
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn find_prefills_from_the_selection() {
    let dir = temp_project("find-prefill");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    {
        let doc = app.editor.active_document_mut().unwrap();
        doc.selection = Some(Selection::new(Position::new(0, 0)));
        doc.cursor = Position::new(0, 2);
    }

    app.execute_command(ids::FIND);
    assert_eq!(app.search.query, "fn");
    assert!(!app.search.matches.is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn search_options_change_the_matches() {
    let dir = temp_project("search-options");
    let file = dir.join("src/main.rs");
    fs::write(&file, "Foo foo food foo\n").unwrap();
    let mut app = app_with_file(&file);

    app.execute_command(ids::FIND);
    app.search.query = "foo".to_string();
    app.refresh_search_matches();
    assert_eq!(app.search.matches.len(), 4, "case-insensitive substring");

    // Alt+C toggles case sensitivity.
    app.handle_search_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
    assert!(app.search.case_sensitive);
    assert_eq!(app.search.matches.len(), 3);

    // Alt+W toggles whole-word matching.
    app.handle_search_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
    assert!(app.search.whole_word);
    assert_eq!(app.search.matches.len(), 2);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn regex_search_matches_and_reports_errors() {
    let dir = temp_project("search-regex");
    let file = dir.join("src/main.rs");
    fs::write(&file, "let count = 42;\nlet total = 7;\n").unwrap();
    let mut app = app_with_file(&file);

    app.execute_command(ids::FIND);
    app.handle_search_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
    assert!(app.search.regex);

    app.search.query = r"\d+".to_string();
    app.refresh_search_matches();
    assert_eq!(app.search.matches.len(), 2);
    assert!(app.search.regex_error.is_none());

    // Unsupported syntax is reported instead of matching silently wrong.
    app.search.query = "(a|b)".to_string();
    app.refresh_search_matches();
    assert!(app.search.matches.is_empty());
    assert!(app.search.regex_error.is_some());

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn replace_all_applies_every_match_as_one_edit() {
    let dir = temp_project("replace-all");
    let file = dir.join("src/main.rs");
    fs::write(&file, "foo foo foo\nbar foo\n").unwrap();
    let mut app = app_with_file(&file);

    app.execute_command(ids::REPLACE);
    app.search.query = "foo".to_string();
    app.search.replacement = "baz".to_string();
    app.refresh_search_matches();
    assert_eq!(app.search.matches.len(), 4);

    app.replace_all();
    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        "baz baz baz\nbar baz\n"
    );

    // A single undo restores the original.
    app.with_doc(|d| d.undo());
    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        "foo foo foo\nbar foo\n"
    );
    fs::remove_dir_all(&dir).ok();
}

/// Spin the background channel until the active document has diagnostics.
fn wait_for_diagnostics(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        app.apply_background_events();
        if app
            .editor
            .active_document()
            .is_some_and(|doc| !doc.diagnostics().is_empty())
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn diagnostics_are_computed_in_the_background() {
    let dir = temp_project("diagnostics");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    let x = 1;\n").unwrap();

    let mut app = app_with_file(&file);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Rust
    );
    app.poll_diagnostics();
    wait_for_diagnostics(&mut app);

    let doc = app.editor.active_document().unwrap();
    assert!(
        !doc.diagnostics().is_empty(),
        "expected an unclosed-brace diagnostic"
    );
    assert_eq!(doc.diagnostic_counts().0, 1);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn next_diagnostic_moves_the_cursor() {
    let dir = temp_project("diagnostics-next");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    let x = 1;\n").unwrap();

    let mut app = app_with_file(&file);
    app.poll_diagnostics();
    wait_for_diagnostics(&mut app);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(0, 0));

    app.execute_command(ids::DIAGNOSTICS_NEXT);
    let cursor = app.editor.active_document().unwrap().clamped_cursor();
    assert_eq!(cursor, Position::new(0, 10));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn diagnostics_list_opens_when_there_are_problems() {
    let dir = temp_project("diagnostics-list");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n").unwrap();

    let mut app = app_with_file(&file);
    app.poll_diagnostics();
    wait_for_diagnostics(&mut app);

    app.execute_command(ids::DIAGNOSTICS_LIST);
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the diagnostics list");
    };
    assert!(!picker.filtered.is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn symbol_outline_lists_definitions() {
    let dir = temp_project("symbols");
    let file = dir.join("src/main.rs");

    let mut app = app_with_file(&file);
    app.execute_command(ids::SHOW_SYMBOLS);

    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the symbol list");
    };
    let labels: Vec<String> = (0..picker.filtered.len())
        .filter_map(|index| picker.item(index))
        .map(|item| item.label.clone())
        .collect();
    assert!(labels.iter().any(|label| label == "main"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn go_to_definition_jumps_to_the_symbol() {
    let dir = temp_project("definition");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 4));

    app.execute_command(ids::GOTO_DEFINITION);
    let cursor = app.editor.active_document().unwrap().clamped_cursor();
    assert_eq!(cursor, Position::new(4, 3));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn go_to_definition_falls_back_to_the_project() {
    let dir = temp_project("definition-project");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&a, "fn main() {\n    helper();\n}\n").unwrap();
    fs::write(&b, "pub fn helper() {}\n").unwrap();
    let mut app = app_with_file(&a);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 4));

    app.execute_command(ids::GOTO_DEFINITION);

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        app.apply_background_events();
        if matches!(app.overlay, Overlay::Picker(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the workspace symbol picker");
    };
    let labels: Vec<String> = (0..picker.filtered.len())
        .filter_map(|index| picker.item(index))
        .map(|item| item.label.clone())
        .collect();
    assert!(
        labels.iter().any(|label| label == "helper"),
        "labels: {labels:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn find_references_lists_every_occurrence() {
    let dir = temp_project("references");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 4));

    app.execute_command(ids::FIND_REFERENCES);
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the references list");
    };
    assert_eq!(picker.filtered.len(), 2);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn completion_accepts_the_selected_candidate() {
    let dir = temp_project("complete");
    let file = dir.join("src/main.rs");
    fs::write(&file, "let counter = 0;\ncount\n").unwrap();

    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 5));
    app.execute_command(ids::COMPLETE);
    assert!(app.completion.is_some(), "completion should open");

    // Candidates are ordered shortest-first, so move to `counter`.
    if let Some(state) = app.completion.as_mut() {
        state.move_down();
    }
    app.accept_completion();
    assert_eq!(
        app.editor.active_document().unwrap().buffer.line_text(1),
        "counter"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn completion_offers_language_keywords() {
    let dir = temp_project("complete-keywords");
    let file = dir.join("src/main.rs");

    let mut app = app_with_file(&file);
    app.execute_command(ids::COMPLETE);
    let state = app.completion.as_ref().expect("completion should open");
    assert!(state.items.iter().any(|item| item.label == "fn"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn typing_offers_completion_automatically() {
    let dir = temp_project("complete-auto");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    let value = 1;\n}\n").unwrap();

    let mut app = app_with_file(&file);
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert!(
        app.completion.is_none(),
        "the popup waits for the typing pause"
    );

    std::thread::sleep(Duration::from_millis(160));
    app.poll_auto_completion();
    let state = app.completion.as_ref().expect("completion should open");
    assert!(
        state.items.iter().any(|item| item.label == "value"),
        "the buffer identifier should be offered"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn alt_shortcuts_toggle_inline_diagnostics_and_diff() {
    let dir = temp_project("alt-keys");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    let before = app.inline_diagnostics;
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::ALT));
    assert_ne!(
        app.inline_diagnostics, before,
        "Alt+I toggles inline diagnostics"
    );

    // Alt+D opens a diff; with no repository it explains itself but must not
    // panic or open an overlay.
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT));
    assert!(app.overlay.is_none());
    assert!(
        app.status_message()
            .unwrap_or("")
            .contains("Not a git repository"),
        "status was {:?}",
        app.status_message()
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn signature_help_response_populates_the_popup() {
    let dir = temp_project("signature");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.signature_request = Some((LanguageId::Rust, 7));
    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::SignatureHelp,
        7,
        Ok(serde_json::json!({
            "signatures": [{
                "label": "fn add(a: i32, b: i32)",
                "parameters": [{ "label": "a: i32" }, { "label": "b: i32" }],
                "documentation": "Adds two numbers"
            }],
            "activeSignature": 0,
            "activeParameter": 1
        })),
    );
    let signature = app.signature.as_ref().expect("signature state");
    assert_eq!(signature.help.active, 0);
    assert_eq!(signature.help.parameter, 1);
    assert_eq!(
        signature.help.signatures[0].parameters,
        vec!["a: i32", "b: i32"]
    );

    // A superseded response is ignored.
    app.signature_request = Some((LanguageId::Rust, 9));
    app.signature = None;
    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::SignatureHelp,
        8,
        Ok(serde_json::json!({ "signatures": [{ "label": "stale" }] })),
    );
    assert!(
        app.signature.is_none(),
        "a stale signature response is dropped"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn lsp_formatting_response_rewrites_the_buffer() {
    let dir = temp_project("lsp-format");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main(){}\n").unwrap();
    let mut app = app_with_file(&file);

    app.pending_lsp_format = Some((file.clone(), LanguageId::Rust, 3, 0));
    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Formatting,
        3,
        Ok(serde_json::json!([
            { "range": { "start": { "line": 0, "character": 9 },
                         "end": { "line": 0, "character": 9 } },
              "newText": " " }
        ])),
    );

    assert!(app.pending_lsp_format.is_none());
    assert_eq!(
        app.editor.active_document().unwrap().buffer.line_text(0),
        "fn main() {}"
    );

    // A superseded formatting response is ignored.
    app.pending_lsp_format = Some((file.clone(), LanguageId::Rust, 5, 0));
    let before = app.editor.active_document().unwrap().buffer.text();
    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Formatting,
        4,
        Ok(serde_json::json!([])),
    );
    assert_eq!(app.editor.active_document().unwrap().buffer.text(), before);
    assert!(app.pending_lsp_format.is_some());
    fs::remove_dir_all(&dir).ok();
}

/// A `WorkspaceEdit` JSON that replaces a range in `path`.
fn replace_edit(path: &Path, start: (u64, u64), end: (u64, u64), text: &str) -> Value {
    let mut changes = serde_json::Map::new();
    changes.insert(
        crate::language::lsp::path_to_uri(path),
        serde_json::json!([
            {
                "range": {
                    "start": { "line": start.0, "character": start.1 },
                    "end": { "line": end.0, "character": end.1 }
                },
                "newText": text
            }
        ]),
    );
    serde_json::json!({ "changes": changes })
}

#[test]
fn workspace_edit_applies_utf8_byte_offsets_at_character_boundaries() {
    let dir = temp_project("lsp-utf8-edit");
    let file = dir.join("src/main.rs");
    // `café` is four characters / five UTF-8 bytes starting at byte 4.
    fs::write(&file, "let café = 1;\n").unwrap();
    let mut app = app_with_file(&file);

    let applied = app.apply_workspace_edit_with_encoding(
        vec![convert::FileEdit {
            path: file.clone(),
            edits: vec![convert::TextEdit {
                start: (0, 4),
                end: (0, 9),
                new_text: "tea".to_string(),
            }],
        }],
        PositionEncoding::Utf8,
    );

    assert_eq!(applied, 1);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.line_text(0),
        "let tea = 1;"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn workspace_edit_applies_utf16_code_unit_offsets() {
    let dir = temp_project("lsp-utf16-edit");
    let file = dir.join("src/main.rs");
    // Same text, but a UTF-16 server counts `café` as four units (4..8).
    fs::write(&file, "let café = 1;\n").unwrap();
    let mut app = app_with_file(&file);

    let applied = app.apply_workspace_edit_with_encoding(
        vec![convert::FileEdit {
            path: file.clone(),
            edits: vec![convert::TextEdit {
                start: (0, 4),
                end: (0, 8),
                new_text: "tea".to_string(),
            }],
        }],
        PositionEncoding::Utf16,
    );

    assert_eq!(applied, 1);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.line_text(0),
        "let tea = 1;"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn stale_rename_response_is_discarded() {
    let dir = temp_project("lsp-stale-rename");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    let version = app.editor.active_document().unwrap().buffer.version;
    app.pending_rename_request = Some(PendingDocRequest {
        language: LanguageId::Rust,
        id: 1,
        path: file.clone(),
        version,
    });
    // The user keeps typing, so the response now describes older text.
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// x\n");
    let before = app.editor.active_document().unwrap().buffer.text();

    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Rename,
        1,
        Ok(replace_edit(&file, (0, 0), (0, 3), "renamed")),
    );

    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        before,
        "a stale rename must not edit the buffer"
    );
    assert!(app.pending_rename_request.is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn stale_formatting_response_is_discarded() {
    let dir = temp_project("lsp-stale-format");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    let stale_version = app.editor.active_document().unwrap().buffer.version;
    app.pending_lsp_format = Some((file.clone(), LanguageId::Rust, 2, stale_version));
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// x\n");
    let before = app.editor.active_document().unwrap().buffer.text();

    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Formatting,
        2,
        Ok(serde_json::json!([
            { "range": { "start": { "line": 0, "character": 0 },
                         "end": { "line": 0, "character": 3 } },
              "newText": "zzz" }
        ])),
    );

    assert_eq!(app.editor.active_document().unwrap().buffer.text(), before);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn diagnostics_for_a_dirty_buffer_are_dropped() {
    let dir = temp_project("lsp-dirty-diagnostics");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    let diagnostic = || {
        Diagnostic::new(
            crate::language::diagnostics::TextPos::new(0, 0),
            crate::language::diagnostics::TextPos::new(0, 1),
            Severity::Error,
            "boom",
        )
    };

    // The buffer changed but the server has not been told yet.
    app.editor.active_document_mut().unwrap().insert_text("x");
    app.apply_lsp_diagnostics(&file, vec![diagnostic()]);
    assert!(
        app.editor
            .active_document()
            .unwrap()
            .diagnostics()
            .is_empty(),
        "diagnostics over unsent text must be dropped"
    );

    // Once the change has been streamed, published diagnostics apply.
    app.editor
        .active_document_mut()
        .unwrap()
        .set_diagnostics_revision(1);
    app.apply_lsp_diagnostics(&file, vec![diagnostic()]);
    assert_eq!(app.editor.active_document().unwrap().diagnostics().len(), 1);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_timed_out_request_clears_its_pending_state() {
    let dir = temp_project("lsp-timeout-state");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.pending_lsp_format = Some((file.clone(), LanguageId::Rust, 9, 0));
    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Formatting,
        9,
        Err("request timed out".to_string()),
    );

    assert!(app.pending_lsp_format.is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn code_actions_are_not_applied_after_the_document_changes() {
    let dir = temp_project("lsp-stale-actions");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    let version = app.editor.active_document().unwrap().buffer.version;
    app.pending_code_actions_context = Some((file.clone(), version));
    app.pending_code_actions = vec![convert::CodeAction {
        title: "Add mut".to_string(),
        edit: Some(replace_edit(&file, (0, 0), (0, 3), "renamed")),
        command: None,
    }];
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// x\n");
    let before = app.editor.active_document().unwrap().buffer.text();

    app.apply_code_action(0);

    assert_eq!(app.editor.active_document().unwrap().buffer.text(), before);
    assert!(app.pending_code_actions.is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn motion_and_scene_commands_toggle() {
    let dir = temp_project("motion");
    let mut app = App::new(Some(&dir)).unwrap();

    assert!(app.motion, "motion defaults on");
    app.execute_command(ids::TOGGLE_MOTION);
    assert!(!app.motion, "motion toggles off");
    app.execute_command(ids::TOGGLE_MOTION);
    assert!(app.motion, "motion toggles back on");

    let scene = app.welcome_scene;
    app.execute_command(ids::WELCOME_SCENE);
    assert_ne!(app.welcome_scene, scene, "the scene cycles");

    // `v` on the welcome screen cycles too.
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert_ne!(app.welcome_scene, scene, "v cycles the scene");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn typing_a_space_dismisses_completion() {
    let dir = temp_project("complete-space");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    let value = 1;\n}\n").unwrap();

    let mut app = app_with_file(&file);
    app.execute_command(ids::COMPLETE);
    assert!(app.completion.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(app.completion.is_none(), "a space should dismiss the popup");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn automatic_completion_is_silent_in_comments() {
    let dir = temp_project("complete-comment");
    let file = dir.join("src/main.rs");
    fs::write(&file, "// a comment with words\nfn main() {}\n").unwrap();

    let mut app = app_with_file(&file);
    let end_of_comment = app
        .editor
        .active_document()
        .unwrap()
        .buffer
        .line_text(0)
        .chars()
        .count();
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(0, end_of_comment));
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

    std::thread::sleep(Duration::from_millis(160));
    app.poll_auto_completion();
    assert!(
        app.completion.is_none(),
        "no popup should appear inside a comment"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn stale_completion_response_is_dropped() {
    let dir = temp_project("complete-stale");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.execute_command(ids::COMPLETE);
    assert!(app.completion.is_some());

    // A newer request is outstanding; the older response must be ignored.
    app.completion_request = Some((LanguageId::Rust, 2));
    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Completion,
        1,
        Ok(serde_json::json!([{ "label": "stale_member" }])),
    );
    assert!(
        !app.completion
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|item| item.label == "stale_member"),
        "a superseded response must not be applied"
    );

    app.handle_lsp_response(
        LanguageId::Rust,
        RequestKind::Completion,
        2,
        Ok(serde_json::json!([{ "label": "fresh_member" }])),
    );
    assert!(
        app.completion
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|item| item.label == "fresh_member"),
        "the current response should be applied"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn member_access_excludes_buffer_words() {
    let dir = temp_project("complete-member");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    total = counter.\n}\n").unwrap();

    let mut app = app_with_file(&file);
    // Put the cursor right after the `.` on line 1.
    let line = app.editor.active_document().unwrap().buffer.line_text(1);
    let col = line.chars().count();
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, col));
    assert!(app.open_completion_inner(false));

    let state = app.completion.as_ref().unwrap();
    assert!(
        state
            .items
            .iter()
            .any(|item| item.label == "fn" || item.label == "let"),
        "language candidates remain available"
    );
    assert!(
        !state.items.iter().any(|item| item.label == "total"),
        "unrelated buffer words must not appear after a member operator"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn formatting_replaces_the_buffer_as_one_edit() {
    let dir = temp_project("format");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main(){let x=1;}\n").unwrap();

    let mut app = app_with_file(&file);
    let before = app.editor.active_document().unwrap().buffer.text();
    app.apply_formatted(&file, "fn main() {\n    let x = 1;\n}\n");
    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        "fn main() {\n    let x = 1;\n}\n"
    );
    assert!(app.editor.active_document().unwrap().is_dirty());

    app.with_doc(|doc| doc.undo());
    assert_eq!(app.editor.active_document().unwrap().buffer.text(), before);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn formatting_is_unavailable_for_plain_text() {
    let dir = temp_project("format-plain");
    let file = dir.join("notes.txt");
    fs::write(&file, "hello\n").unwrap();

    let mut app = app_with_file(&file);
    app.format_document();
    assert!(
        app.pending_format.is_none(),
        "plain text has no formatter, so no request should be sent"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn format_document_runs_the_formatter() {
    // A rustup shim can exist without the component installed (a minimal
    // toolchain profile), so require a successful run, not merely a spawn.
    if !std::process::Command::new("rustfmt")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return; // Skip when rustfmt is unavailable.
    }
    let dir = temp_project("format-flow");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main(){let x=1;}\n").unwrap();

    let mut app = app_with_file(&file);
    app.format_document();

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        app.apply_background_events();
        if app.pending_format.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let text = app.editor.active_document().unwrap().buffer.text();
    assert!(
        text.contains("fn main() {") && text.contains("    let x = 1;"),
        "expected formatted output, got:\n{text}"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn hover_describes_the_symbol_under_the_cursor() {
    let dir = temp_project("hover");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    helper();\n}\n\nfn helper() {}\n").unwrap();

    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 4));

    app.execute_command(ids::HOVER);
    let hover = app.hover.as_ref().expect("hover should open");
    assert!(hover.title.contains("helper"), "title was {}", hover.title);
    assert!(hover.body.iter().any(|line| line.contains("occurrence")));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn workspace_symbols_lists_definitions_across_files() {
    let dir = temp_project("workspace-symbols");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&b, "pub fn beta() {}\n").unwrap();

    let mut app = app_with_file(&a);
    app.open_workspace_symbols();

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        app.apply_background_events();
        if !matches!(app.overlay, Overlay::None) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the workspace symbol list");
    };
    let labels: Vec<String> = (0..picker.filtered.len())
        .filter_map(|index| picker.item(index))
        .map(|item| item.label.clone())
        .collect();
    assert!(
        labels.iter().any(|label| label == "beta"),
        "labels: {labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "main"),
        "labels: {labels:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn project_search_lists_matches_across_files() {
    let dir = temp_project("project-search");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&a, "fn main() { needle(); }\n").unwrap();
    fs::write(&b, "pub fn needle() {}\n").unwrap();

    let mut app = app_with_file(&a);
    app.execute_command(ids::PROJECT_SEARCH);
    assert!(matches!(app.overlay, Overlay::Prompt(_)));
    app.submit_prompt(PromptKind::ProjectSearch, "needle".to_string());

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        app.apply_background_events();
        if matches!(app.overlay, Overlay::Picker(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected search results");
    };
    assert!(picker.filtered.len() >= 2, "expected both files to match");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn changed_files_lists_git_status() {
    let dir = temp_project("changed-files");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    let mut files = std::collections::HashMap::new();
    files.insert(file.clone(), GitFileStatus::Modified);
    let mut staged = std::collections::HashSet::new();
    staged.insert(file.clone());
    app.workspace.git = crate::git::GitInfo {
        repo_root: Some(dir.clone()),
        branch: Some("main".to_string()),
        files,
        staged,
        available: true,
    };

    app.execute_command(ids::CHANGED_FILES);
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the changed-files picker");
    };
    assert!(!picker.filtered.is_empty());
    let item = picker.item(0).expect("an item");
    assert!(matches!(item.action, PickerAction::OpenPath(_)));
    assert!(item.detail.contains('M'), "detail was {}", item.detail);
    assert!(
        item.detail.contains("modified · staged"),
        "detail was {}",
        item.detail
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn commit_flow_reports_state_and_prompts() {
    let dir = temp_project("commit");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    // Not a repository: the command explains instead of prompting.
    app.workspace.git = crate::git::GitInfo::default();
    app.execute_command(ids::GIT_COMMIT);
    assert!(matches!(app.overlay, Overlay::None));
    assert!(
        app.status_message()
            .unwrap_or("")
            .contains("Not a git repository")
    );

    // With changes, it opens a commit-message prompt and dispatches work.
    let mut files = std::collections::HashMap::new();
    files.insert(file.clone(), GitFileStatus::Modified);
    app.workspace.git = crate::git::GitInfo {
        repo_root: Some(dir.clone()),
        branch: Some("main".to_string()),
        files,
        staged: std::collections::HashSet::new(),
        available: true,
    };
    app.execute_command(ids::GIT_COMMIT);
    assert!(matches!(app.overlay, Overlay::Prompt(_)));

    app.submit_prompt(PromptKind::CommitMessage, "a message".to_string());
    assert!(app.pending_commit);
    assert_eq!(app.busy(), Some("committing"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn space_in_changed_files_stages_the_selected_file() {
    let dir = temp_project("stage-space");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    let mut files = std::collections::HashMap::new();
    files.insert(file.clone(), GitFileStatus::Modified);
    app.workspace.git = crate::git::GitInfo {
        repo_root: Some(dir.clone()),
        branch: Some("main".to_string()),
        files,
        staged: std::collections::HashSet::new(),
        available: true,
    };

    app.execute_command(ids::CHANGED_FILES);
    assert!(matches!(app.overlay, Overlay::Picker(_)));

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(app.pending_stage_refresh);
    assert!(
        app.status_message().unwrap_or("").contains("Staging"),
        "status was {:?}",
        app.status_message()
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn diff_active_file_shows_working_tree_changes() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        return; // Skip when git is unavailable.
    }
    let dir = temp_project("diff-file");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
    };
    assert!(git(&["init", "-q"]).unwrap().status.success());
    assert!(
        git(&["config", "user.email", "koda@example.com"])
            .unwrap()
            .status
            .success()
    );
    assert!(
        git(&["config", "user.name", "Koda Test"])
            .unwrap()
            .status
            .success()
    );
    assert!(git(&["add", "-A"]).unwrap().status.success());
    assert!(
        git(&["commit", "-q", "-m", "init"])
            .unwrap()
            .status
            .success()
    );

    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    assert!(app.workspace.git.available, "expected a git repository");

    // Change the file on disk so the working tree differs from the index.
    fs::write(&file, "fn main() {\n    let y = 2;\n}\n").unwrap();
    app.diff_active_file();

    match &app.overlay {
        Overlay::Diff(diff) => assert!(
            diff.lines
                .iter()
                .any(|line| line.kind == overlay::DiffLineKind::Add && line.text.contains("let y")),
            "expected an added line, got {:?}",
            diff.lines.iter().map(|line| &line.text).collect::<Vec<_>>()
        ),
        _ => panic!("expected a diff overlay"),
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_shift_s_opens_save_as() {
    let dir = temp_project("save-as");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.handle_key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(matches!(
        &app.overlay,
        Overlay::Prompt(prompt) if prompt.kind == PromptKind::SaveAs
    ));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_n_opens_the_new_file_prompt() {
    let dir = temp_project("ctrl-n");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(matches!(
        &app.overlay,
        Overlay::Prompt(prompt) if prompt.kind == PromptKind::NewFile
    ));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn alt_m_jumps_to_the_matching_bracket() {
    let dir = temp_project("alt-m");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(0, 7));

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::ALT));
    assert_eq!(
        app.editor.active_document().unwrap().clamped_cursor(),
        Position::new(0, 8)
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn f3_advances_to_the_next_match() {
    let dir = temp_project("f3");
    let file = dir.join("src/main.rs");
    fs::write(&file, "foo\nfoo\nfoo\n").unwrap();
    let mut app = app_with_file(&file);
    app.execute_command(ids::FIND);
    app.search.query = "foo".to_string();
    app.refresh_search_matches();
    app.jump_to_match(0);

    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
    assert_eq!(app.search.current, Some(1));
    app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::SHIFT));
    assert_eq!(app.search.current, Some(0));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn toasts_deduplicate_and_cap() {
    let dir = temp_project("toasts");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.push_toast(ToastKind::Success, "Saved");
    app.push_toast(ToastKind::Success, "Saved");
    assert_eq!(app.toasts.len(), 1, "duplicate toasts are ignored");

    for index in 0..6 {
        app.push_toast(ToastKind::Info, format!("message {index}"));
    }
    assert_eq!(app.toasts.len(), 4, "toasts are capped");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn busy_reports_pending_background_work() {
    let dir = temp_project("busy");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    assert!(app.busy().is_none());

    app.pending_format = Some((file.clone(), 1));
    assert_eq!(app.busy(), Some("formatting"));

    app.pending_format = None;
    app.pending_workspace_symbols = Some(1);
    assert_eq!(app.busy(), Some("searching symbols"));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn language_setup_lists_discovered_tools() {
    let dir = temp_project("setup");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    // Tool discovery happens on the worker; wait briefly for it.
    let deadline = Instant::now() + Duration::from_secs(15);
    while app.tools.is_none() && Instant::now() < deadline {
        app.apply_background_events();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.tools.is_some(), "tool discovery should complete");

    app.execute_command(ids::SETUP);
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the language setup list");
    };
    assert!(picker.filtered.len() >= crate::language::tools::Tool::ALL.len());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn installing_a_tool_reports_progress() {
    let dir = temp_project("install");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.install_tool(Tool::Gofmt);
    assert_eq!(app.busy(), Some("installing"));
    assert!(app.pending_install.is_some());

    // Simulate the worker's result.
    app.apply_background_event(BackgroundEvent::ToolInstalled {
        tool: Tool::Gofmt,
        result: Err("gofmt: it ships with the Go toolchain".to_string()),
    });
    assert!(app.pending_install.is_none());
    assert!(app.status.error);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn language_setup_offers_install_for_missing_tools() {
    let dir = temp_project("setup-install");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    let deadline = Instant::now() + Duration::from_secs(15);
    while app.tools.is_none() && Instant::now() < deadline {
        app.apply_background_events();
        std::thread::sleep(Duration::from_millis(5));
    }
    // Only meaningful when rust-analyzer is missing and rustup is present.
    if !app
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.available(Tool::RustAnalyzer))
        || !crate::language::tools::can_install(Tool::RustAnalyzer)
    {
        fs::remove_dir_all(&dir).ok();
        return;
    }

    app.execute_command(ids::SETUP);
    let Overlay::Picker(picker) = &app.overlay else {
        panic!("expected the language setup list");
    };
    let item = (0..picker.filtered.len())
        .filter_map(|index| picker.item(index))
        .find(|item| item.label.contains("rust-analyzer"))
        .expect("rust-analyzer entry");
    assert!(
        item.enabled,
        "a missing installable tool should be actionable"
    );
    assert!(matches!(
        item.action,
        PickerAction::InstallTool(Tool::RustAnalyzer)
    ));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn offers_to_install_a_missing_language_server_once() {
    let dir = temp_project("offer-install");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    let deadline = Instant::now() + Duration::from_secs(15);
    while app.tools.is_none() && Instant::now() < deadline {
        app.apply_background_events();
        std::thread::sleep(Duration::from_millis(5));
    }

    let missing = app
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.available(Tool::RustAnalyzer));
    let installable = crate::language::tools::can_install(Tool::RustAnalyzer);
    let offered = app.maybe_offer_tool_setup();
    assert_eq!(
        offered,
        missing && installable,
        "the offer must track tool availability and installability"
    );
    if offered {
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the install prompt");
        };
        assert!(matches!(
            picker.item(0).map(|item| &item.action),
            Some(PickerAction::InstallTool(Tool::RustAnalyzer))
        ));
        // Dismissing must not re-offer in the same session.
        app.overlay = Overlay::None;
        assert!(!app.maybe_offer_tool_setup());
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn offers_setup_for_the_project_language_with_no_file_open() {
    let dir = temp_project("project-offer");
    let mut app = App::new(None).unwrap();
    assert!(app.open_workspace(dir.clone()));
    assert_eq!(app.workspace.project.kind.language(), LanguageId::Rust);

    let deadline = Instant::now() + Duration::from_secs(15);
    while app.tools.is_none() && Instant::now() < deadline {
        app.apply_background_events();
        std::thread::sleep(Duration::from_millis(5));
    }

    let missing = app
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.available(Tool::RustAnalyzer));
    let installable = crate::language::tools::can_install(Tool::RustAnalyzer);
    let offered = app.maybe_offer_tool_setup();
    assert_eq!(
        offered,
        missing && installable,
        "opening a Rust project must offer Rust tooling without an open file"
    );
    if offered {
        let Overlay::Picker(picker) = &app.overlay else {
            panic!("expected the install prompt");
        };
        assert!(matches!(
            picker.item(0).map(|item| &item.action),
            Some(PickerAction::InstallTool(Tool::RustAnalyzer))
        ));
    }
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn lsp_handshake_timeout_falls_back_to_builtin() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // A server that accepts input and never answers `initialize`.
    const MOCK: &str = "#!/bin/sh\ncat >/dev/null\n";
    let dir = temp_project("lsp-timeout");
    let file = dir.join("src/main.rs");
    let script = dir.join("mock-lsp.sh");
    {
        let mut handle = fs::File::create(&script).unwrap();
        handle.write_all(MOCK.as_bytes()).unwrap();
    }
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();

    let mut app = app_with_file(&file);
    app.start_lsp(LanguageId::Rust, script.to_str().unwrap(), &[]);
    // Pretend the handshake has been pending for too long.
    if let Some(job) = app.lsp.get_mut(&LanguageId::Rust) {
        job.started_at = Some(Instant::now() - Duration::from_secs(60));
    }

    assert!(app.poll_lsp_health());
    assert!(matches!(app.lsp_status(), LspStatus::Failed(_)));
    let job = app.lsp.get(&LanguageId::Rust).unwrap();
    assert!(job.server.is_none());
    assert!(job.started_at.is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn automatic_restart_stops_after_the_budget() {
    let dir = temp_project("restart-budget");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.lsp.insert(
        LanguageId::Rust,
        LspJob {
            restarts: MAX_LSP_RESTARTS,
            ..LspJob::default()
        },
    );

    app.schedule_lsp_restart(LanguageId::Rust);
    assert!(
        app.lsp.get(&LanguageId::Rust).unwrap().start_at.is_none(),
        "the budget is exhausted"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_successful_handshake_does_not_forgive_an_earlier_crash() {
    let dir = temp_project("restart-budget-reset");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.lsp.clear();
    app.lsp.insert(
        LanguageId::Rust,
        LspJob {
            restarts: 1,
            ..LspJob::default()
        },
    );

    // Connecting must not reset the budget on its own.
    app.lsp_ready(LanguageId::Rust);
    assert_eq!(
        app.lsp.get(&LanguageId::Rust).unwrap().restarts,
        1,
        "a fresh handshake must not grant a fresh restart budget"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn restart_server_reports_for_unsupported_files() {
    let dir = temp_project("restart-server");
    let file = dir.join("notes.txt");
    fs::write(&file, "hello\n").unwrap();
    let mut app = app_with_file(&file);

    app.execute_command(ids::RESTART_SERVER);
    assert!(app.lsp.is_empty());
    assert!(
        app.status_message()
            .unwrap_or("")
            .contains("No language server"),
        "status was {:?}",
        app.status_message()
    );
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn language_server_diagnostics_reach_the_document() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // Mock server: answer `initialize` (id 1) and publish one diagnostic
    // for the file passed as the first argument.
    const MOCK: &str = r#"#!/bin/sh
target="$1"
read -r header
len=$(printf '%s' "$header" | tr -dc '0-9')
read -r blank
dd bs=1 count="$len" of=/dev/null 2>/dev/null
resp='{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}'
printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
note=$(printf '{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file://%s","diagnostics":[{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":3}},"severity":1,"message":"boom"}]}}' "$target")
printf 'Content-Length: %s\r\n\r\n%s' "${#note}" "$note"
cat >/dev/null
"#;

    let dir = temp_project("lsp-wire");
    let file = dir.join("src/main.rs");
    let script = dir.join("mock-lsp.sh");
    {
        let mut handle = fs::File::create(&script).unwrap();
        handle.write_all(MOCK.as_bytes()).unwrap();
    }
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();

    let mut app = app_with_file(&file);
    app.start_lsp(
        LanguageId::Rust,
        script.to_str().unwrap(),
        &[file.to_str().unwrap()],
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut applied = false;
    while Instant::now() < deadline {
        app.poll_lsp();
        applied = app
            .editor
            .active_document()
            .is_some_and(|doc| doc.diagnostics_from_lsp() && !doc.diagnostics().is_empty());
        if applied {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(applied, "expected language-server diagnostics");
    assert_eq!(app.lsp_status(), LspStatus::Ready);
    assert_eq!(
        app.editor.active_document().unwrap().diagnostics()[0].message,
        "boom"
    );
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn documents_opened_after_ready_are_synced_to_the_server() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // Mock server: answer `initialize`, append every `didOpen` URI to the
    // log file named by the first argument, and stay alive.
    const MOCK: &str = r#"#!/bin/sh
log="$1"
while read -r header; do
  header=$(printf '%s' "$header" | tr -d '\r')
  case "$header" in
    Content-Length:*) len=${header#Content-Length: } ;;
    *) continue ;;
  esac
  read -r blank
  body=$(dd bs=1 count="$len" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$body" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  if [ "$method" = "textDocument/didOpen" ]; then
    uri=$(printf '%s' "$body" | sed -n 's/.*"uri":"\([^"]*\)".*/\1/p')
    printf '%s\n' "$uri" >> "$log"
  fi
  [ -z "$id" ] && continue
  case "$method" in
    initialize) result='{"capabilities":{}}' ;;
    *) result='null' ;;
  esac
  resp=$(printf '{"jsonrpc":"2.0","id":%s,"result":%s}' "$id" "$result")
  printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
done
"#;

    let dir = temp_project("lsp-late-open");
    let first = dir.join("src/main.rs");
    let second = dir.join("src/other.rs");
    fs::write(&second, "pub fn other() {}\n").unwrap();
    let log = dir.join("opens.log");
    let script = dir.join("mock-lsp.sh");
    {
        let mut handle = fs::File::create(&script).unwrap();
        handle.write_all(MOCK.as_bytes()).unwrap();
    }
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();

    let mut app = app_with_file(&first);
    // Drop any scheduled auto-start so only our mock runs.
    app.lsp.clear();
    app.start_lsp(
        LanguageId::Rust,
        script.to_str().unwrap(),
        &[log.to_str().unwrap()],
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline
        && !app
            .lsp
            .get(&LanguageId::Rust)
            .and_then(|job| job.server.as_ref())
            .is_some_and(|server| server.is_ready())
    {
        app.poll_lsp();
        std::thread::sleep(Duration::from_millis(5));
    }

    // Opening a second file after the handshake must attach it too.
    app.open_path(second.clone());
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut synced = false;
    while Instant::now() < deadline && !synced {
        app.apply_background_events();
        app.poll_lsp();
        synced = fs::read_to_string(&log)
            .map(|log| log.contains("other.rs"))
            .unwrap_or(false);
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(
        synced,
        "a document opened after ready must be sent to the server"
    );
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn a_second_language_server_starts_alongside_the_first() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // A minimal server that answers `initialize` and stays alive.
    const MOCK: &str = r#"#!/bin/sh
while read -r header; do
  header=$(printf '%s' "$header" | tr -d '\r')
  case "$header" in
    Content-Length:*) len=${header#Content-Length: } ;;
    *) continue ;;
  esac
  read -r blank
  body=$(dd bs=1 count="$len" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$body" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  [ -z "$id" ] && continue
  case "$method" in
    initialize) result='{"capabilities":{}}' ;;
    *) result='null' ;;
  esac
  resp=$(printf '{"jsonrpc":"2.0","id":%s,"result":%s}' "$id" "$result")
  printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
done
"#;

    let dir = temp_project("lsp-multi");
    let file = dir.join("src/main.rs");
    let script = dir.join("mock-lsp.sh");
    {
        let mut handle = fs::File::create(&script).unwrap();
        handle.write_all(MOCK.as_bytes()).unwrap();
    }
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();

    let mut app = app_with_file(&file);
    app.start_lsp(LanguageId::Rust, script.to_str().unwrap(), &[]);
    app.start_lsp(LanguageId::Python, script.to_str().unwrap(), &[]);
    assert_eq!(app.lsp.len(), 2, "each language gets its own server");

    let deadline = Instant::now() + Duration::from_secs(15);
    let ready = |app: &App| {
        [LanguageId::Rust, LanguageId::Python]
            .iter()
            .all(|language| {
                app.lsp
                    .get(language)
                    .and_then(|job| job.server.as_ref())
                    .is_some_and(|server| server.is_ready())
            })
    };
    while !ready(&app) && Instant::now() < deadline {
        app.poll_lsp();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ready(&app), "both servers should complete the handshake");
    assert_eq!(app.lsp_status(), LspStatus::Ready);
    fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn language_server_answers_hover_completion_and_definition() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    // A mock server that answers each feature request by method.
    const MOCK: &str = r#"#!/bin/sh
target="$1"
while read -r header; do
  header=$(printf '%s' "$header" | tr -d '\r')
  case "$header" in
    Content-Length:*) len=${header#Content-Length: } ;;
    *) continue ;;
  esac
  read -r blank
  body=$(dd bs=1 count="$len" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$body" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  [ -z "$id" ] && continue
  case "$method" in
    initialize) result='{"capabilities":{"completionProvider":true,"hoverProvider":true,"definitionProvider":true,"referencesProvider":true,"renameProvider":true,"codeActionProvider":true,"workspaceSymbolProvider":true}}' ;;
    textDocument/hover) result='{"contents":{"kind":"plaintext","value":"LSP HOVER TEXT"}}' ;;
    textDocument/completion) result='{"items":[{"label":"koda_lsp_item","kind":3}]}' ;;
    textDocument/definition) result="[{\"uri\":\"file://$target\",\"range\":{\"start\":{\"line\":2,\"character\":0},\"end\":{\"line\":2,\"character\":5}}}]" ;;
    textDocument/references) result="[{\"uri\":\"file://$target\",\"range\":{\"start\":{\"line\":1,\"character\":4},\"end\":{\"line\":1,\"character\":9}}}]" ;;
    textDocument/rename) result="{\"changes\":{\"file://$target\":[{\"range\":{\"start\":{\"line\":1,\"character\":8},\"end\":{\"line\":1,\"character\":13}},\"newText\":\"renamed\"}]}}" ;;
    textDocument/codeAction) result="[{\"title\":\"Apply fix\",\"edit\":{\"changes\":{\"file://$target\":[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":0}},\"newText\":\"FIX \"}]}}}]" ;;
    workspace/symbol) result="[{\"name\":\"koda_ws_symbol\",\"kind\":12,\"location\":{\"uri\":\"file://$target\",\"range\":{\"start\":{\"line\":3,\"character\":0},\"end\":{\"line\":3,\"character\":3}}}}]" ;;
    *) result='null' ;;
  esac
  resp=$(printf '{"jsonrpc":"2.0","id":%s,"result":%s}' "$id" "$result")
  printf 'Content-Length: %s\r\n\r\n%s' "${#resp}" "$resp"
done
"#;

    let dir = temp_project("lsp-features");
    let file = dir.join("src/main.rs");
    fs::write(&file, "fn main() {\n    let value = 1;\n    value;\n}\n").unwrap();
    let script = dir.join("mock-lsp.sh");
    {
        let mut handle = fs::File::create(&script).unwrap();
        handle.write_all(MOCK.as_bytes()).unwrap();
    }
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();

    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .set_language(LanguageId::Rust);
    app.start_lsp(
        LanguageId::Rust,
        script.to_str().unwrap(),
        &[file.to_str().unwrap()],
    );
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(2, 4));

    let wait = |app: &mut App, predicate: &dyn Fn(&App) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            app.poll_lsp();
            if predicate(app) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    };

    assert!(
        wait(&mut app, &|app| app
            .lsp
            .get(&LanguageId::Rust)
            .and_then(|job| job.server.as_ref())
            .is_some_and(|server| server.is_ready())),
        "server should become ready"
    );

    // Hover.
    app.open_hover();
    assert!(
        wait(&mut app, &|app| app
            .hover
            .as_ref()
            .is_some_and(|hover| hover.title.contains("LSP HOVER"))),
        "expected the server's hover"
    );

    // Completion.
    app.hover = None;
    app.open_completion();
    assert!(
        wait(&mut app, &|app| app.completion.as_ref().is_some_and(
            |state| state.items.iter().any(|item| item.label == "koda_lsp_item")
        )),
        "expected the server's completion"
    );

    // Definition (same file, line 2 column 0).
    app.completion = None;
    app.goto_definition();
    assert!(
        wait(&mut app, &|app| app.editor.active_document().is_some_and(
            |doc| doc.clamped_cursor() == Position::new(2, 0)
        )),
        "expected the server's definition to move the cursor"
    );

    // References (the server returns one location).
    app.find_references();
    assert!(
        wait(&mut app, &|app| matches!(app.overlay, Overlay::Picker(_))),
        "expected the references picker"
    );

    // Rename (the server returns a workspace edit).
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 8));
    app.rename_symbol();
    assert!(
        matches!(app.overlay, Overlay::Prompt(_)),
        "rename should prompt for a new name"
    );
    app.submit_prompt(PromptKind::Rename, "renamed".to_string());
    assert!(
        wait(&mut app, &|app| app.editor.active_document().is_some_and(
            |doc| doc.buffer.line_text(1).contains("renamed")
        )),
        "expected the rename edit to apply"
    );
    assert_eq!(
        app.editor.active_document().unwrap().buffer.line_text(1),
        "    let renamed = 1;"
    );

    // Code actions (the server returns one edit-based action).
    app.code_actions();
    assert!(
        wait(&mut app, &|app| matches!(app.overlay, Overlay::Picker(_))),
        "expected the code actions picker"
    );
    app.apply_code_action(0);
    assert!(
        wait(&mut app, &|app| app.editor.active_document().is_some_and(
            |doc| doc.buffer.line_text(0).starts_with("FIX ")
        )),
        "expected the code action edit to apply"
    );

    // Workspace symbols: the server's results are merged into the picker.
    app.open_workspace_symbols();
    assert!(
        wait(&mut app, &|app| match &app.overlay {
            Overlay::Picker(picker) => (0..picker.filtered.len())
                .filter_map(|index| picker.item(index))
                .any(|item| item.label == "koda_ws_symbol"),
            _ => false,
        }),
        "expected the server's workspace symbol"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn captures_and_restores_open_files() {
    let dir = temp_project("session-restore");
    let a = dir.join("src/main.rs");
    let b = dir.join("src/lib.rs");
    fs::write(&b, "pub fn lib() {\n    let x = 1;\n}\n").unwrap();

    let mut app = app_with_file(&a);
    app.open_path(b.clone());
    app.editor
        .active_document_mut()
        .unwrap()
        .move_to(Position::new(1, 4));

    let session = app.capture_session();
    assert_eq!(session.files.len(), 2);
    assert_eq!(session.active, 1);

    let mut fresh = App::new(Some(&dir)).unwrap();
    fresh.restore_session(session);
    assert_eq!(fresh.editor.len(), 2);
    assert_eq!(fresh.editor.active_index(), 1);
    assert_eq!(
        fresh.editor.active_document().unwrap().clamped_cursor(),
        Position::new(1, 4)
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn reloads_clean_files_changed_on_disk() {
    let dir = temp_project("external-clean");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    std::thread::sleep(Duration::from_millis(10));
    fs::write(&file, "fn changed() {}\n").unwrap();
    app.last_disk_check = Instant::now() - Duration::from_secs(5);

    assert!(app.poll_external_changes());
    assert_eq!(
        app.editor.active_document().unwrap().buffer.text(),
        "fn changed() {}\n"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_markup_and_config_languages() {
    let dir = temp_project("config-langs");
    let cases = [
        ("notes.md", "# Title\n", LanguageId::Markdown),
        ("data.json", "{\"a\": 1}\n", LanguageId::Json),
        ("config.yaml", "a: true\n", LanguageId::Yaml),
    ];
    for (name, content, expected) in cases {
        let file = dir.join(name);
        fs::write(&file, content).unwrap();
        let app = app_with_file(&file);
        let language = app.editor.active_document().unwrap().buffer.language;
        assert_eq!(language, expected, "detected {name} as {language:?}");
    }
    // `Cargo.toml` is a TOML file, even though it marks a Rust project.
    let manifest = dir.join("Cargo.toml");
    let app = app_with_file(&manifest);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Toml
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_c_and_cpp() {
    let dir = temp_project("c-langs");
    let cases = [
        (
            "main.c",
            "#include <stdio.h>\nint main(void) { return 0; }\n",
            LanguageId::C,
        ),
        (
            "app.cpp",
            "#include <iostream>\nint main() { std::cout << 1; }\n",
            LanguageId::Cpp,
        ),
        (
            "view.hpp",
            "class View { public: int w; };\n",
            LanguageId::Cpp,
        ),
    ];
    for (name, content, expected) in cases {
        let file = dir.join(name);
        fs::write(&file, content).unwrap();
        let app = app_with_file(&file);
        let language = app.editor.active_document().unwrap().buffer.language;
        assert_eq!(language, expected, "detected {name} as {language:?}");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_html_and_css() {
    let dir = temp_project("web-markup");
    let cases = [
        (
            "index.html",
            "<!DOCTYPE html>\n<html><body><h1>Hi</h1></body></html>\n",
            LanguageId::Html,
        ),
        (
            "style.css",
            "body {\n  color: #333;\n  display: flex;\n}\n",
            LanguageId::Css,
        ),
    ];
    for (name, content, expected) in cases {
        let file = dir.join(name);
        fs::write(&file, content).unwrap();
        let app = app_with_file(&file);
        let language = app.editor.active_document().unwrap().buffer.language;
        assert_eq!(language, expected, "detected {name} as {language:?}");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_java_and_csharp() {
    let dir = temp_project("jvm-langs");
    let cases = [
        (
            "Main.java",
            "public class Main {\n    public static void main(String[] args) {}\n}\n",
            LanguageId::Java,
        ),
        (
            "Program.cs",
            "using System;\n\nclass Program {\n    static void Main() {}\n}\n",
            LanguageId::CSharp,
        ),
    ];
    for (name, content, expected) in cases {
        let file = dir.join(name);
        fs::write(&file, content).unwrap();
        let app = app_with_file(&file);
        let language = app.editor.active_document().unwrap().buffer.language;
        assert_eq!(language, expected, "detected {name} as {language:?}");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_typescript_and_javascript() {
    let dir = temp_project("web-langs");
    let cases = [
        (
            "app.ts",
            "export const total: number = 1;\n",
            LanguageId::TypeScript,
        ),
        (
            "view.tsx",
            "export const App = () => null;\n",
            LanguageId::TypeScript,
        ),
        ("index.js", "const total = 1;\n", LanguageId::JavaScript),
        (
            "component.jsx",
            "export default function App() { return null; }\n",
            LanguageId::JavaScript,
        ),
    ];
    for (name, content, expected) in cases {
        let file = dir.join(name);
        fs::write(&file, content).unwrap();
        let app = app_with_file(&file);
        let language = app.editor.active_document().unwrap().buffer.language;
        assert_eq!(language, expected, "detected {name} as {language:?}");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_python_from_extension_and_shebang() {
    let dir = temp_project("python");
    let file = dir.join("app.py");
    fs::write(&file, "def main():\n    pass\n").unwrap();
    let app = app_with_file(&file);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Python
    );

    // An extensionless script is recognised from its shebang.
    let script = dir.join("tool");
    fs::write(&script, "#!/usr/bin/env python3\nprint('hi')\n").unwrap();
    let app = app_with_file(&script);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Python
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn detects_shell_from_extension_and_shebang() {
    let dir = temp_project("shell");
    let file = dir.join("build.sh");
    fs::write(&file, "#!/usr/bin/env bash\necho hi\n").unwrap();
    let app = app_with_file(&file);
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Shell
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn warns_when_a_dirty_file_changes_on_disk() {
    let dir = temp_project("external-dirty");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.editor
        .active_document_mut()
        .unwrap()
        .insert_text("// mine\n");

    std::thread::sleep(Duration::from_millis(10));
    fs::write(&file, "fn theirs() {}\n").unwrap();
    app.last_disk_check = Instant::now() - Duration::from_secs(5);

    assert!(app.poll_external_changes());
    assert!(app.status.error);
    assert!(
        app.status_message()
            .unwrap_or("")
            .contains("changed on disk")
    );
    // The edit is preserved rather than silently overwritten.
    assert!(
        app.editor
            .active_document()
            .unwrap()
            .buffer
            .text()
            .starts_with("// mine")
    );
    fs::remove_dir_all(&dir).ok();
}

// ------------------------------------------------------------------
// Welcome screen and project creation
// ------------------------------------------------------------------

#[test]
fn startup_shows_the_welcome_screen_for_a_file_target() {
    let dir = temp_project("startup-file");
    let file = dir.join("src/main.rs");
    let app = App::new(Some(&file)).unwrap();

    assert!(
        app.editor.is_empty(),
        "a CLI file must not be opened automatically"
    );
    assert!(app.welcome_active());
    let labels: Vec<String> = app
        .welcome_items()
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert!(labels.iter().any(|label| label.contains("Open a file")));
    assert!(
        labels
            .iter()
            .any(|label| label.contains("Create a new project"))
    );
    assert!(
        labels.iter().any(|label| label.contains("main.rs")),
        "the CLI target should be offered: {labels:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_shows_the_welcome_screen_for_a_directory_target() {
    let dir = temp_project("startup-dir");
    let app = App::new(Some(&dir)).unwrap();
    assert!(app.editor.is_empty());
    assert!(app.welcome_active());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn welcome_target_action_opens_the_file() {
    let dir = temp_project("welcome-open");
    let file = dir.join("src/main.rs");
    let mut app = App::new(Some(&file)).unwrap();

    // Find and activate the command-line target row.
    let action = app
        .welcome_items()
        .into_iter()
        .find(|item| matches!(item.action, WelcomeAction::OpenTarget(_)))
        .expect("a target action");
    app.activate_welcome(action.action);
    assert_eq!(app.editor.len(), 1);
    // Detection runs behind the queued git/tool probes; wait for the result.
    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .editor
        .active_document()
        .is_some_and(|doc| doc.buffer.language != LanguageId::Rust)
        && Instant::now() < deadline
    {
        app.pump_background(Duration::from_millis(20));
    }
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Rust
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resume_restores_the_saved_session() {
    let dir = temp_project("welcome-resume");
    let file = dir.join("src/main.rs");
    let app = app_with_file(&file);
    app.save_session();

    let mut fresh = App::new(Some(&file)).unwrap();
    assert!(
        fresh.resume_session.is_some(),
        "a saved session should be offered"
    );
    assert!(fresh.editor.is_empty());
    fresh.activate_welcome(WelcomeAction::Resume);
    assert_eq!(fresh.editor.len(), 1);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn starting_and_quitting_without_engaging_keeps_the_session() {
    let dir = temp_project("welcome-keep");
    let file = dir.join("src/main.rs");

    // Establish a session.
    let engaged = app_with_file(&file);
    engaged.save_session();
    let session_path = crate::session::session_path(engaged.workspace.root()).unwrap();
    let before = Session::load_from(&session_path).expect("a session");

    // A fresh start that quits untouched must not overwrite it.
    let untouched = App::new(Some(&file)).unwrap();
    untouched.save_session();
    assert_eq!(Session::load_from(&session_path), Some(before));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn home_command_returns_to_the_welcome_screen() {
    let dir = temp_project("welcome-home");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    assert!(!app.editor.is_empty());

    app.execute_command(ids::HOME);
    assert!(app.editor.is_empty());
    assert!(app.welcome_active());
    fs::remove_dir_all(&dir).ok();
}

/// Drive the new-project flow up to the language step with `name`.
fn new_project_to_language(app: &mut App, parent: &Path, name: &str) {
    app.execute_command(ids::NEW_PROJECT);
    if let Overlay::NewProject(flow) = &mut app.overlay {
        flow.browser.current = parent.to_path_buf();
        flow.browser.selected = 0;
        flow.browser.refresh();
    }
    // Choose the current folder.
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for c in name.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

#[test]
fn new_project_flow_creates_and_opens_a_project() {
    let dir = temp_project("new-project");
    let parent = dir.join("projects");
    fs::create_dir_all(&parent).unwrap();
    let mut app = App::new(Some(&dir)).unwrap();

    new_project_to_language(&mut app, &parent, "myapp");
    // Language step defaults to Rust; Enter starts creation.
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.pending_project);

    let deadline = Instant::now() + Duration::from_secs(15);
    while app.pending_project && Instant::now() < deadline {
        app.pump_background(Duration::from_millis(20));
    }
    // Detection runs on the background worker behind the git and tool
    // probes `open_workspace` queues, so wait for the result.
    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .editor
        .active_document()
        .is_some_and(|doc| doc.buffer.language != LanguageId::Rust)
        && Instant::now() < deadline
    {
        app.pump_background(Duration::from_millis(20));
    }

    let root = parent.join("myapp");
    assert!(root.join("Cargo.toml").is_file(), "project not created");
    assert_eq!(
        app.workspace.root().canonicalize().unwrap(),
        root.canonicalize().unwrap()
    );
    assert_eq!(app.editor.len(), 1, "the entry file should be open");
    assert_eq!(
        app.editor.active_document().unwrap().buffer.language,
        LanguageId::Rust
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_project_rejects_invalid_and_existing_names() {
    let dir = temp_project("new-project-errors");
    let parent = dir.join("projects");
    fs::create_dir_all(parent.join("taken")).unwrap();
    let mut app = App::new(Some(&dir)).unwrap();

    // A traversal-style name stays on the name step with an error.
    new_project_to_language(&mut app, &parent, "..");
    assert!(matches!(
        &app.overlay,
        Overlay::NewProject(flow) if flow.step == NewProjectStep::Name && flow.error.is_some()
    ));

    // An existing directory is refused too.
    if let Overlay::NewProject(flow) = &mut app.overlay {
        flow.name.clear();
    }
    for c in "taken".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        &app.overlay,
        Overlay::NewProject(flow) if flow.error.as_deref().is_some_and(|e| e.contains("exists"))
    ));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_project_can_be_cancelled_and_stepped_back() {
    let dir = temp_project("new-project-cancel");
    let parent = dir.join("projects");
    fs::create_dir_all(&parent).unwrap();
    let mut app = App::new(Some(&dir)).unwrap();

    new_project_to_language(&mut app, &parent, "cancelme");
    // On the language step, Esc goes back to the name step.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(
        &app.overlay,
        Overlay::NewProject(flow) if flow.step == NewProjectStep::Name
    ));
    // Then back to the parent step, then cancel.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(
        &app.overlay,
        Overlay::NewProject(flow) if flow.step == NewProjectStep::Parent
    ));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none());

    // Cancelling left nothing behind.
    assert!(!parent.join("cancelme").exists());
    assert!(!app.pending_project);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn opening_a_project_switches_the_workspace() {
    let dir = temp_project("switch");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    // A second project with its own Cargo.toml and source.
    let other = dir.join("other");
    fs::create_dir_all(other.join("src")).unwrap();
    fs::write(other.join("Cargo.toml"), "[package]\nname = \"other\"\n").unwrap();
    fs::write(other.join("src/lib.rs"), "pub fn other() {}\n").unwrap();

    assert!(app.open_workspace(other.clone()));
    app.pump_background(Duration::from_millis(300));
    assert_eq!(
        app.workspace.root().canonicalize().unwrap(),
        other.canonicalize().unwrap()
    );
    assert!(
        app.editor.is_empty(),
        "the editor resets for the new project"
    );
    assert!(app.welcome_active());
    fs::remove_dir_all(&dir).ok();
}

// ---- Project setup -------------------------------------------------------

use crate::language::setup::{LanguageSetup, ProjectSetupPlan, ProjectSetupRun, SetupState};

fn ready_plan(language: LanguageId) -> ProjectSetupPlan {
    ProjectSetupPlan {
        languages: vec![LanguageSetup {
            language,
            state: SetupState::Ready,
            detail: "built-in support — no server needed".to_string(),
        }],
    }
}

#[test]
fn project_setup_is_skipped_when_every_language_is_ready() {
    let dir = temp_project("setup-ready");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.tools = Some(ToolRegistry::discover_cached());
    // Markdown has no server: nothing to install, nothing to flag.
    app.project_languages = Some(vec![LanguageId::Markdown]);
    app.project_setup_offered = false;
    app.overlay = Overlay::None;

    assert!(!app.maybe_offer_project_setup());
    assert!(app.project_setup_offered, "the offer must be recorded once");
    assert!(app.overlay.is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn project_setup_reports_when_nothing_is_detected() {
    let dir = temp_project("setup-empty");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.tools = Some(ToolRegistry::discover_cached());
    app.project_languages = Some(Vec::new());
    app.overlay = Overlay::None;

    app.open_project_setup();
    assert!(app.overlay.is_none());
    assert!(
        app.status_message()
            .is_some_and(|message| message.contains("No project languages"))
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn starting_setup_with_nothing_missing_reports_already_set_up() {
    let dir = temp_project("setup-nothing");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.tools = Some(ToolRegistry::discover_cached());
    app.project_languages = Some(vec![LanguageId::Markdown]);

    app.start_project_setup();
    assert!(app.project_setup.is_none());
    assert!(
        app.status_message()
            .is_some_and(|message| message.contains("already set up"))
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn finishing_setup_reports_installed_and_failed_tools() {
    let dir = temp_project("setup-finish");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.project_setup = Some(ProjectSetupRun {
        plan: ready_plan(LanguageId::Markdown),
        queue: std::collections::VecDeque::new(),
        installed: vec![Tool::LuaLs],
        failed: Vec::new(),
    });
    app.advance_project_setup();
    assert!(app.project_setup.is_none(), "the run is finished");
    assert!(
        app.status_message()
            .is_some_and(|message| message.contains("1 installed")),
        "got {:?}",
        app.status_message()
    );

    // A failed tool is reported honestly rather than as success.
    app.project_setup = Some(ProjectSetupRun {
        plan: ready_plan(LanguageId::Markdown),
        queue: std::collections::VecDeque::new(),
        installed: Vec::new(),
        failed: vec![(Tool::LuaLs, "download failed".to_string())],
    });
    app.advance_project_setup();
    assert!(
        app.status_message()
            .is_some_and(|message| message.contains("1 failed")),
        "got {:?}",
        app.status_message()
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_single_language_offer_is_suppressed_for_project_languages() {
    let dir = temp_project("setup-suppress");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    app.tools = Some(ToolRegistry::discover_cached());
    app.project_languages = Some(vec![LanguageId::Rust]);
    app.overlay = Overlay::None;
    app.lsp.clear();

    // The project flow owns Rust, so the per-file offer stays quiet.
    assert!(!app.maybe_offer_tool_setup());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_setup_overlay_lists_languages_and_offers_set_up() {
    let dir = temp_project("setup-overlay");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.show_project_setup(ProjectSetupPlan {
        languages: vec![
            LanguageSetup {
                language: LanguageId::Rust,
                state: SetupState::Ready,
                detail: "rust-analyzer is available".to_string(),
            },
            LanguageSetup {
                language: LanguageId::Go,
                state: SetupState::NeedsInstall(Tool::Gopls),
                detail: "Koda installs gopls".to_string(),
            },
            LanguageSetup {
                language: LanguageId::Php,
                state: SetupState::Prerequisite("php".to_string()),
                detail: "phpactor needs a PHP runtime".to_string(),
            },
        ],
    });

    let picker = match &app.overlay {
        Overlay::Picker(picker) => picker,
        _ => panic!("expected a setup picker"),
    };
    assert_eq!(picker.title, "Project setup");
    let mut labels = Vec::new();
    let mut has_set_up = false;
    for index in 0..picker.filtered.len() {
        if let Some(item) = picker.item(index) {
            labels.push(item.label.clone());
            if matches!(item.action, PickerAction::ProjectSetup) {
                has_set_up = true;
            }
        }
    }
    assert!(
        labels.iter().any(|label| label.contains("Rust")),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label.contains("Go · install")),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label.contains("PHP · needs php")),
        "{labels:?}"
    );
    assert!(has_set_up, "a missing tool must offer Set up project");
    fs::remove_dir_all(&dir).ok();
}

/// Move the Settings selection to `row` directly, for deterministic setup.
fn select_setting(app: &mut App, row: SettingsRow) {
    match &mut app.overlay {
        Overlay::Settings(state) => {
            state.selected = SETTINGS_ROWS
                .iter()
                .position(|candidate| *candidate == row)
                .expect("row is in the settings list");
        }
        _ => panic!("the settings screen is not open"),
    }
}

#[test]
fn settings_screen_opens_navigates_and_closes() {
    let dir = temp_project("settings-open");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::SETTINGS);
    let Overlay::Settings(state) = &app.overlay else {
        panic!("settings did not open");
    };
    assert_eq!(
        state.selected_row(),
        SettingsRow::Theme,
        "first actionable row"
    );

    // Down skips the section header; up skips it back.
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let Overlay::Settings(state) = &app.overlay else {
        panic!("settings did not stay open");
    };
    assert_eq!(
        state.selected_row(),
        SettingsRow::SoftWrap,
        "skips the Editor header"
    );
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let Overlay::Settings(state) = &app.overlay else {
        panic!("settings did not stay open");
    };
    assert_eq!(state.selected_row(), SettingsRow::Animations);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.overlay.is_none(), "Esc leaves the settings screen");

    crate::ui::theme::set_theme(ThemeId::Mellow);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn changing_the_theme_applies_immediately() {
    let dir = temp_project("settings-theme");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);
    assert_eq!(app.settings.theme, ThemeId::Mellow, "Mellow is the default");

    app.execute_command(ids::SETTINGS);
    select_setting(&mut app, SettingsRow::Theme);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.settings.theme, ThemeId::Midnight);
    assert_eq!(
        crate::ui::theme::theme_id(),
        ThemeId::Midnight,
        "the theme is applied live"
    );
    assert!(app.settings_dirty, "a change is marked for persistence");

    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.settings.theme, ThemeId::Mellow);

    crate::ui::theme::set_theme(ThemeId::Mellow);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn boolean_and_editor_settings_apply_live() {
    let dir = temp_project("settings-editor");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::SETTINGS);
    select_setting(&mut app, SettingsRow::LineNumbers);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.settings.line_numbers);
    assert!(!app.show_line_numbers, "line numbers hide immediately");

    select_setting(&mut app, SettingsRow::SoftWrap);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert!(app.settings.soft_wrap);
    assert!(app.wrap, "soft wrap turns on immediately");

    select_setting(&mut app, SettingsRow::AutoCompletion);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.settings.auto_completion);
    app.schedule_auto_completion();
    assert!(
        app.completion_due.is_none(),
        "automatic completion is suppressed"
    );

    select_setting(&mut app, SettingsRow::Animations);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.settings.motion);
    assert!(!app.motion, "animations stop immediately");

    select_setting(&mut app, SettingsRow::InlineDiagnostics);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.settings.inline_diagnostics);
    assert!(
        !app.inline_diagnostics,
        "inline diagnostics hide immediately"
    );

    crate::ui::theme::set_theme(ThemeId::Mellow);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn indentation_preference_applies_to_open_documents() {
    let dir = temp_project("settings-indent");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::SETTINGS);
    // Cycle Indent width: Auto -> 2 -> 4 -> 8 -> Auto. Pin 2.
    select_setting(&mut app, SettingsRow::IndentWidth);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.settings.indent_width, Some(2));
    assert_eq!(app.editor.active_document().unwrap().indent_width(), 2);

    // With spaces off, indentation inserts a tab.
    select_setting(&mut app, SettingsRow::UseSpaces);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.settings.use_spaces);
    let doc = app.editor.active_document_mut().unwrap();
    doc.move_to(Position::new(0, 0));
    doc.indent();
    assert!(doc.buffer.line_text(0).starts_with('\t'));

    crate::ui::theme::set_theme(ThemeId::Mellow);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn reset_restores_defaults() {
    let dir = temp_project("settings-reset");
    let file = dir.join("src/main.rs");
    let mut app = app_with_file(&file);

    app.execute_command(ids::SETTINGS);
    select_setting(&mut app, SettingsRow::Theme);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    select_setting(&mut app, SettingsRow::LineNumbers);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(app.settings, Settings::default());

    select_setting(&mut app, SettingsRow::Reset);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.settings, Settings::default());
    assert_eq!(crate::ui::theme::theme_id(), ThemeId::Mellow);
    assert!(app.show_line_numbers);
    assert!(!app.wrap);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_with_settings_applies_preferences() {
    let dir = temp_project("settings-load");
    let file = dir.join("src/main.rs");
    let settings = Settings {
        theme: ThemeId::Midnight,
        line_numbers: false,
        soft_wrap: true,
        indent_width: Some(2),
        use_spaces: false,
        auto_completion: false,
        motion: false,
        inline_diagnostics: false,
    };
    let mut app = App::new_with_settings(Some(&dir), settings).unwrap();
    assert_eq!(crate::ui::theme::theme_id(), ThemeId::Midnight);
    assert!(!app.show_line_numbers);
    assert!(app.wrap);
    assert!(!app.motion);
    assert!(!app.inline_diagnostics);

    app.open_path(file);
    assert_eq!(app.editor.active_document().unwrap().indent_width(), 2);

    crate::ui::theme::set_theme(ThemeId::Mellow);
    fs::remove_dir_all(&dir).ok();
}
