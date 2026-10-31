//! Task detection: the project's own commands, found with no configuration.
//!
//! Koda detects the project kind from its markers and offers the commands that
//! kind implies — `cargo test` for a Rust project, the scripts in
//! `package.json`, a plain `make` where a Makefile drives the build. A task is
//! plain data (a program and its arguments); nothing in the editor or the UI
//! needs to know about a specific ecosystem, so adding a project kind's tasks
//! is one more branch in [`detect`].

use std::path::Path;

use serde_json::Value;

/// A task the project provides: a command run in the project root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// The name shown in the task picker, e.g. `npm run check`.
    pub label: String,
    /// The program to run, resolved on `PATH` at spawn time.
    pub program: String,
    /// The arguments following the program.
    pub args: Vec<String>,
}

impl Task {
    fn builtin(label: &str, program: &str, args: &[&str]) -> Task {
        Task {
            label: label.to_string(),
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }

    /// The full command line, as it appears in the picker and in documentation.
    pub fn command_line(&self) -> String {
        let mut line = self.program.clone();
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }
}

/// Detect the tasks of the project rooted at `root`.
///
/// Detection reads only the project's marker files, so it is cheap enough to
/// call when the task picker opens. A project kind with no marker found offers
/// nothing rather than a guessed command.
pub fn detect(root: &Path) -> Vec<Task> {
    let mut tasks = Vec::new();
    if root.join("Cargo.toml").is_file() {
        tasks.push(Task::builtin("Build", "cargo", &["build"]));
        tasks.push(Task::builtin("Test", "cargo", &["test"]));
    }
    if root.join("go.mod").is_file() {
        tasks.push(Task::builtin("Build", "go", &["build", "./..."]));
        tasks.push(Task::builtin("Test", "go", &["test", "./..."]));
    }
    if root.join("Makefile").is_file() {
        tasks.push(Task::builtin("Build (make)", "make", &[]));
    }
    if root.join("pyproject.toml").is_file() {
        tasks.push(Task::builtin("Test (pytest)", "python3", &["-m", "pytest"]));
    }
    if root.join("mix.exs").is_file() {
        tasks.push(Task::builtin("Test (mix)", "mix", &["test"]));
    }
    if root.join("pom.xml").is_file() {
        tasks.push(Task::builtin("Test (maven)", "mvn", &["test"]));
    }
    if root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file() {
        tasks.push(Task::builtin("Test (gradle)", "gradle", &["test"]));
    }
    if root.join("pubspec.yaml").is_file() {
        tasks.push(Task::builtin("Test (flutter)", "flutter", &["test"]));
    }
    if has_csproj(root) {
        tasks.push(Task::builtin("Build (dotnet)", "dotnet", &["build"]));
        tasks.push(Task::builtin("Test (dotnet)", "dotnet", &["test"]));
    }
    tasks.extend(detect_scripts(&root.join("package.json")));
    tasks
}

/// Whether the project contains a .NET project file.
fn has_csproj(root: &Path) -> bool {
    std::fs::read_dir(root)
        .ok()
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("csproj"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// The tasks implied by `package.json`: every declared script, capped so one
/// enormous object cannot flood the picker.
///
/// Lifecycle scripts (`pre*`, `post*`) are skipped: they run automatically with
/// their main script, so listing them would duplicate rows.
fn detect_scripts(path: &Path) -> Vec<Task> {
    const MAX_SCRIPTS: usize = 12;
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        // A malformed package.json is the project author's bug; Koda offers the
        // other tasks it found and says nothing here.
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut names: Vec<&String> = scripts
        .keys()
        .filter(|name| !name.starts_with("pre") && !name.starts_with("post"))
        .take(MAX_SCRIPTS)
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| Task {
            label: format!("npm run {name}"),
            program: "npm".to_string(),
            args: vec!["run".to_string(), name.clone()],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-tasks-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_rust_project_gets_build_and_test() {
        let dir = scratch("rust");
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let tasks = detect(&dir);
        let lines: Vec<String> = tasks.iter().map(Task::command_line).collect();
        assert_eq!(lines, vec!["cargo build", "cargo test"]);
        for task in tasks {
            assert_eq!(task.program, "cargo");
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_go_project_gets_build_and_test() {
        let dir = scratch("go");
        fs::write(dir.join("go.mod"), "module x\n\ngo 1.21\n").unwrap();
        let tasks = detect(&dir);
        let lines: Vec<String> = tasks.iter().map(Task::command_line).collect();
        assert_eq!(lines, vec!["go build ./...", "go test ./..."]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn makers_and_pure_tools_are_offered() {
        let dir = scratch("mixed");
        fs::write(dir.join("Makefile"), "all:\n\ttrue\n").unwrap();
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        fs::write(dir.join("mix.exs"), "defmodule X do\nend\n").unwrap();
        let lines: Vec<String> = detect(&dir).iter().map(Task::command_line).collect();
        assert!(lines.contains(&"make".to_string()));
        assert!(lines.contains(&"python3 -m pytest".to_string()));
        assert!(lines.contains(&"mix test".to_string()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn npm_scripts_become_tasks_and_lifecycle_hooks_are_skipped() {
        let dir = scratch("npm");
        fs::write(
            dir.join("package.json"),
            r#"{
                "name": "x",
                "scripts": {
                    "build": "tsc",
                    "test": "vitest",
                    "prebuild": "echo one",
                    "posttest": "echo two",
                    "zeta": "echo z"
                }
            }"#,
        )
        .unwrap();
        let lines: Vec<String> = detect(&dir).iter().map(Task::command_line).collect();
        assert_eq!(
            lines,
            vec!["npm run build", "npm run test", "npm run zeta"],
            "scripts are sorted and pre/post hooks are not listed"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn more_project_kinds_are_offered() {
        let dir = scratch("more");
        fs::write(dir.join("pom.xml"), "<project/>\n").unwrap();
        fs::write(dir.join("build.gradle.kts"), "plugins {}\n").unwrap();
        fs::write(dir.join("pubspec.yaml"), "name: x\n").unwrap();
        fs::write(dir.join("App.csproj"), "<Project/>\n").unwrap();
        let lines: Vec<String> = detect(&dir).iter().map(Task::command_line).collect();
        assert!(lines.contains(&"mvn test".to_string()));
        assert!(lines.contains(&"gradle test".to_string()));
        assert!(lines.contains(&"flutter test".to_string()));
        assert!(lines.contains(&"dotnet build".to_string()));
        assert!(lines.contains(&"dotnet test".to_string()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn without_markers_no_tasks_are_offered() {
        let dir = scratch("empty");
        fs::write(dir.join("notes.txt"), "hello\n").unwrap();
        assert!(detect(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_malformed_package_json_yields_nothing() {
        let dir = scratch("bad-json");
        fs::write(dir.join("package.json"), "{ not json").unwrap();
        assert!(detect(&dir).is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_command_line_joins_the_program_and_arguments() {
        let task = Task::builtin("Test", "cargo", &["test", "--quiet"]);
        assert_eq!(task.command_line(), "cargo test --quiet");
        assert_eq!(task.label, "Test");
    }
}
