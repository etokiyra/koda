//! Per-project session persistence.
//!
//! Koda remembers which files were open, where the cursor was and which
//! directories were expanded, so reopening a project resumes where you left
//! off. State lives in the user's state directory — never inside the project —
//! keyed by the canonical project root, and is written on quit.
//!
//! The format is a small JSON object, parsed without derive macros to keep the
//! dependency surface minimal.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// A snapshot of the UI state worth restoring.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    /// Open files, in tab order.
    pub files: Vec<PathBuf>,
    /// Index of the active tab within `files`.
    pub active: usize,
    /// Cursor `(row, col)` per file, in the same order as `files`.
    pub cursors: Vec<(usize, usize)>,
    /// Expanded directory paths.
    pub expanded: Vec<PathBuf>,
    /// Whether dotfiles and ignored directories are shown.
    pub show_hidden: bool,
}

impl Session {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.expanded.is_empty()
    }

    /// Write the session to `path`, creating parent directories.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let value = json!({
            "files": self.files.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "active": self.active,
            "cursors": self.cursors.iter().map(|(row, col)| json!([row, col])).collect::<Vec<_>>(),
            "expanded": self.expanded.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "show_hidden": self.show_hidden,
        });
        let text = serde_json::to_string_pretty(&value).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Read a session, returning `None` when it is missing or malformed.
    pub fn load_from(path: &Path) -> Option<Session> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Some(Session::from_value(&value))
    }

    fn from_value(value: &Value) -> Session {
        Session {
            files: string_list(value.get("files")),
            active: value.get("active").and_then(Value::as_u64).unwrap_or(0) as usize,
            cursors: value
                .get("cursors")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(cursor_pair).collect())
                .unwrap_or_default(),
            expanded: string_list(value.get("expanded")),
            show_hidden: value
                .get("show_hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
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

fn cursor_pair(item: &Value) -> Option<(usize, usize)> {
    let pair = item.as_array()?;
    Some((
        pair.first()?.as_u64()? as usize,
        pair.get(1)?.as_u64()? as usize,
    ))
}

/// The state file for a project root, if a state directory is available.
pub fn session_path(root: &Path) -> Option<PathBuf> {
    let base = if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        PathBuf::from(state).join("koda")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".local/state/koda")
    };
    let key = fnv1a(root.to_string_lossy().as_bytes());
    Some(base.join(format!("session-{key:016x}.json")))
}

/// FNV-1a, for a short, stable file name from a project path.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = scratch("roundtrip");
        let path = dir.join("session.json");
        let session = Session {
            files: vec![PathBuf::from("/tmp/a.rs"), PathBuf::from("/tmp/b.rs")],
            active: 1,
            cursors: vec![(3, 4), (0, 0)],
            expanded: vec![PathBuf::from("/tmp/src")],
            show_hidden: true,
        };
        session.save_to(&path).unwrap();
        assert_eq!(Session::load_from(&path), Some(session));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_none() {
        assert_eq!(
            Session::load_from(Path::new("/nonexistent/koda/session.json")),
            None
        );
    }

    #[test]
    fn session_path_is_stable_per_root() {
        let (Some(a), Some(b)) = (
            session_path(Path::new("/tmp/project-a")),
            session_path(Path::new("/tmp/project-b")),
        ) else {
            return; // No HOME/XDG in this environment.
        };
        assert_ne!(a, b);
        assert_eq!(Some(&a), session_path(Path::new("/tmp/project-a")).as_ref());
    }
}
