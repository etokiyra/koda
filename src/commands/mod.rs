//! The extensible command system.
//!
//! Commands are the single source of truth for what Koda can do. They power the
//! command palette and give the keymap something stable to target. Actions are
//! dispatched by id in the app layer, which keeps commands free of UI details.

use std::collections::HashMap;

use crate::language::Capability;

/// Stable command identifiers.
pub mod ids {
    pub const SAVE: &str = "file.save";
    pub const SAVE_ALL: &str = "file.saveAll";
    pub const SAVE_AS: &str = "file.saveAs";
    pub const OPEN: &str = "file.open";
    pub const QUICK_OPEN: &str = "file.quickOpen";
    pub const CLOSE_TAB: &str = "file.closeTab";
    pub const CLOSE_ALL: &str = "file.closeAll";
    pub const REVERT: &str = "file.revert";
    pub const NEW_FILE: &str = "file.new";
    pub const RENAME_FILE: &str = "file.rename";
    pub const DELETE_FILE: &str = "file.delete";
    pub const DUPLICATE_FILE: &str = "file.duplicate";
    pub const COPY_FILE: &str = "file.copy";
    pub const QUIT: &str = "app.quit";
    pub const HOME: &str = "app.home";

    pub const UNDO: &str = "edit.undo";
    pub const REDO: &str = "edit.redo";
    pub const SELECT_ALL: &str = "edit.selectAll";
    pub const SELECT_NEXT: &str = "edit.selectNext";
    pub const SELECT_ALL_OCCURRENCES: &str = "edit.selectAllOccurrences";
    pub const ADD_CURSOR_BELOW: &str = "edit.addCursorBelow";
    pub const ADD_CURSOR_ABOVE: &str = "edit.addCursorAbove";
    pub const COPY: &str = "edit.copy";
    pub const CUT: &str = "edit.cut";
    pub const PASTE: &str = "edit.paste";
    pub const YANK_POP: &str = "edit.yankPop";
    pub const COMPLETE: &str = "edit.complete";
    pub const FIND: &str = "edit.find";
    pub const REPLACE: &str = "edit.replace";
    pub const REPLACE_ALL: &str = "edit.replaceAll";
    pub const GOTO_LINE: &str = "edit.gotoLine";
    pub const TOGGLE_COMMENT: &str = "edit.toggleComment";
    pub const INDENT: &str = "edit.indent";
    pub const OUTDENT: &str = "edit.outdent";
    pub const MOVE_LINE_UP: &str = "edit.moveLineUp";
    pub const MOVE_LINE_DOWN: &str = "edit.moveLineDown";
    pub const DUPLICATE_LINE: &str = "edit.duplicateLine";
    pub const DELETE_LINE: &str = "edit.deleteLine";
    pub const MATCHING_BRACKET: &str = "edit.matchingBracket";

    pub const FORMAT: &str = "language.format";
    pub const HOVER: &str = "language.hover";
    pub const SETUP: &str = "language.setup";
    pub const RESTART_SERVER: &str = "language.restartServer";
    pub const GOTO_DEFINITION: &str = "language.gotoDefinition";
    pub const FIND_REFERENCES: &str = "language.findReferences";
    pub const SHOW_SYMBOLS: &str = "language.symbols";
    pub const RENAME: &str = "language.rename";
    pub const CODE_ACTIONS: &str = "language.codeActions";

    pub const DIAGNOSTICS_NEXT: &str = "diagnostics.next";
    pub const DIAGNOSTICS_PREV: &str = "diagnostics.previous";
    pub const DIAGNOSTICS_LIST: &str = "diagnostics.list";

    pub const WORKSPACE_SYMBOLS: &str = "project.symbols";
    pub const PROJECT_SEARCH: &str = "project.search";
    pub const OPEN_PROJECT: &str = "project.openDir";
    pub const NEW_PROJECT: &str = "project.new";
    pub const SETUP_PROJECT: &str = "project.setup";
    pub const CHANGED_FILES: &str = "git.changedFiles";
    pub const GIT_COMMIT: &str = "git.commit";
    pub const GIT_TOGGLE_STAGE: &str = "git.toggleStage";
    pub const DIFF: &str = "git.diff";

