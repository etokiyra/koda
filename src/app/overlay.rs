//! Transient UI state: pickers, prompts and the search bar.

use std::path::{Path, PathBuf};

use crate::editor::Position;
use crate::filesystem;
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
    /// Open a file and reveal an LSP-encoded position, converting it to a Koda
    /// character position using the target line's text.
    RevealLsp {
        path: PathBuf,
        position: Position,
    },
    /// Show a short informational message in the statusline.
    Info(String),
    /// Install an external tool through its trusted package manager.
    InstallTool(crate::language::tools::Tool),
    /// Open the update/remove choices for a Koda-managed tool.
    ToolActions(crate::language::tools::Tool),
    /// Reinstall a Koda-managed tool at Koda's pinned version.
    UpdateTool(crate::language::tools::Tool),
    /// Remove a Koda-managed tool's files (never a user/system install).
    RemoveTool(crate::language::tools::Tool),
    /// Apply the code action at an index in the last response.
    ApplyCodeAction(usize),
    /// Delete a file or directory after confirmation.
    DeletePath(PathBuf),
    /// Show the unified diff for a path (staged or working tree).
    ShowDiff {
        path: PathBuf,
        staged: bool,
    },
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
    ProjectSearch,
    CommitMessage,
    CopyFile,
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
    /// A directory browser, used to open a project.
    DirPicker(DirPicker),
    /// The guided "create a new project" flow.
    NewProject(NewProject),
    /// A read-only unified diff.
    Diff(DiffState),
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }
}

/// The role of a line inside a unified diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    /// A `diff`/`index`/`---`/`+++` header line.
    Header,
    /// An `@@ … @@` hunk marker.
    Hunk,
    /// An added line.
    Add,
    /// A removed line.
    Remove,
    /// Unchanged context.
    Context,
}

/// One line of a rendered diff.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// A read-only unified diff, scrolled with the arrows.
pub struct DiffState {
    pub title: String,
    pub lines: Vec<DiffLine>,
    pub scroll: usize,
}

impl DiffState {
    /// Build state from the output of `git diff`, classifying each line.
    pub fn from_unified(title: impl Into<String>, text: &str) -> Self {
        DiffState {
            title: title.into(),
            lines: text.lines().map(classify_diff_line).collect(),
            scroll: 0,
        }
    }

    pub fn scroll_by(&mut self, delta: i64) {
        self.scroll = (self.scroll as i64 + delta).max(0) as usize;
    }
}

fn classify_diff_line(line: &str) -> DiffLine {
    let kind = if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with("+++")
        || line.starts_with("---")
        || line.starts_with("diff ")
        || line.starts_with("index ")
    {
        DiffLineKind::Header
    } else if line.starts_with('+') {
        DiffLineKind::Add
    } else if line.starts_with('-') {
        DiffLineKind::Remove
    } else {
        DiffLineKind::Context
    };
    DiffLine {
        kind,
        text: line.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Directory browsing
// ---------------------------------------------------------------------------

/// What a row in the directory browser represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirEntryKind {
    /// Confirm the directory currently being browsed.
    ChooseCurrent,
    /// Move to the parent directory.
    Parent,
    /// Descend into a subdirectory.
    Directory,
}

/// A row in the directory browser.
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: DirEntryKind,
}

/// A keyboard directory browser: navigate with the arrows, descend with Enter,
/// and confirm the current folder with the `ChooseCurrent` row.
pub struct DirBrowser {
    pub current: PathBuf,
    pub entries: Vec<DirEntry>,
    pub selected: usize,
    /// A message shown when the directory could not be read.
    pub error: Option<String>,
}

impl DirBrowser {
    pub fn new(start: &Path) -> Self {
        let mut browser = DirBrowser {
            current: start.to_path_buf(),
            entries: Vec::new(),
            selected: 0,
            error: None,
        };
        browser.refresh();
        browser
    }

    /// Re-read the current directory's subdirectories.
    pub fn refresh(&mut self) {
        let mut entries = vec![DirEntry {
            name: "use this folder".to_string(),
            path: self.current.clone(),
            kind: DirEntryKind::ChooseCurrent,
        }];
        if let Some(parent) = self.current.parent() {
            entries.push(DirEntry {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                kind: DirEntryKind::Parent,
            });
        }
        match filesystem::read_dir_sorted(&self.current) {
            Ok(list) => {
                self.error = None;
                for entry in list.into_iter().filter(|entry| entry.is_dir) {
                    entries.push(DirEntry {
                        name: entry.name,
                        path: entry.path,
                        kind: DirEntryKind::Directory,
                    });
                }
            }
            Err(err) => {
                self.error = Some(format!("cannot read this folder: {err}"));
            }
        }
        self.entries = entries;
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    pub fn selected(&self) -> Option<&DirEntry> {
        self.entries.get(self.selected)
    }

    /// Activate the highlighted row. Returns the chosen directory when the user
    /// confirmed the current folder; navigating returns `None`.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let entry = self.selected()?.clone();
        match entry.kind {
            DirEntryKind::ChooseCurrent => Some(self.current.clone()),
            DirEntryKind::Parent | DirEntryKind::Directory => {
                self.current = entry.path;
                self.selected = 0;
                self.refresh();
                None
            }
        }
    }
}

