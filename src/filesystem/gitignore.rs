//! A pragmatic `.gitignore` matcher.
//!
//! Koda deliberately links no git library, so this implements the subset of
//! `.gitignore` syntax that removes noise from the file tree, quick open, the
//! inline tree filter and workspace symbol search:
//!
//! * comments (`#`) and blank lines
//! * negation (`!`)
//! * directory-only patterns (trailing `/`)
//! * anchoring (a leading `/` or any other `/` pins a pattern to the file's
//!   directory)
//! * `*`, `?` and `**` wildcards
//!
//! Character classes (`[a-z]`) and backslash escapes are not supported; a
//! pattern using them simply will not match. That is a deliberate trade-off:
//! the common cases are covered without pulling in a glob dependency.
//!
//! Nested `.gitignore` files are discovered once, shallow-first, so a deeper
//! rule overrides a shallower one exactly as git does. `.git/info/exclude` is
//! read as well.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::IGNORED_DIRS;

/// Upper bound on how many nested `.gitignore` files are read. Keeps loading
/// predictable in enormous monorepos.
const MAX_GITIGNORE_FILES: usize = 256;
/// Upper bound on directories visited while discovering nested `.gitignore`.
const MAX_DIRS: usize = 4096;

/// One parsed `.gitignore` line.
struct Rule {
    /// The directory holding the `.gitignore`, relative to the root (`""` at the
    /// root).
    base: PathBuf,
    pattern: String,
    negated: bool,
    dir_only: bool,
    /// Whether the pattern is pinned to `base` (rather than matching at any
    /// depth).
    anchored: bool,
}

/// The effective ignore rules for a project root.
pub struct Gitignore {
    root: PathBuf,
    rules: Vec<Rule>,
}

impl Gitignore {
    /// Load the root `.gitignore`, `.git/info/exclude` and any nested
    /// `.gitignore` files under `root`, shallow-first.
    pub fn load(root: &Path) -> Self {
        let mut rules = Vec::new();
        let mut queue: VecDeque<PathBuf> = VecDeque::new();
        queue.push_back(PathBuf::new());
        let mut files = 0usize;
        let mut dirs = 0usize;

        while let Some(relative) = queue.pop_front() {
            if dirs >= MAX_DIRS {
                break;
            }
            dirs += 1;
            let dir = root.join(&relative);

            // `.git/info/exclude` has lower priority than any `.gitignore`, so
            // its rules are read first; last-match-wins then lets `.gitignore`
            // (root or nested) override them, as git does.
            if relative.as_os_str().is_empty() {
                let exclude = root.join(".git/info/exclude");
                if exclude.is_file() {
                    read_rules(&exclude, &relative, &mut rules);
                }
            }
            let ignore_file = dir.join(".gitignore");
            if files < MAX_GITIGNORE_FILES && ignore_file.is_file() {
                files += 1;
                read_rules(&ignore_file, &relative, &mut rules);
            }

            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || IGNORED_DIRS.contains(&name.as_str()) {
                    continue;
                }
                queue.push_back(relative.join(&name));
            }
        }

        Gitignore {
            root: root.to_path_buf(),
            rules,
        }
    }

    /// Whether there are no rules to apply.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether `path` is ignored. `is_dir` selects directory-only patterns.
    ///
    /// The path is expected to live under the root the matcher was loaded from.
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        let relative = match path.strip_prefix(&self.root) {
            Ok(relative) => relative,
            Err(_) => return false,
        };
        let Some(relative) = path_to_slash(relative) else {
            return false;
        };
        let basename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");

        let mut ignored = false;
        for rule in &self.rules {
            if rule.dir_only && !is_dir {
                continue;
            }
            let Some(under_base) = under_base(&relative, &rule.base) else {
                continue;
            };
            let matched = if rule.anchored {
                glob_paths(&rule.pattern, under_base)
                    || (rule.dir_only && dir_prefix(&rule.pattern, under_base))
            } else {
                segment_match(&rule.pattern, basename)
            };
            if matched {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

/// Parse one ignore file into rules based in `base`.
fn read_rules(file: &Path, base: &Path, rules: &mut Vec<Rule>) {
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negated, rest) = match line.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let (dir_only, rest) = match rest.strip_suffix('/') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let (mut anchored, rest) = match rest.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let pattern = rest.trim_end().to_string();
        if pattern.is_empty() {
            continue;
        }
        anchored |= pattern.contains('/');
        rules.push(Rule {
            base: base.to_path_buf(),
            pattern,
            negated,
            dir_only,
            anchored,
        });
    }
}