    pub const TOGGLE_TREE: &str = "view.toggleTree";
    pub const FOCUS_TREE: &str = "view.focusTree";
    pub const TOGGLE_HIDDEN: &str = "view.toggleHidden";
    pub const TOGGLE_INLINE_DIAGNOSTICS: &str = "view.toggleInlineDiagnostics";
    pub const TOGGLE_WRAP: &str = "view.toggleWrap";
    pub const REFRESH: &str = "view.refresh";
    pub const SPLIT: &str = "view.split";
    pub const FOCUS_PANE: &str = "view.focusPane";
    pub const FILTER_TREE: &str = "view.filterTree";
    pub const NEXT_TAB: &str = "view.nextTab";
    pub const PREV_TAB: &str = "view.previousTab";
    pub const PALETTE: &str = "view.commandPalette";
    pub const HELP: &str = "app.help";
    pub const SETTINGS: &str = "app.settings";
    pub const TOGGLE_MOTION: &str = "app.toggleMotion";
    pub const WELCOME_SCENE: &str = "app.welcomeScene";
}

/// A single command definition.
#[derive(Clone, Debug)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub shortcut: Option<&'static str>,
    /// One-line explanation shown in the palette.
    pub description: &'static str,
    /// Whether the command needs an open document.
    pub needs_doc: bool,
    /// Language capability the command depends on.
    pub capability: Option<Capability>,
}

impl Command {
    fn new(
        id: &'static str,
        title: &'static str,
        category: &'static str,
        shortcut: Option<&'static str>,
    ) -> Self {
        Command {
            id,
            title,
            category,
            shortcut,
            description: "",
            needs_doc: false,
            capability: None,
        }
    }

