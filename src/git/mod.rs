//! Lightweight git integration.
//!
//! Koda shells out to the `git` binary rather than linking a large library. This
//! keeps the dependency graph small and matches the user's real repositories.
//! If git is missing or the directory is not a repository, everything degrades
//! gracefully.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The status of a single file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
    TypeChanged,
}

impl GitFileStatus {
    /// A single-character indicator, similar to `git status --short`.
    pub fn indicator(self) -> char {
        match self {
            GitFileStatus::Modified => 'M',
            GitFileStatus::Added => 'A',
            GitFileStatus::Deleted => 'D',
            GitFileStatus::Renamed => 'R',
            GitFileStatus::Untracked => '?',
            GitFileStatus::Conflicted => 'U',
            GitFileStatus::TypeChanged => 'T',
        }
    }

    /// A human-readable name for lists.
    pub fn label(self) -> &'static str {
        match self {
            GitFileStatus::Modified => "modified",
            GitFileStatus::Added => "added",
            GitFileStatus::Deleted => "deleted",
            GitFileStatus::Renamed => "renamed",
            GitFileStatus::Untracked => "untracked",
            GitFileStatus::Conflicted => "conflicted",
            GitFileStatus::TypeChanged => "type changed",
        }
    }
}

/// Snapshot of repository state for a directory.
#[derive(Clone, Debug, Default)]
pub struct GitInfo {
    pub repo_root: Option<PathBuf>,
    pub branch: Option<String>,
    pub files: HashMap<PathBuf, GitFileStatus>,
    /// `true` when a repository was found and git responded.
    pub available: bool,
}

impl GitInfo {
    /// Inspect the repository containing `dir`.
    pub fn detect(dir: &Path) -> GitInfo {
        let Some(repo_root) = run(dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from) else {
            return GitInfo::default();
        };

        let branch = run(&repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let mut files = HashMap::new();

        if let Some(output) = run(
            &repo_root,
            &["status", "--porcelain", "--untracked-files=normal"],
        ) {
            for line in output.lines() {
                if line.len() < 4 {
                    continue;
                }
                let status = parse_status(&line[..2]);
                // Renames are reported as "old -> new"; take the destination.
                let raw_path = line[3..].trim();
                let path = match raw_path.split_once(" -> ") {
                    Some((_, new)) => new,
                    None => raw_path,
                };
                let path = path.trim_matches('"');
                files.insert(repo_root.join(path), status);
            }
        }

        GitInfo {
            repo_root: Some(repo_root),
            branch,
            files,
            available: true,
        }
    }

    /// The status of a path, trying the path itself and its parents.
    pub fn status_for(&self, path: &Path) -> Option<GitFileStatus> {
        if let Some(status) = self.files.get(path) {
            return Some(*status);
        }
        // Git reports the containing directory for some statuses.
        let mut current = path.parent();
        while let Some(dir) = current {
            if let Some(status) = self.files.get(dir) {
                return Some(*status);
            }
            current = dir.parent();
        }
        None
    }

    /// Short branch label for the status bar.
    pub fn branch_label(&self) -> Option<&str> {
        self.branch.as_deref()
    }
}

/// Run a git subcommand in `dir`, returning trimmed stdout on success.
fn run(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn parse_status(code: &str) -> GitFileStatus {
    let mut chars = code.chars();
    let x = chars.next().unwrap_or(' ');
    let y = chars.next().unwrap_or(' ');

    if x == '?' && y == '?' {
        return GitFileStatus::Untracked;
    }
    if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
        return GitFileStatus::Conflicted;
    }
    if x == 'R' || y == 'R' {
        return GitFileStatus::Renamed;
    }
    if x == 'T' || y == 'T' {
        return GitFileStatus::TypeChanged;
    }
    if y == 'D' || x == 'D' {
        return GitFileStatus::Deleted;
    }
    if y == 'A' || x == 'A' {
        return GitFileStatus::Added;
    }
    GitFileStatus::Modified
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_codes() {
        assert_eq!(parse_status("??"), GitFileStatus::Untracked);
        assert_eq!(parse_status(" M"), GitFileStatus::Modified);
        assert_eq!(parse_status("A "), GitFileStatus::Added);
        assert_eq!(parse_status("UU"), GitFileStatus::Conflicted);
        assert_eq!(parse_status("R "), GitFileStatus::Renamed);
    }
}
