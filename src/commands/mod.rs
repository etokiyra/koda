//! The extensible command system.
//!
//! Commands are the single source of truth for what Koda can do. They power the
//! command palette and give the keymap something stable to target. Actions are
//! dispatched by id in the app layer, which keeps commands free of UI details.

/// Stable command identifiers.
pub mod ids {
    pub const SAVE: &str = "file.save";
    pub const OPEN: &str = "file.open";
    pub const QUICK_OPEN: &str = "file.quickOpen";
    pub const CLOSE_TAB: &str = "file.closeTab";
    pub const QUIT: &str = "app.quit";

    pub const UNDO: &str = "edit.undo";
    pub const REDO: &str = "edit.redo";
    pub const SELECT_ALL: &str = "edit.selectAll";
    pub const COPY: &str = "edit.copy";
    pub const CUT: &str = "edit.cut";
    pub const PASTE: &str = "edit.paste";
    pub const FIND: &str = "edit.find";
    pub const REPLACE: &str = "edit.replace";
    pub const GOTO_LINE: &str = "edit.gotoLine";
    pub const TOGGLE_COMMENT: &str = "edit.toggleComment";

    pub const FORMAT: &str = "language.format";
    pub const GOTO_DEFINITION: &str = "language.gotoDefinition";
    pub const FIND_REFERENCES: &str = "language.findReferences";
    pub const SHOW_SYMBOLS: &str = "language.symbols";
    pub const RENAME: &str = "language.rename";
    pub const CODE_ACTIONS: &str = "language.codeActions";

    pub const TOGGLE_TREE: &str = "view.toggleTree";
    pub const FOCUS_TREE: &str = "view.focusTree";
    pub const NEXT_TAB: &str = "view.nextTab";
    pub const PREV_TAB: &str = "view.previousTab";
    pub const PALETTE: &str = "view.commandPalette";
}

/// A single command definition.
#[derive(Clone, Debug)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub shortcut: Option<&'static str>,
}

impl Command {
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
        let c = |id, title, category, shortcut| Command {
            id,
            title,
            category,
            shortcut,
        };
        let commands = vec![
            c(ids::SAVE, "Save", "File", Some("Ctrl+S")),
            c(ids::OPEN, "Open File…", "File", Some("Ctrl+O")),
            c(ids::QUICK_OPEN, "Quick Open…", "File", Some("Ctrl+P")),
            c(ids::CLOSE_TAB, "Close Tab", "File", Some("Ctrl+W")),
            c(ids::QUIT, "Quit", "File", Some("Ctrl+Q")),
            c(ids::UNDO, "Undo", "Edit", Some("Ctrl+Z")),
            c(ids::REDO, "Redo", "Edit", Some("Ctrl+Shift+Z")),
            c(ids::SELECT_ALL, "Select All", "Edit", Some("Ctrl+A")),
            c(ids::COPY, "Copy", "Edit", Some("Ctrl+C")),
            c(ids::CUT, "Cut", "Edit", Some("Ctrl+X")),
            c(ids::PASTE, "Paste", "Edit", Some("Ctrl+V")),
            c(ids::FIND, "Find", "Edit", Some("Ctrl+F")),
            c(ids::REPLACE, "Replace", "Edit", Some("Ctrl+H")),
            c(ids::GOTO_LINE, "Go to Line…", "Edit", Some("Ctrl+G")),
            c(
                ids::TOGGLE_COMMENT,
                "Toggle Comment",
                "Edit",
                Some("Ctrl+/"),
            ),
            c(ids::FORMAT, "Format Document", "Language", None),
            c(
                ids::GOTO_DEFINITION,
                "Go to Definition",
                "Language",
                Some("F12"),
            ),
            c(
                ids::FIND_REFERENCES,
                "Find References",
                "Language",
                Some("Shift+F12"),
            ),
            c(
                ids::SHOW_SYMBOLS,
                "Go to Symbol…",
                "Language",
                Some("Ctrl+Shift+O"),
            ),
            c(ids::RENAME, "Rename Symbol", "Language", Some("F2")),
            c(
                ids::CODE_ACTIONS,
                "Code Actions",
                "Language",
                Some("Ctrl+."),
            ),
            c(ids::TOGGLE_TREE, "Toggle File Tree", "View", Some("Ctrl+B")),
            c(
                ids::FOCUS_TREE,
                "Focus File Tree / Editor",
                "View",
                Some("Ctrl+E"),
            ),
            c(ids::NEXT_TAB, "Next Tab", "View", Some("Ctrl+Tab")),
            c(
                ids::PREV_TAB,
                "Previous Tab",
                "View",
                Some("Ctrl+Shift+Tab"),
            ),
            c(
                ids::PALETTE,
                "Command Palette",
                "View",
                Some("Ctrl+Shift+P"),
            ),
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
