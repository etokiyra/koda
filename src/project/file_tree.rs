//! A lazily-loaded file tree.
//!
//! Children are only read from disk when a directory is expanded. This keeps
//! opening a large project fast and predictable.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::filesystem::gitignore::Gitignore;
use crate::filesystem::{self, EntryInfo};

/// A row in the flattened, currently-visible tree.
#[derive(Clone, Debug)]
pub struct VisibleEntry {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

/// A lazy view of a project directory.
pub struct FileTree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Vec<EntryInfo>>,
    visible: Vec<VisibleEntry>,
    /// Ignore rules from the project's `.gitignore` files.
    ignore: Gitignore,
    /// Currently highlighted row.
    pub selected: usize,
    /// Whether dotfiles and ignored directories are shown.
    pub show_hidden: bool,
}

impl FileTree {
    /// Build a tree rooted at `root`, with the root expanded.
    pub fn new(root: &Path) -> Self {
        let mut tree = FileTree {
            root: root.to_path_buf(),
            expanded: HashSet::new(),
            cache: HashMap::new(),
            visible: Vec::new(),
            ignore: Gitignore::load(root),
            selected: 0,
            show_hidden: false,
        };
        tree.expanded.insert(root.to_path_buf());
        tree.rebuild();
        tree
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entries(&self) -> &[VisibleEntry] {
        &self.visible
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    pub fn selected_entry(&self) -> Option<&VisibleEntry> {
        self.visible.get(self.selected)
    }

    /// Toggle visibility of dotfiles and ignored directories.
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.cache.clear();
        self.rebuild();
    }

    /// Drop all cached directory listings, reload ignore rules and re-read the
    /// visible tree.
    pub fn refresh(&mut self) {
        self.cache.clear();
        self.ignore = Gitignore::load(&self.root);
        self.rebuild();
    }

    /// The directories currently expanded.
    pub fn expanded_paths(&self) -> Vec<PathBuf> {
        self.expanded.iter().cloned().collect()
    }

    /// Replace the expanded set and rebuild the visible tree. The root is
    /// always expanded; paths outside the project are ignored.
    pub fn set_expanded(&mut self, paths: &[PathBuf]) {
        self.expanded.clear();
        self.expanded.insert(self.root.clone());
        for path in paths {
            if path.starts_with(&self.root) && path.is_dir() {
                self.expanded.insert(path.clone());
            }
        }
        self.refresh();
    }

    pub fn select_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn select_down(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
        }
    }

    /// Select the row for `path` if it is visible.
    pub fn select_path(&mut self, path: &Path) {
        if let Some(idx) = self.visible.iter().position(|e| e.path == path) {
            self.selected = idx;
        }
    }

    /// Expand the selected directory, or return the file to open.
    pub fn activate_selected(&mut self) -> Option<PathBuf> {
        let entry = self.visible.get(self.selected)?.clone();
        if entry.is_dir {
            self.toggle_dir(&entry.path);
            None
        } else {
            Some(entry.path)
        }
    }

    /// Collapse the selected directory, or move to the parent directory.
    pub fn collapse_selected(&mut self) {
        let Some(entry) = self.visible.get(self.selected).cloned() else {
            return;
        };
        if entry.is_dir && entry.expanded {
            self.toggle_dir(&entry.path);
        } else if let Some(parent) = entry.path.parent()
            && parent.starts_with(&self.root)
        {
            self.select_path(parent);
        }
    }

    fn toggle_dir(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.visible.clear();
        let root = self.root.clone();
        self.visit(&root, 0);
        if self.selected >= self.visible.len() {
            self.selected = self.visible.len().saturating_sub(1);
        }
    }

    fn visit(&mut self, dir: &Path, depth: usize) {
        let children = self.children(dir).to_vec();
        for entry in children {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            self.visible.push(VisibleEntry {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
            });
            if expanded {
                self.visit(&entry.path, depth + 1);
            }
        }
    }

    fn children(&mut self, dir: &Path) -> &[EntryInfo] {
        if !self.cache.contains_key(dir) {
            let entries = filesystem::read_dir_sorted(dir)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| self.show_entry(e))
                .collect();
            self.cache.insert(dir.to_path_buf(), entries);
        }
        &self.cache[dir]
    }

    fn show_entry(&self, entry: &EntryInfo) -> bool {
        if self.show_hidden {
            return true;
        }
        if entry.name.starts_with('.') {
            return false;
        }
        if entry.is_dir && filesystem::is_ignored_dir(&entry.name) {
            return false;
        }
        !self.ignore.is_ignored(&entry.path, entry.is_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-tree-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("generated")).unwrap();
        fs::write(dir.join(".gitignore"), "generated/\n*.tmp\n").unwrap();
        fs::write(dir.join("src/main.rs"), "").unwrap();
        fs::write(dir.join("notes.tmp"), "").unwrap();
        dir
    }

    fn names(tree: &FileTree) -> Vec<String> {
        tree.entries().iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn hides_gitignored_entries() {
        let dir = temp_project("ignore");
        let tree = FileTree::new(&dir);
        let names = names(&tree);
        assert!(names.iter().any(|n| n == "src"));
        assert!(!names.iter().any(|n| n == "generated"));
        assert!(!names.iter().any(|n| n == "notes.tmp"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn toggling_hidden_reveals_ignored_entries() {
        let dir = temp_project("toggle");
        let mut tree = FileTree::new(&dir);
        tree.toggle_hidden();
        let names = names(&tree);
        assert!(names.iter().any(|n| n == "generated"));
        assert!(names.iter().any(|n| n == "notes.tmp"));
        fs::remove_dir_all(&dir).ok();
    }
}
