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
    /// Paths that have staged (index) changes.
    pub staged: std::collections::HashSet<PathBuf>,
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
        let mut staged = std::collections::HashSet::new();

        if let Some(output) = run(
            &repo_root,
            &["status", "--porcelain", "--untracked-files=normal"],
        ) {
            for line in output.lines() {
                if line.len() < 4 {
                    continue;
                }
                let code = &line[..2];
                let status = parse_status(code);
                // Renames are reported as "old -> new"; take the destination.
                let raw_path = line[3..].trim();
                let path = match raw_path.split_once(" -> ") {
                    Some((_, new)) => new,
                    None => raw_path,
                };
                let path = path.trim_matches('"');
                let absolute = repo_root.join(path);
                // The first column is the index (staged) state; `?` is untracked.
                let index = code.chars().next().unwrap_or(' ');
                if index != ' ' && index != '?' {
                    staged.insert(absolute.clone());
                }
                files.insert(absolute, status);
            }
        }

        GitInfo {
            repo_root: Some(repo_root),
            branch,
            files,
            staged,
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

    /// Whether `path` has staged changes.
    pub fn is_staged(&self, path: &Path) -> bool {
        self.staged.contains(path)
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

/// Run a git subcommand, returning trimmed stdout or the first error line.
///
/// Unlike [`run`], this surfaces failures so the UI can explain what git said.
fn run_checked(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("git command failed")
        .trim()
        .to_string();
    Err(message)
}

/// Stage every change in the repository at `root`.
pub fn stage_all(root: &Path) -> Result<(), String> {
    run_checked(root, &["add", "-A"]).map(|_| ())
}

/// Stage every change and commit it with `message`.
///
/// Koda never rewrites history; this is a plain `git add -A` followed by a
/// `git commit`, run only when the user explicitly asks for it.
pub fn commit_all(root: &Path, message: &str) -> Result<String, String> {
    stage_all(root)?;
    let output = run_checked(root, &["commit", "-m", message])?;
    if output.is_empty() {
        Ok("Committed".to_string())
    } else {
        Ok(output)
    }
}

/// Stage one path (`git add -- <path>`).
pub fn stage(root: &Path, path: &Path) -> Result<(), String> {
    let path = path.to_string_lossy().to_string();
    run_checked(root, &["add", "--", path.as_str()]).map(|_| ())
}

/// Unstage one path, falling back to `git reset` on older git versions.
pub fn unstage(root: &Path, path: &Path) -> Result<(), String> {
    let path = path.to_string_lossy().to_string();
    match run_checked(root, &["restore", "--staged", "--", path.as_str()]) {
        Ok(_) => Ok(()),
        Err(_) => run_checked(root, &["reset", "-q", "HEAD", "--", path.as_str()]).map(|_| ()),
    }
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

    #[test]
    fn stages_and_commits_when_git_is_available() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // Skip when git is unavailable.
        }
        let dir = std::env::temp_dir().join(format!("koda-git-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
        };
        // A fresh repository with a deterministic identity.
        assert!(git(&["init", "-q"]).unwrap().status.success());
        assert!(
            git(&["config", "user.email", "koda@example.com"])
                .unwrap()
                .status
                .success()
        );
        assert!(
            git(&["config", "user.name", "Koda Test"])
                .unwrap()
                .status
                .success()
        );
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();

        assert!(stage_all(&dir).is_ok());
        let result = commit_all(&dir, "initial commit");
        assert!(result.is_ok(), "commit failed: {result:?}");

        let log = git(&["log", "--oneline"]).unwrap();
        assert!(String::from_utf8_lossy(&log.stdout).contains("initial commit"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stages_and_unstages_a_file() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // Skip when git is unavailable.
        }
        let dir = std::env::temp_dir().join(format!("koda-git-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
        };
        assert!(git(&["init", "-q"]).unwrap().status.success());
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        let path = dir.join("a.txt");

        let info = GitInfo::detect(&dir);
        assert!(info.available);
        assert!(!info.is_staged(&path), "an untracked file is not staged");

        stage(&dir, &path).unwrap();
        assert!(GitInfo::detect(&dir).is_staged(&path));

        unstage(&dir, &path).unwrap();
        assert!(!GitInfo::detect(&dir).is_staged(&path));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
