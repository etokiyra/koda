//! Thin, well-behaved filesystem helpers.
//!
//! All reads and writes funnel through here so the rest of Koda does not sprinkle
//! `std::fs` calls around. Errors are surfaced rather than swallowed.

pub mod gitignore;

use std::path::{Path, PathBuf};

use gitignore::Gitignore;

/// A directory entry with just the metadata the UI needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Directories that are almost never useful to browse in an IDE tree.
pub const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    "dist",
    "build",
    ".cache",
];

/// Read a directory, sorted with directories first and then alphabetically.
pub fn read_dir_sorted(path: &Path) -> std::io::Result<Vec<EntryInfo>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().into_owned();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        entries.push(EntryInfo {
            name,
            path: entry.path(),
            is_dir: file_type.is_dir(),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

pub fn sort_entries(entries: &mut [EntryInfo]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

pub fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.contains(&name)
}

/// `true` when a path looks like a directory.
pub fn is_dir(path: &Path) -> bool {
    path.is_dir()
}

/// `true` when a path looks like a regular file.
pub fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// The last-modified time of a file, if it can be read.
pub fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Read a file as UTF-8 (lossy). Binary detection is left to callers that need it.
pub fn read_to_string(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Write a string to a file, creating parent directories when necessary.
pub fn write_string(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// Collect files under `root`, breadth-first, skipping ignored directories and
/// anything hidden by the project's `.gitignore` rules.
///
/// `limit` bounds the work so quick-open stays responsive even in huge trees.
pub fn collect_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let ignore = Gitignore::load(root);
    let mut files = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        if files.len() >= limit {
            break;
        }
        let entries = match read_dir_sorted(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            if files.len() >= limit {
                break;
            }
            if entry.is_dir {
                if entry.name.starts_with('.') || is_ignored_dir(&entry.name) {
                    continue;
                }
                if ignore.is_ignored(&entry.path, true) {
                    continue;
                }
                queue.push_back(entry.path);
            } else if !entry.name.starts_with('.') {
                if ignore.is_ignored(&entry.path, false) {
                    continue;
                }
                files.push(entry.path);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn collect_files_respects_gitignore() {
        let dir = std::env::temp_dir().join(format!("koda-fs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("build_out")).unwrap();
        fs::write(dir.join(".gitignore"), "build_out/\n").unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
        fs::write(dir.join("build_out/gen.rs"), "").unwrap();

        let files = collect_files(&dir, 100);
        let names: Vec<String> = files
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|name| name == "lib.rs"));
        assert!(!names.iter().any(|name| name == "gen.rs"));

        fs::remove_dir_all(&dir).ok();
    }
}
