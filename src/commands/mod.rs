//! The extensible command system.
//!
//! Commands are the single source of truth for what Koda can do. They power the
//! command palette and give the keymap something stable to target. Actions are
//! dispatched by id in the app layer, which keeps commands free of UI details.

use crate::language::Capability;

/// Stable command identifiers.
pub mod ids {
    pub const SAVE: &str = "file.save";
    pub const SAVE_ALL: &str = "file.saveAll";
    pub const OPEN: &str = "file.open";
    pub const QUICK_OPEN: &str = "file.quickOpen";
    pub const CLOSE_TAB: &str = "file.closeTab";
    pub const CLOSE_ALL: &str = "file.closeAll";
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
    pub const INDENT: &str = "edit.indent";
    pub const OUTDENT: &str = "edit.outdent";
    pub const MOVE_LINE_UP: &str = "edit.moveLineUp";
    pub const MOVE_LINE_DOWN: &str = "edit.moveLineDown";
    pub const DUPLICATE_LINE: &str = "edit.duplicateLine";

    pub const FORMAT: &str = "language.format";
    pub const GOTO_DEFINITION: &str = "language.gotoDefinition";
    pub const FIND_REFERENCES: &str = "language.findReferences";
    pub const SHOW_SYMBOLS: &str = "language.symbols";
    pub const RENAME: &str = "language.rename";
    pub const CODE_ACTIONS: &str = "language.codeActions";

    pub const TOGGLE_TREE: &str = "view.toggleTree";
    pub const FOCUS_TREE: &str = "view.focusTree";
    pub const TOGGLE_HIDDEN: &str = "view.toggleHidden";
    pub const FILTER_TREE: &str = "view.filterTree";
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
            Command::new(OPEN, "Open File…", "File", Some("Ctrl+O"))
                .describes("Open a file by path"),
            Command::new(QUICK_OPEN, "Quick Open…", "File", Some("Ctrl+P"))
                .describes("Jump to a file in the project"),
            Command::new(CLOSE_TAB, "Close Tab", "File", Some("Ctrl+W"))
                .describes("Close the active tab")
                .needs_doc(),
            Command::new(CLOSE_ALL, "Close All Tabs", "File", None)
                .describes("Close every open tab"),
            Command::new(QUIT, "Quit", "File", Some("Ctrl+Q")).describes("Leave Koda"),
            Command::new(UNDO, "Undo", "Edit", Some("Ctrl+Z"))
                .describes("Undo the last change")
                .needs_doc(),
            Command::new(REDO, "Redo", "Edit", Some("Ctrl+Shift+Z"))
                .describes("Redo the last undone change")
                .needs_doc(),
            Command::new(SELECT_ALL, "Select All", "Edit", Some("Ctrl+A"))
                .describes("Select the whole file")
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
            Command::new(FIND, "Find", "Edit", Some("Ctrl+F"))
                .describes("Search the active file")
                .needs_doc(),
            Command::new(REPLACE, "Replace", "Edit", Some("Ctrl+H"))
                .describes("Search and replace in the active file")
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
            Command::new(FORMAT, "Format Document", "Language", None)
                .describes("Format the active file with its language provider")
                .needs_doc()
                .capability(Capability::Formatting),
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
            Command::new(TOGGLE_TREE, "Toggle File Tree", "View", Some("Ctrl+B"))
                .describes("Show or hide the project sidebar"),
            Command::new(
                FOCUS_TREE,
                "Focus File Tree / Editor",
                "View",
                Some("Ctrl+E"),
            )
            .describes("Move keyboard focus between the tree and the editor"),
            Command::new(TOGGLE_HIDDEN, "Toggle Hidden Files", "View", None)
                .describes("Show or hide dotfiles and ignored directories"),
            Command::new(FILTER_TREE, "Filter File Tree", "View", Some("/"))
                .describes("Fuzzy-filter the project files from the sidebar"),
            Command::new(NEXT_TAB, "Next Tab", "View", Some("Ctrl+Tab"))
                .describes("Switch to the next open tab"),
            Command::new(PREV_TAB, "Previous Tab", "View", Some("Ctrl+Shift+Tab"))
                .describes("Switch to the previous open tab"),
            Command::new(PALETTE, "Command Palette", "View", Some("Ctrl+Shift+P"))
                .describes("Search every command Koda offers"),
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
