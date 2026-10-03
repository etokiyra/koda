//! Transient UI state: pickers, prompts and the search bar.

use std::path::{Path, PathBuf};

use crate::editor::Position;
use crate::language::completion::Completion;

/// What happens when a picker item is chosen.
#[derive(Clone, Debug)]
pub enum PickerAction {
    Command(&'static str),
    OpenPath(PathBuf),
    /// Open a file and place the cursor at a position (diagnostics, symbols).
    Reveal {
        path: PathBuf,
        position: Position,
    },
    /// Show a short informational message in the statusline.
    Info(String),
    /// Install an external tool through its trusted package manager.
    InstallTool(crate::language::tools::Tool),
    /// Apply the code action at an index in the last response.
    ApplyCodeAction(usize),
    /// Delete a file or directory after confirmation.
    DeletePath(PathBuf),
}

/// A single row in a picker.
#[derive(Clone, Debug)]
pub struct PickerItem {
    pub label: String,
    pub detail: String,
    pub shortcut: String,
    pub action: PickerAction,
    /// Whether the action can run right now.
    pub enabled: bool,
    /// Why the action is unavailable, when it is.
    pub hint: Option<String>,
}

impl PickerItem {
    pub fn new(label: impl Into<String>, detail: impl Into<String>, action: PickerAction) -> Self {
        PickerItem {
            label: label.into(),
            detail: detail.into(),
            shortcut: String::new(),
            action,
            enabled: true,
            hint: None,
        }
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    pub fn disabled(mut self, hint: impl Into<String>) -> Self {
        self.enabled = false;
        self.hint = Some(hint.into());
        self
    }
}

/// A filterable list overlay, used for the command palette and quick open.
pub struct Picker {
    pub title: String,
    pub placeholder: String,
    items: Vec<PickerItem>,
    pub filtered: Vec<usize>,
    pub query: String,
    pub selected: usize,
}

impl Picker {
    pub fn new(
        title: impl Into<String>,
        placeholder: impl Into<String>,
        items: Vec<PickerItem>,
    ) -> Self {
        let filtered = (0..items.len()).collect();
        Picker {
            title: title.into(),
            placeholder: placeholder.into(),
            items,
            filtered,
            query: String::new(),
            selected: 0,
        }
    }

    pub fn refilter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.items.len()).collect();
        } else {
            let mut scored: Vec<(i32, usize)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    let haystack = format!("{} {}", item.label, item.detail);
                    fuzzy_score(&self.query, &haystack).map(|score| (score, i))
                })
                .collect();
            scored.sort_by_key(|(score, i)| (*score, *i));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.selected = 0;
    }

    pub fn item(&self, filtered_index: usize) -> Option<&PickerItem> {
        let idx = *self.filtered.get(filtered_index)?;
        self.items.get(idx)
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.item(self.selected)
    }

    /// Append items that are not already present, preserving the selection.
    ///
    /// Used to merge language-server workspace symbols into a picker that was
    /// already populated by Koda's built-in scan.
    pub fn extend_items(&mut self, items: Vec<PickerItem>) {
        let selected = self
            .selected_item()
            .map(|item| (item.label.clone(), item.detail.clone()));
        let mut seen: std::collections::HashSet<(String, String)> = self
            .items
            .iter()
            .map(|item| (item.label.clone(), item.detail.clone()))
            .collect();
        for item in items {
            if seen.insert((item.label.clone(), item.detail.clone())) {
                self.items.push(item);
            }
        }
        self.refilter();
        if let Some((label, detail)) = selected
            && let Some(index) = self
                .filtered
                .iter()
                .position(|&i| self.items[i].label == label && self.items[i].detail == detail)
        {
            self.selected = index;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }
}

/// The kind of text prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    GotoLine,
    OpenPath,
    SaveAs,
    Rename,
    NewFile,
    RenameFile,
}

/// A single-line text prompt.
pub struct Prompt {
    pub kind: PromptKind,
    pub label: String,
    pub placeholder: String,
    pub input: String,
}

impl Prompt {
    pub fn new(kind: PromptKind, label: impl Into<String>, placeholder: impl Into<String>) -> Self {
        Prompt {
            kind,
            label: label.into(),
            placeholder: placeholder.into(),
            input: String::new(),
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }
}

/// The currently visible overlay, if any.
pub enum Overlay {
    None,
    Picker(Picker),
    Prompt(Prompt),
    /// The keyboard-shortcuts cheatsheet.
    Help(Help),
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }
}

/// Scroll state for the help cheatsheet.
#[derive(Default)]
pub struct Help {
    pub scroll: usize,
}

/// Which search field has focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchField {
    Query,
    Replacement,
}

/// State for the find/replace bar.
pub struct Search {
    pub open: bool,
    pub replace_mode: bool,
    pub field: SearchField,
    pub query: String,
    pub replacement: String,
    /// Match case exactly rather than case-insensitively.
    pub case_sensitive: bool,
    /// Match whole words only.
    pub whole_word: bool,
    pub matches: Vec<(Position, Position)>,
    pub current: Option<usize>,
}