    fn describes(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    fn needs_doc(mut self) -> Self {
        self.needs_doc = true;
        self
    }

    fn capability(mut self, capability: Capability) -> Self {
        self.capability = Some(capability);
        self
    }

    /// The label shown in the command palette, e.g. `File: Save`.
    pub fn palette_label(&self) -> String {
        format!("{}: {}", self.category, self.title)
    }
}

/// The registry of everything Koda can currently do.
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    /// Commands built into Koda. A plugin/extensibility layer will append to this.
    pub fn builtin() -> Self {
        use ids::*;
        let commands = vec![
            Command::new(SAVE, "Save", "File", Some("Ctrl+S"))
                .describes("Write the active file to disk")
                .needs_doc(),
            Command::new(SAVE_ALL, "Save All", "File", None)
                .describes("Save every modified file")
                .needs_doc(),
            Command::new(SAVE_AS, "Save As…", "File", Some("Ctrl+Shift+S"))
                .describes("Write the active file to a new path")
                .needs_doc(),
            Command::new(OPEN, "Open File…", "File", Some("Ctrl+O"))
                .describes("Open a file by path"),
            Command::new(QUICK_OPEN, "Quick Open…", "File", Some("Ctrl+P"))
                .describes("Jump to a file in the project"),
            Command::new(CLOSE_TAB, "Close Tab", "File", Some("Ctrl+W"))
                .describes("Close the active tab")
                .needs_doc(),
            Command::new(CLOSE_ALL, "Close All Tabs", "File", None)
                .describes("Close every open tab"),
            Command::new(REVERT, "Revert File", "File", None)
                .describes("Discard changes and reload the file from disk")
                .needs_doc(),
            Command::new(NEW_FILE, "New File…", "File", Some("Ctrl+N"))
                .describes("Create a file in the selected folder"),
            Command::new(RENAME_FILE, "Rename…", "File", None)
                .describes("Rename the selected file or folder"),
            Command::new(DELETE_FILE, "Delete…", "File", None)
                .describes("Delete the selected file or folder"),
            Command::new(DUPLICATE_FILE, "Duplicate File", "File", None)
                .describes("Create a copy of the selected file and open it"),
            Command::new(COPY_FILE, "Copy File…", "File", None)
                .describes("Copy the selected file to a chosen path"),
            Command::new(QUIT, "Quit", "File", Some("Ctrl+Q")).describes("Leave Koda"),
            Command::new(HOME, "Welcome Screen", "File", None)
                .describes("Close all tabs and return to the home screen"),
            Command::new(WELCOME_SCENE, "Change Welcome Scene", "View", None)
                .describes("Cycle the animated scene on the welcome screen"),
            Command::new(TOGGLE_MOTION, "Toggle Animations", "View", None)
                .describes("Turn the welcome and busy animations on or off"),
            Command::new(UNDO, "Undo", "Edit", Some("Ctrl+Z"))
                .describes("Undo the last change")
                .needs_doc(),
            Command::new(REDO, "Redo", "Edit", Some("Ctrl+Shift+Z"))
                .describes("Redo the last undone change")
                .needs_doc(),
            Command::new(SELECT_ALL, "Select All", "Edit", Some("Ctrl+A"))
                .describes("Select the whole file")
                .needs_doc(),
            Command::new(
                SELECT_NEXT,
                "Select Next Occurrence",
                "Edit",
                Some("Ctrl+D"),
            )
            .describes("Add a cursor at the next occurrence of the selection")
            .needs_doc(),
            Command::new(
                SELECT_ALL_OCCURRENCES,
                "Select All Occurrences",
                "Edit",
                Some("Ctrl+Shift+L"),
            )
            .describes("Put a cursor on every occurrence of the selection")
            .needs_doc(),
            Command::new(
                ADD_CURSOR_BELOW,
                "Add Cursor Below",
                "Edit",
                Some("Ctrl+Alt+↓"),
            )
            .describes("Add a caret on the line below each cursor")
            .needs_doc(),
            Command::new(
                ADD_CURSOR_ABOVE,
                "Add Cursor Above",
                "Edit",
                Some("Ctrl+Alt+↑"),
            )
            .describes("Add a caret on the line above each cursor")
            .needs_doc(),
            Command::new(COPY, "Copy", "Edit", Some("Ctrl+C"))
                .describes("Copy the selection")
                .needs_doc(),
            Command::new(CUT, "Cut", "Edit", Some("Ctrl+X"))
                .describes("Cut the selection")
                .needs_doc(),
            Command::new(PASTE, "Paste", "Edit", Some("Ctrl+V"))
                .describes("Paste the clipboard")
                .needs_doc(),
            Command::new(YANK_POP, "Yank Pop", "Edit", Some("Alt+Y"))
                .describes("Replace the last paste with an earlier kill")
                .needs_doc(),
            Command::new(COMPLETE, "Complete", "Edit", Some("Ctrl+Space"))
                .describes("Suggest completions for the word being typed")
                .needs_doc(),
            Command::new(FIND, "Find", "Edit", Some("Ctrl+F"))
                .describes("Search the active file")
                .needs_doc(),
            Command::new(REPLACE, "Replace", "Edit", Some("Ctrl+H"))
                .describes("Search and replace in the active file")
                .needs_doc(),
            Command::new(REPLACE_ALL, "Replace All", "Edit", Some("Alt+Enter"))
                .describes("Replace every match in the active file")
                .needs_doc(),
            Command::new(GOTO_LINE, "Go to Line…", "Edit", Some("Ctrl+G"))
                .describes("Jump to a line number")
                .needs_doc(),
            Command::new(TOGGLE_COMMENT, "Toggle Comment", "Edit", Some("Ctrl+/"))
                .describes("Comment or uncomment the selected lines")
                .needs_doc(),
            Command::new(INDENT, "Indent", "Edit", Some("Tab"))
                .describes("Indent the selection or current line")
                .needs_doc(),
            Command::new(OUTDENT, "Outdent", "Edit", Some("Shift+Tab"))
                .describes("Outdent the selection or current line")
                .needs_doc(),
            Command::new(MOVE_LINE_UP, "Move Line Up", "Edit", Some("Alt+↑"))
                .describes("Move the current line or selection up")
                .needs_doc(),
            Command::new(MOVE_LINE_DOWN, "Move Line Down", "Edit", Some("Alt+↓"))
                .describes("Move the current line or selection down")
                .needs_doc(),
            Command::new(
                DUPLICATE_LINE,
                "Duplicate Line",
                "Edit",
                Some("Ctrl+Shift+D"),
            )
            .describes("Duplicate the current line")
            .needs_doc(),
            Command::new(DELETE_LINE, "Delete Line", "Edit", Some("Ctrl+Shift+K"))
                .describes("Delete the current line or selection")
                .needs_doc(),
            Command::new(
                MATCHING_BRACKET,
                "Go to Matching Bracket",
                "Edit",
                Some("Alt+M"),
            )
            .describes("Jump to the bracket matching the one under the cursor")
            .needs_doc(),
            Command::new(FORMAT, "Format Document", "Language", Some("Ctrl+Shift+I"))
                .describes("Format the active file with its language formatter")
                .needs_doc()
                .capability(Capability::Formatting),
            Command::new(HOVER, "Hover", "Language", Some("Ctrl+Shift+H"))
                .describes("Show information about the symbol under the cursor")
                .needs_doc()
                .capability(Capability::Hover),
            Command::new(SETUP, "Language Setup…", "Language", None)
                .describes("See which language tools Koda found"),
            Command::new(RESTART_SERVER, "Restart Language Server", "Language", None)
                .describes("Reconnect the language server for the active file")
                .needs_doc(),
            Command::new(GOTO_DEFINITION, "Go to Definition", "Language", Some("F12"))
                .describes("Jump to where the symbol is defined")
                .needs_doc()
                .capability(Capability::GotoDefinition),
            Command::new(
                FIND_REFERENCES,
                "Find References",
                "Language",
                Some("Shift+F12"),
            )
            .describes("Find every use of the symbol")
            .needs_doc()
            .capability(Capability::GotoReference),
            Command::new(
                SHOW_SYMBOLS,
                "Go to Symbol…",
                "Language",
                Some("Ctrl+Shift+O"),
            )
            .describes("Jump to a symbol in the file")
            .needs_doc()
            .capability(Capability::DocumentSymbols),
            Command::new(RENAME, "Rename Symbol", "Language", Some("F2"))
                .describes("Rename a symbol across the project")
                .needs_doc()
                .capability(Capability::Rename),
            Command::new(CODE_ACTIONS, "Code Actions", "Language", Some("Ctrl+."))
                .describes("Show quick fixes and refactors for the cursor")
                .needs_doc()
                .capability(Capability::CodeActions),
            Command::new(
                DIAGNOSTICS_NEXT,
                "Next Diagnostic",
                "Diagnostics",
                Some("F8"),
            )
            .describes("Jump to the next problem in the file")
            .needs_doc(),
            Command::new(
                DIAGNOSTICS_PREV,
                "Previous Diagnostic",
                "Diagnostics",
                Some("Shift+F8"),
            )
            .describes("Jump to the previous problem in the file")
            .needs_doc(),
            Command::new(
                DIAGNOSTICS_LIST,
                "Show Diagnostics",
                "Diagnostics",
                Some("Ctrl+Shift+M"),
            )
            .describes("List every problem in open files")
            .needs_doc(),
            Command::new(
                WORKSPACE_SYMBOLS,
                "Go to Symbol in Workspace…",
                "Project",
                Some("Ctrl+T"),
            )
            .describes("Search every definition in the project"),
            Command::new(
                PROJECT_SEARCH,
                "Search in Project…",
                "Project",
                Some("Ctrl+Shift+F"),
            )
            .describes("Find text across every file in the project"),
            Command::new(OPEN_PROJECT, "Open Project…", "Project", None)
                .describes("Open a folder as a project"),
            Command::new(NEW_PROJECT, "Create New Project…", "Project", None)
                .describes("Scaffold a new Rust, Go, Python or Shell project"),
            Command::new(SETUP_PROJECT, "Set Up Project…", "Project", None)
                .describes("Install the language tooling this project needs"),
            Command::new(CHANGED_FILES, "Changed Files…", "Git", Some("Ctrl+Shift+G"))
                .describes("List the files changed in the working tree"),
            Command::new(GIT_COMMIT, "Commit Changes…", "Git", None)
                .describes("Stage all changes and commit with a message"),
            Command::new(GIT_TOGGLE_STAGE, "Stage / Unstage File", "Git", None)
                .describes("Toggle the selected file in the git index"),
            Command::new(DIFF, "Diff File", "Git", Some("Alt+D"))
                .describes("Show the unified diff for the active file"),
            Command::new(
                TOGGLE_TREE,
                "File Panel: Focus / Hide",
                "View",
                Some("Ctrl+B"),
            )
            .describes("Focus the project sidebar, or hide it when focused"),
            Command::new(
                FOCUS_TREE,
                "Focus File Tree / Editor",
                "View",
                Some("Ctrl+E"),
            )
            .describes("Move keyboard focus between the tree and the editor"),
            Command::new(TOGGLE_HIDDEN, "Toggle Hidden Files", "View", None)
                .describes("Show or hide dotfiles and ignored directories"),
            Command::new(
                TOGGLE_INLINE_DIAGNOSTICS,
                "Toggle Inline Diagnostics",
                "View",
                Some("Alt+I"),
            )
            .describes("Show or hide diagnostic messages at the end of each line"),
            Command::new(TOGGLE_WRAP, "Toggle Soft Wrap", "View", Some("Alt+Z"))
                .describes("Wrap long lines to the width of the editor"),
            Command::new(REFRESH, "Refresh File Tree", "View", Some("F5"))
                .describes("Re-read the project tree and git status"),
            Command::new(SPLIT, "Split Editor", "View", Some("Alt+V"))
                .describes("Show two files side by side"),
            Command::new(FOCUS_PANE, "Focus Other Pane", "View", Some("Alt+O"))
                .describes("Move editing focus between the two panes")
                .needs_doc(),
            Command::new(FILTER_TREE, "Filter File Tree", "View", Some("/"))
                .describes("Fuzzy-filter the project files from the sidebar"),
            Command::new(NEXT_TAB, "Next Tab", "View", Some("Ctrl+Tab"))
                .describes("Switch to the next open tab"),
            Command::new(PREV_TAB, "Previous Tab", "View", Some("Ctrl+Shift+Tab"))
                .describes("Switch to the previous open tab"),
            Command::new(PALETTE, "Command Palette", "View", Some("Ctrl+Shift+P"))
                .describes("Search every command Koda offers"),
            Command::new(HELP, "Keyboard Shortcuts", "Help", Some("F1"))
                .describes("Show every shortcut at a glance"),
            Command::new(SETTINGS, "Settings", "General", None)
                .describes("Adjust Koda's appearance, editor and completion preferences"),
        ];
        CommandRegistry { commands }
    }