/// A directory picker overlay, used by "Open Project…".
pub struct DirPicker {
    pub browser: DirBrowser,
}

impl DirPicker {
    pub fn new(start: &Path) -> Self {
        DirPicker {
            browser: DirBrowser::new(start),
        }
    }
}

// ---------------------------------------------------------------------------
// New project flow
// ---------------------------------------------------------------------------

/// Which step of the new-project flow is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewProjectStep {
    Parent,
    Name,
    Language,
}

/// State for the guided "create a new project" flow.
pub struct NewProject {
    pub step: NewProjectStep,
    pub browser: DirBrowser,
    /// The chosen parent directory (updated when step one completes).
    pub parent: PathBuf,
    pub name: String,
    pub error: Option<String>,
    /// Index into [`crate::project::create::CREATABLE`].
    pub language: usize,
}

impl NewProject {
    pub fn new(start: &Path) -> Self {
        NewProject {
            step: NewProjectStep::Parent,
            browser: DirBrowser::new(start),
            parent: start.to_path_buf(),
            name: String::new(),
            error: None,
            language: 0,
        }
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
    /// Interpret the query as a regular expression.
    pub regex: bool,
    /// The last regex compilation error, shown in the bar.
    pub regex_error: Option<String>,
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
            regex: false,
            regex_error: None,
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
        self.regex_error = None;
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

/// The signature-help popup's state.
pub struct SignatureState {
    pub help: crate::language::lsp::convert::SignatureHelp,
    /// The screen anchor captured when the response arrived.
    pub anchor: Option<(u16, u16)>,
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
    ///
    /// Matching is a fuzzy subsequence so `mrs` still finds `main_result`, but
    /// the ordering keeps prefix and short matches first: `if` ranks above
    /// `impl`, and an exact prefix beats a scattered match.
    pub fn refilter(&mut self) {
        let lower = self.prefix.to_lowercase();
        if lower.is_empty() {
            self.items = self.pool.clone();
        } else {
            let mut scored: Vec<(i32, &Completion)> = self
                .pool
                .iter()
                .filter_map(|item| {
                    fuzzy_score(&lower, &item.label).map(|score| {
                        // A prefix match is a strong signal; bias it upward.
                        let bias = if item.label.to_lowercase().starts_with(&lower) {
                            -100
                        } else {
                            0
                        };
                        (score + bias, item)
                    })
                })
                .collect();
            scored.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.label.len().cmp(&b.1.label.len()))
                    .then_with(|| a.1.label.cmp(&b.1.label))
            });
            self.items = scored.into_iter().map(|(_, item)| item.clone()).collect();
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::completion::CompletionKind;

    #[test]
    fn classifies_unified_diff_lines() {
        let diff = DiffState::from_unified(
            "working tree · a.rs",
            "diff --git a/a.rs b/a.rs\n\
             index 111..222 100644\n\
             --- a/a.rs\n\
             +++ b/a.rs\n\
             @@ -1,2 +1,2 @@\n\
             - old\n\
             + new\n\
             \u{20}unchanged\n",
        );
        assert_eq!(diff.title, "working tree · a.rs");
        let kinds: Vec<_> = diff.lines.iter().map(|line| line.kind).collect();
        assert_eq!(kinds[0], DiffLineKind::Header);
        assert_eq!(kinds[4], DiffLineKind::Hunk);
        assert_eq!(kinds[5], DiffLineKind::Remove);
        assert_eq!(kinds[6], DiffLineKind::Add);
        assert_eq!(kinds[7], DiffLineKind::Context);
        assert!(diff.lines[6].text.starts_with("+ new"));
    }

    #[test]
    fn completion_matches_fuzzy_subsequences() {
        let pool = vec![
            Completion::new("main_result", CompletionKind::Variable),
            Completion::new("map", CompletionKind::Variable),
            Completion::new("maximum", CompletionKind::Variable),
        ];
        let state = CompletionState::new(pool, "mrs".to_string());
        let labels: Vec<&str> = state.items.iter().map(|item| item.label.as_str()).collect();
        assert!(labels.contains(&"main_result"), "labels: {labels:?}");
        assert!(!labels.contains(&"map"), "labels: {labels:?}");
    }

    #[test]
    fn completion_ranks_prefix_matches_first() {
        let pool = vec![
            Completion::new("account", CompletionKind::Variable),
            Completion::new("counter", CompletionKind::Variable),
            Completion::new("count", CompletionKind::Variable),
        ];
        let state = CompletionState::new(pool, "count".to_string());
        let labels: Vec<&str> = state.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels.first(), Some(&"count"), "labels: {labels:?}");
        assert!(labels.contains(&"counter"));
        assert!(labels.contains(&"account"));
    }
}