/// Render a relative path with `/` separators, or `None` for non-UTF-8 paths.
fn path_to_slash(path: &Path) -> Option<String> {
    let mut out = String::new();
    for component in path.components() {
        let part = component.as_os_str().to_str()?;
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    Some(out)
}

/// Strip `base` from `relative`, returning the part beneath it.
fn under_base<'a>(relative: &'a str, base: &Path) -> Option<&'a str> {
    let base = base.to_str()?;
    if base.is_empty() {
        return Some(relative);
    }
    if relative == base {
        return Some("");
    }
    relative
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// Whether `relative` sits inside the directory named by `pattern`.
fn dir_prefix(pattern: &str, relative: &str) -> bool {
    relative.len() > pattern.len()
        && relative.starts_with(pattern)
        && relative.as_bytes()[pattern.len()] == b'/'
}

/// Match a slash-separated glob (supporting `**`) against a slash path.
fn glob_paths(pattern: &str, text: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let text: Vec<&str> = text.split('/').collect();
    match_segments(&pattern, &text)
}

fn match_segments(pattern: &[&str], text: &[&str]) -> bool {
    // Memoise subproblems by their lengths; a pattern with several `**` would
    // otherwise backtrack exponentially against a deep path.
    let mut seen = std::collections::HashSet::new();
    match_segments_rec(pattern, text, &mut seen)
}

fn match_segments_rec(
    pattern: &[&str],
    text: &[&str],
    seen: &mut std::collections::HashSet<(usize, usize)>,
) -> bool {
    if !seen.insert((pattern.len(), text.len())) {
        return false;
    }
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            (0..=text.len()).any(|skip| match_segments_rec(&pattern[1..], &text[skip..], seen))
        }
        (Some(segment), Some(part)) => {
            segment_match(segment, part) && match_segments_rec(&pattern[1..], &text[1..], seen)
        }
        _ => false,
    }
}

/// Match one path segment, where `*` and `?` do not cross a separator.
fn segment_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    segment_rec(&pattern, &text)
}

/// Iterative wildcard match (no exponential backtracking).
///
/// Standard two-pointer algorithm with a single remembered `*`: the worst case
/// is O(pattern × text), never the exponential blow-up a naive recursive `*`
/// gives on a pattern like `*a*a*a*a*b`.
fn segment_rec(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_text = 0usize;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            star_text = t;
            p += 1;
        } else if let Some(star_p) = star {
            p = star_p + 1;
            star_text += 1;
            t = star_text;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(name: &str, gitignore: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-ignore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".gitignore"), gitignore).unwrap();
        dir
    }

    fn ignored(matcher: &Gitignore, relative: &str, is_dir: bool) -> bool {
        matcher.is_ignored(&matcher.root.join(relative), is_dir)
    }

    #[test]
    fn matches_directory_and_file_globs() {
        let dir = project("basic", "target/\n*.log\n");
        let matcher = Gitignore::load(&dir);
        assert!(ignored(&matcher, "target", true));
        assert!(!ignored(&matcher, "target", false));
        assert!(ignored(&matcher, "deep/notes.log", false));
        assert!(!ignored(&matcher, "src/main.rs", false));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn negation_reincludes() {
        let dir = project("negate", "*.log\n!keep.log\n");
        let matcher = Gitignore::load(&dir);
        assert!(ignored(&matcher, "a.log", false));
        assert!(!ignored(&matcher, "keep.log", false));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leading_slash_anchors_to_the_root() {
        let dir = project("anchor", "/build\n");
        let matcher = Gitignore::load(&dir);
        assert!(ignored(&matcher, "build", true));
        assert!(!ignored(&matcher, "src/build", true));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn double_star_spans_directories() {
        let dir = project("doublestar", "**/generated\nsrc/*.rs\n");
        let matcher = Gitignore::load(&dir);
        assert!(ignored(&matcher, "a/b/generated", true));
        assert!(ignored(&matcher, "src/lib.rs", false));
        assert!(!ignored(&matcher, "src/sub/lib.rs", false));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nested_gitignore_overrides_the_root() {
        let dir = project("nested", "*.log\n");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/.gitignore"), "!keep.log\n").unwrap();
        let matcher = Gitignore::load(&dir);
        assert!(ignored(&matcher, "sub/drop.log", false));
        assert!(!ignored(&matcher, "sub/keep.log", false));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let dir = project("comments", "# comment\n\n   \nreal\n");
        let matcher = Gitignore::load(&dir);
        assert_eq!(matcher.rules.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gitignore_overrides_info_exclude() {
        let dir = project("exclude", "*.log\n");
        fs::create_dir_all(dir.join(".git/info")).unwrap();
        fs::write(dir.join(".git/info/exclude"), "!keep.log\n").unwrap();
        let matcher = Gitignore::load(&dir);
        assert!(
            ignored(&matcher, "keep.log", false),
            "a .gitignore rule outranks .git/info/exclude"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hostile_globs_are_not_exponential() {
        // These would blow up a naive recursive matcher; both must return fast.
        let text = "a".repeat(128);
        assert!(!segment_match("*a*a*a*a*a*a*a*a*a*a*a*a*a*b", &text));
        assert!(!glob_paths(
            "**/**/**/**/**/**/**/**/x",
            "a/b/c/d/e/f/g/h/i/j"
        ));
    }
}