    pub fn all(&self) -> &[Command] {
        &self.commands
    }

    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
    }
}

/// Editor movement keys that are not commands.
///
/// Shared by the F1 cheatsheet and the README keymap so the two cannot drift.
pub const EDITOR_KEYS: &[(&str, &str)] = &[
    ("Arrows", "move the cursor"),
    ("Ctrl+←/→", "move by word"),
    ("Shift+Arrows", "select"),
    ("Home / End", "line start / end"),
    ("Ctrl+Home / End", "document start / end"),
    ("PageUp / PageDown", "scroll a page"),
    ("F3 / Shift+F3", "find next / previous"),
    ("Ctrl+PageUp/Down", "previous / next tab"),
];

/// Context-local keys that are not registry commands (the find bar, the file
/// tree and the welcome screen).
pub const CONTEXT_KEYS: &[(&str, &str)] = &[
    (
        "Alt+C / Alt+W / Alt+R",
        "find: toggle case / whole word / regex",
    ),
    ("Alt+Enter", "find: replace every match"),
    ("Space", "changed files: stage or unstage"),
    ("d", "changed files: show the diff"),
    ("/", "file tree: filter project files"),
    (".", "file tree: toggle hidden files"),
    ("↑ / ↓", "welcome: choose an action"),
    ("Enter", "welcome: open the chosen action"),
    ("v", "welcome: cycle the animated scene"),
];

