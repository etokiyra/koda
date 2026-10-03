//! Recently opened projects and files.
//!
//! A small global store — not per project — so the welcome screen can offer
//! where you were last. It lives alongside the per-project sessions in the
//! user's state directory and is written when Koda quits.
//!
//! The format is a tiny JSON object parsed without derive macros, matching the
//! session store's approach.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// How many recent projects to remember.
pub const MAX_PROJECTS: usize = 8;
/// How many recent files to remember.
pub const MAX_FILES: usize = 12;

/// Recently opened projects and files, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recent {
    pub projects: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

impl Recent {
    /// Load the store from the user's state directory, if available.
    pub fn load() -> Recent {
        state_path("recent.json")
            .and_then(|path| Recent::load_from(&path))
            .unwrap_or_default()
    }

    /// Persist the store. Errors are non-fatal.
    pub fn save(&self) {
        if let Some(path) = state_path("recent.json") {
            let _ = self.save_to(&path);
        }
    }

    pub fn load_from(path: &Path) -> Option<Recent> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Some(Recent {
            projects: string_list(value.get("projects")),
            files: string_list(value.get("files")),
        })
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let value = json!({
            "projects": self.projects.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "files": self.files.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Record a project root, moving it to the front.
    pub fn add_project(&mut self, path: &Path) {
        push_unique(&mut self.projects, path, MAX_PROJECTS);
    }

    /// Record a file, moving it to the front.
    pub fn add_file(&mut self, path: &Path) {
        push_unique(&mut self.files, path, MAX_FILES);
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.files.is_empty()
    }
}

/// Move `path` to the front, deduplicating and capping the list.
fn push_unique(list: &mut Vec<PathBuf>, path: &Path, max: usize) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    list.retain(|existing| {
        existing.canonicalize().unwrap_or_else(|_| existing.clone()) != canonical
    });
    list.insert(0, canonical);
    list.truncate(max);
}

fn string_list(value: Option<&Value>) -> Vec<PathBuf> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The Koda state directory, if a home directory is available.
pub fn state_dir() -> Option<PathBuf> {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(state).join("koda"));
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/state/koda"))
}

fn state_path(name: &str) -> Option<PathBuf> {
    state_dir().map(|dir| dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-recent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = scratch("roundtrip");
        let path = dir.join("recent.json");
        let mut recent = Recent::default();
        recent.add_project(&dir.join("alpha"));
        recent.add_file(&dir.join("main.rs"));
        recent.save_to(&path).unwrap();
        let loaded = Recent::load_from(&path).unwrap();
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.files.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moves_repeats_to_the_front_and_caps() {
        let dir = scratch("order");
        let mut recent = Recent::default();
        recent.add_project(&dir.join("a"));
        recent.add_project(&dir.join("b"));
        recent.add_project(&dir.join("a"));
        assert_eq!(recent.projects.len(), 2);
        assert!(recent.projects[0].ends_with("a"));
        for index in 0..(MAX_FILES + 5) {
            recent.add_file(&dir.join(format!("file-{index}")));
        }
        assert_eq!(recent.files.len(), MAX_FILES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