impl Default for Search {
    fn default() -> Self {
        Search {
            open: false,
            replace_mode: false,
            field: SearchField::Query,
            query: String::new(),
            replacement: String::new(),
            case_sensitive: false,
            whole_word: false,
            matches: Vec::new(),
            current: None,
        }
    }
}

impl Search {
    pub fn close(&mut self) {
        self.open = false;
        self.replace_mode = false;
        self.field = SearchField::Query;
        self.matches.clear();
        self.current = None;
    }
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

/// The hover popup's state.
pub struct HoverState {
    pub title: String,
    pub kind: Option<crate::language::symbols::SymbolKind>,
    pub body: Vec<String>,
}

/// The completion popup's state: a candidate pool filtered by the typed prefix.
pub struct CompletionState {
    pool: Vec<Completion>,
    pub prefix: String,
    pub items: Vec<Completion>,
    pub selected: usize,
}

impl CompletionState {
    pub fn new(pool: Vec<Completion>, prefix: String) -> Self {
        let mut state = CompletionState {
            pool,
            prefix,
            items: Vec::new(),
            selected: 0,
        };
        state.refilter();
        state
    }

    /// Recompute `items` from the pool for the current prefix.
    pub fn refilter(&mut self) {
        let lower = self.prefix.to_lowercase();
        self.items = self
            .pool
            .iter()
            .filter(|item| lower.is_empty() || item.label.to_lowercase().starts_with(&lower))
            .cloned()
            .collect();
        // Prefer short names, then alphabetical: `if` ranks above `impl`.
        self.items.sort_by(|a, b| {
            a.label
                .len()
                .cmp(&b.label.len())
                .then_with(|| a.label.cmp(&b.label))
        });
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// Merge additional candidates (e.g. from a language server) and re-filter.
    pub fn extend(&mut self, items: Vec<Completion>) {
        for item in items {
            if !self
                .pool
                .iter()
                .any(|existing| existing.label == item.label)
            {
                self.pool.push(item);
            }
        }
        self.refilter();
    }

    pub fn set_prefix(&mut self, prefix: String) {
        self.prefix = prefix;
        self.selected = 0;
        self.refilter();
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
    }

    pub fn selected_item(&self) -> Option<&Completion> {
        self.items.get(self.selected)
    }
}

/// A simple subsequence fuzzy matcher. Lower scores are better.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text_chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0i32;
    let mut cursor = 0usize;
    let mut last_match: Option<usize> = None;

    for q in query.to_lowercase().chars() {
        let mut found = None;
        let mut i = cursor;
        while i < text_chars.len() {
            if text_chars[i] == q {
                found = Some(i);
                break;
            }
            i += 1;
        }
        let i = found?;
        if let Some(last) = last_match
            && i == last + 1
        {
            score -= 5;
        }
        score += i as i32;
        last_match = Some(i);
        cursor = i + 1;
    }
    Some(score)
}

/// Inline file-tree filter state.
///
/// The project file list is collected once when the filter opens, then filtered
/// in memory on each keystroke, so typing stays fast even in large projects.
pub struct TreeFilter {
    pub root: PathBuf,
    all: Vec<PathBuf>,
    pub query: String,
    pub matches: Vec<PathBuf>,
    pub selected: usize,
}

impl TreeFilter {
    pub fn new(root: &Path) -> Self {
        let all = crate::filesystem::collect_files(root, 8000);
        let mut filter = TreeFilter {
            root: root.to_path_buf(),
            all,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
        };
        filter.refilter();
        filter
    }

    pub fn refilter(&mut self) {
        if self.query.is_empty() {
            self.matches = self.all.iter().take(500).cloned().collect();
        } else {
            let query = self.query.to_lowercase();
            let root = self.root.clone();
            let mut scored: Vec<(i32, PathBuf)> = self
                .all
                .iter()
                .filter_map(|path| {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // Score the path relative to the project root: the absolute
                    // prefix ("/home/you/project") would otherwise match almost
                    // any query.
                    let relative = path.strip_prefix(&root).unwrap_or(path);
                    let full = relative.to_string_lossy();
                    let name_score = fuzzy_score(&query, name);
                    let path_score = fuzzy_score(&query, &full).map(|score| score + 200);
                    match (name_score, path_score) {
                        (Some(a), Some(b)) => Some((a.min(b), path.clone())),
                        (Some(a), None) => Some((a, path.clone())),
                        (None, Some(b)) => Some((b, path.clone())),
                        (None, None) => None,
                    }
                })
                .collect();
            scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            self.matches = scored.into_iter().map(|(_, path)| path).collect();
        }
        self.selected = 0;
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.matches.get(self.selected).map(PathBuf::as_path)
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }
}
