//! Transient UI state: pickers, prompts and the search bar.

use std::path::PathBuf;

use crate::editor::Position;

/// What happens when a picker item is chosen.
#[derive(Clone, Debug)]
pub enum PickerAction {
    Command(&'static str),
    OpenPath(PathBuf),
}

/// A single row in a picker.
#[derive(Clone, Debug)]
pub struct PickerItem {
    pub label: String,
    pub detail: String,
    pub shortcut: String,
    pub action: PickerAction,
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
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }
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