/// Render the keymap as a Markdown table from the command registry plus the
/// editor and context keys.
///
/// `examples/keymap.rs` prints this to regenerate the README block, and
/// `tests/keymap.rs` fails when `README.md` drifts from it.
pub fn keymap_markdown() -> String {
    let registry = CommandRegistry::builtin();
    let mut out = String::from("| Category | Shortcut | Action |\n| --- | --- | --- |\n");

    let mut order: Vec<&'static str> = Vec::new();
    let mut groups: HashMap<&'static str, Vec<(&'static str, &'static str)>> = HashMap::new();
    for command in registry.all() {
        let Some(shortcut) = command.shortcut else {
            continue;
        };
        if !groups.contains_key(command.category) {
            order.push(command.category);
        }
        groups
            .entry(command.category)
            .or_default()
            .push((shortcut, command.title));
    }
    for category in order {
        for (shortcut, title) in &groups[category] {
            out.push_str(&format!("| {category} | `{shortcut}` | {title} |\n"));
        }
    }
    for (shortcut, title) in EDITOR_KEYS {
        out.push_str(&format!("| Editor | `{shortcut}` | {title} |\n"));
    }
    for (shortcut, title) in CONTEXT_KEYS {
        out.push_str(&format!("| Context | `{shortcut}` | {title} |\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn command_ids_and_shortcuts_are_unique() {
        let registry = CommandRegistry::builtin();
        let mut ids = HashSet::new();
        let mut shortcuts = HashSet::new();
        for command in registry.all() {
            assert!(
                ids.insert(command.id),
                "duplicate command id {}",
                command.id
            );
            if let Some(shortcut) = command.shortcut {
                assert!(
                    shortcuts.insert(shortcut),
                    "duplicate shortcut {shortcut} on {}",
                    command.id
                );
            }
        }
    }

    #[test]
    fn core_commands_are_registered() {
        let registry = CommandRegistry::builtin();
        for id in [
            ids::SAVE,
            ids::SAVE_ALL,
            ids::PALETTE,
            ids::COMPLETE,
            ids::GOTO_DEFINITION,
            ids::FORMAT,
            ids::DIFF,
            ids::SETUP,
        ] {
            assert!(registry.get(id).is_some(), "missing command {id}");
        }
    }
}
