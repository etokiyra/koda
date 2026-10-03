//! Project-wide text search.
//!
//! A fast, dependency-free scan over the files Koda already knows about
//! (respecting `.gitignore`), used by the background worker for **Search in
//! Project…**. Matching is a plain case-insensitive substring search, line by
//! line; binary and oversized files are skipped so the scan stays predictable.

use std::path::{Path, PathBuf};

/// A single matching line in a project file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line: usize,
    /// Zero-based character column of the match.
    pub col: usize,
    /// The trimmed line text, for display.
    pub text: String,
}

/// Files larger than this are skipped: they are almost never source, and
/// reading them would stall the scan.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// How many files to consider, mirroring quick open's bound.
const MAX_FILES: usize = 8000;

/// Search every text file under `root` for `query`, up to `limit` matches.
///
/// At most one match is reported per line, which keeps the result list legible
/// on minified or repetitive files.
pub fn search_project(root: &Path, query: &str, limit: usize) -> Vec<SearchMatch> {
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let mut matches = Vec::new();

    for path in crate::filesystem::collect_files(root, MAX_FILES) {
        if matches.len() >= limit {
            break;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (row, line) in text.lines().enumerate() {
            if matches.len() >= limit {
                break;
            }
            let lower = line.to_lowercase();
            let Some(byte_col) = lower.find(&needle) else {
                continue;
            };
            matches.push(SearchMatch {
                path: path.clone(),
                line: row,
                col: lower[..byte_col].chars().count(),
                text: line.trim().to_string(),
            });
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-search-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    #[test]
    fn finds_case_insensitive_matches() {
        let dir = project("basic");
        fs::write(dir.join("src/a.rs"), "fn main() {\n    let Name = 1;\n}\n").unwrap();
        fs::write(dir.join("src/b.rs"), "// nothing here\n").unwrap();

        let matches = search_project(&dir, "name", 100);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[0].col, 8);
        assert!(matches[0].text.contains("let Name"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_binary_and_ignored_files() {
        let dir = project("skip");
        fs::write(dir.join("src/keep.txt"), "needle\n").unwrap();
        fs::write(dir.join("src/blob.bin"), b"needle\0binary").unwrap();
        fs::create_dir_all(dir.join("generated")).unwrap();
        fs::write(dir.join("generated/gen.txt"), "needle\n").unwrap();
        fs::write(dir.join(".gitignore"), "generated/\n").unwrap();

        let matches = search_project(&dir, "needle", 100);
        assert_eq!(matches.len(), 1);
        assert!(matches[0].path.ends_with("keep.txt"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn respects_the_limit_and_empty_query() {
        let dir = project("limit");
        fs::write(dir.join("src/many.txt"), "hit\nhit\nhit\n").unwrap();
        assert_eq!(search_project(&dir, "hit", 2).len(), 2);
        assert!(search_project(&dir, "", 10).is_empty());
        fs::remove_dir_all(&dir).ok();
    }
}
