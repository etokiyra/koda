//! Workspaces and projects.
//!
//! Koda treats files as living inside projects. A [`Project`] is the directory
//! that establishes language context, discovered by walking up from the file or
//! directory the user opened.

pub mod create;
pub mod file_tree;

use std::path::{Path, PathBuf};

use crate::git::GitInfo;
use crate::language::id::LanguageId;
use file_tree::FileTree;

/// The kind of project, inferred from its marker files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKind {
    Rust,
    Go,
    Python,
    Generic,
}

/// Every marker Koda knows about, paired with the project kind it implies.
pub const KNOWN_MARKERS: &[(&str, ProjectKind)] = &[
    ("Cargo.toml", ProjectKind::Rust),
    ("Cargo.lock", ProjectKind::Rust),
    ("go.mod", ProjectKind::Go),
    ("go.sum", ProjectKind::Go),
    ("pyproject.toml", ProjectKind::Python),
    ("setup.py", ProjectKind::Python),
    ("requirements.txt", ProjectKind::Python),
    ("Pipfile", ProjectKind::Python),
];

impl ProjectKind {
    pub fn label(self) -> &'static str {
        match self {
            ProjectKind::Rust => "Rust",
            ProjectKind::Go => "Go",
            ProjectKind::Python => "Python",
            ProjectKind::Generic => "Workspace",
        }
    }

    pub fn language(self) -> LanguageId {
        match self {
            ProjectKind::Rust => LanguageId::Rust,
            ProjectKind::Go => LanguageId::Go,
            ProjectKind::Python => LanguageId::Python,
            ProjectKind::Generic => LanguageId::Unknown,
        }
    }
}

/// A detected project.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub kind: ProjectKind,
    /// Marker file names present at the root.
    pub markers: Vec<String>,
}

impl Project {
    /// Detect the project containing `start`.
    ///
    /// Language markers (`Cargo.toml`, `go.mod`, …) take precedence over the
    /// generic `.git` marker. The nearest ancestor wins.
    pub fn detect(start: &Path) -> Project {
        let start_dir = normalize_start(start);

        // 1. Prefer the nearest language project.
        for ancestor in start_dir.ancestors() {
            let markers = markers_present(ancestor);
            if let Some(kind) = markers.iter().find_map(|m| kind_for_marker(m)) {
                return Project {
                    root: ancestor.to_path_buf(),
                    kind,
                    markers,
                };
            }
        }

        // 2. Fall back to a VCS root, then the starting directory.
        for ancestor in start_dir.ancestors() {
            if ancestor.join(".git").exists() {
                return Project {
                    root: ancestor.to_path_buf(),
                    kind: ProjectKind::Generic,
                    markers: vec![".git".to_string()],
                };
            }
        }

        Project {
            root: start_dir,
            kind: ProjectKind::Generic,
            markers: Vec::new(),
        }
    }

    /// A short description used by the status bar.
    pub fn label(&self) -> String {
        self.kind.label().to_string()
    }
}

/// A project plus the state that surrounds it: file tree and git snapshot.
pub struct Workspace {
    pub project: Project,
    pub tree: FileTree,
    pub git: GitInfo,
}

impl Workspace {
    /// Open a workspace rooted at the project containing `start` (or the current
    /// directory when `start` is `None`).
    pub fn open(start: Option<&Path>) -> std::io::Result<Workspace> {
        let start = match start {
            Some(path) => path.to_path_buf(),
            None => std::env::current_dir()?,
        };
        // Work with absolute paths so ancestor walking and git detection behave
        // consistently regardless of how the user invoked Koda.
        let start = if start.is_absolute() {
            start
        } else {
            std::env::current_dir()?.join(&start)
        };
        let start = start.canonicalize().unwrap_or(start);
        let project = Project::detect(&start);
        let tree = FileTree::new(&project.root);
        // Git status is loaded by the background worker so startup never waits on
        // a potentially slow `git status`.
        let git = GitInfo::default();
        Ok(Workspace { project, tree, git })
    }

    pub fn root(&self) -> &Path {
        &self.project.root
    }

    /// Re-read the file tree.
    pub fn refresh(&mut self) {
        self.tree.refresh();
    }

    pub fn marker_names(&self) -> Vec<String> {
        self.project.markers.clone()
    }
}

fn normalize_start(start: &Path) -> PathBuf {
    if start.is_file() {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        start.to_path_buf()
    }
}

fn markers_present(dir: &Path) -> Vec<String> {
    KNOWN_MARKERS
        .iter()
        .filter(|(name, _)| dir.join(name).exists())
        .map(|(name, _)| (*name).to_string())
        .collect()
}

/// The marker file names of the nearest enclosing project for `path`.
///
/// Walks up from `path`'s directory and returns the first directory that
/// declares a known project marker. This lets a file inside a monorepo
/// subproject (`crates/a/Cargo.toml`) get the same corroborating context it
/// would in a single-project checkout. Falls back to `fallback` (usually the
/// workspace root's markers) when nothing is found, so behaviour is unchanged
/// for ordinary workspaces.
pub fn nearest_markers(path: &Path, fallback: &[String]) -> Vec<String> {
    let start = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    if let Some(start) = start {
        for ancestor in start.ancestors() {
            let markers = markers_present(ancestor);
            if !markers.is_empty() {
                return markers;
            }
        }
    }
    fallback.to_vec()
}

fn kind_for_marker(marker: &str) -> Option<ProjectKind> {
    KNOWN_MARKERS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(marker))
        .map(|(_, kind)| *kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_rust_project_from_nested_file() {
        let dir = std::env::temp_dir().join(format!("koda-proj-test-{}", std::process::id()));
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        let file = src.join("main.rs");
        fs::write(&file, "fn main() {}").unwrap();

        let project = Project::detect(&file);
        assert_eq!(project.kind, ProjectKind::Rust);
        assert_eq!(
            project.root.canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nearest_markers_finds_nested_subprojects() {
        let dir = std::env::temp_dir().join(format!("koda-mono-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sub = dir.join("crates/a/src");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.join("crates/a/Cargo.toml"), "[package]").unwrap();
        let file = sub.join("lib.rs");
        fs::write(&file, "pub fn a() {}\n").unwrap();

        assert_eq!(nearest_markers(&file, &[]), vec!["Cargo.toml".to_string()]);

        // Falls back when no ancestor declares a marker.
        let plain = dir.join("notes/readme.txt");
        fs::create_dir_all(dir.join("notes")).unwrap();
        fs::write(&plain, "hi").unwrap();
        assert_eq!(
            nearest_markers(&plain, &["go.mod".to_string()]),
            vec!["go.mod".to_string()]
        );

        fs::remove_dir_all(&dir).ok();
    }
}
