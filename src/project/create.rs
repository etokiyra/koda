//! Deterministic, offline project scaffolding.
//!
//! Creating a project writes a small, conventional structure directly rather
//! than shelling out to `cargo`, `go` or `pip`. That keeps the operation
//! instant, deterministic and usable with no network and no toolchain, which
//! matches Koda's zero-configuration promise: the project is ready to code in
//! before the user sees the editor.
//!
//! Initialization never removes anything. If a write fails after the directory
//! was created, the outcome carries the partially created root so the UI can
//! explain what happened without destroying the user's files.

use std::path::{Path, PathBuf};

use crate::language::id::LanguageId;

/// Languages Koda can scaffold, in the order it presents them.
pub const CREATABLE: &[LanguageId] = &[
    LanguageId::Rust,
    LanguageId::Go,
    LanguageId::Python,
    LanguageId::Shell,
];

/// Whether Koda can generate a conventional project for `language`.
pub fn is_creatable(language: LanguageId) -> bool {
    CREATABLE.contains(&language)
}

/// A short description of what a language's template generates.
pub fn describe(language: LanguageId) -> &'static str {
    match language {
        LanguageId::Rust => "Cargo.toml and src/main.rs (binary crate)",
        LanguageId::Go => "go.mod and main.go (package main)",
        LanguageId::Python => "pyproject.toml and a src/<package> entry point",
        LanguageId::Shell => "an executable <name>.sh script",
        _ => "a minimal project structure",
    }
}

/// The outcome of creating a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateOutcome {
    /// The project was created; `files` lists what was written, for reporting.
    Created { root: PathBuf, files: Vec<PathBuf> },
    /// Creation failed. `root` is `Some` when a partial directory was left
    /// behind (Koda never deletes it).
    Failed {
        root: Option<PathBuf>,
        message: String,
    },
}

impl CreateOutcome {
    /// The project root, whether creation succeeded or was partial.
    pub fn root(&self) -> Option<&Path> {
        match self {
            CreateOutcome::Created { root, .. } => Some(root),
            CreateOutcome::Failed { root, .. } => root.as_deref(),
        }
    }
}

/// Validate a project directory name, independent of the target language.
///
/// Returns a human-readable reason when the name cannot be used safely on the
/// current platform.
pub fn validate_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Enter a project name".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("That name is not allowed".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("A name cannot contain path separators".to_string());
    }
    if trimmed.chars().any(|c| c == '\0' || c.is_control()) {
        return Err("A name cannot contain control characters".to_string());
    }
    if trimmed.chars().count() > 128 {
        return Err("That name is too long".to_string());
    }
    if cfg!(windows) {
        if trimmed.ends_with('.') || trimmed.ends_with(' ') {
            return Err("Windows does not allow names ending in a dot or space".to_string());
        }
        let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
        const RESERVED: &[&str] = &[
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if RESERVED.contains(&stem.as_str()) {
            return Err("That name is reserved on Windows".to_string());
        }
    }
    Ok(())
}

/// Create a project named `name` inside `parent` for `language`.
pub fn create(parent: &Path, name: &str, language: LanguageId) -> CreateOutcome {
    let name = name.trim();
    if let Err(message) = validate_name(name) {
        return CreateOutcome::Failed {
            root: None,
            message,
        };
    }
    let root = parent.join(name);
    if root.exists() {
        return CreateOutcome::Failed {
            root: None,
            message: format!("{} already exists", root.display()),
        };
    }
    if let Err(err) = std::fs::create_dir_all(&root) {
        return CreateOutcome::Failed {
            root: None,
            message: format!("could not create the folder: {err}"),
        };
    }

    let result = match language {
        LanguageId::Rust => rust(&root, name),
        LanguageId::Go => go(&root, name),
        LanguageId::Python => python(&root, name),
        LanguageId::Shell => shell(&root, name),
        _ => Ok(Vec::new()),
    };

    match result {
        Ok(files) => CreateOutcome::Created { root, files },
        Err(message) => CreateOutcome::Failed {
            root: Some(root),
            message,
        },
    }
}

/// Write `contents` to `path`, returning the path on success.
fn write(root: &Path, relative: &str, contents: &str) -> Result<PathBuf, String> {
    let path = root.join(relative);
    crate::filesystem::write_string(&path, contents)
        .map_err(|err| format!("could not write {}: {err}", path.display()))?;
    Ok(path)
}

fn rust(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let package = crate_name(name);
    let manifest = format!(
        "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n"
    );
    let main = format!("fn main() {{\n    println!(\"Hello from {name}!\");\n}}\n");
    Ok(vec![
        write(root, "Cargo.toml", &manifest)?,
        write(root, "src/main.rs", &main)?,
        write(root, ".gitignore", "/target\n")?,
    ])
}

fn go(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let module = module_name(name);
    let manifest = format!("module {module}\n\ngo 1.18\n");
    let main = format!(
        "package main\n\nimport \"fmt\"\n\nfunc main() {{\n\tfmt.Println(\"Hello from {name}!\")\n}}\n"
    );
    Ok(vec![
        write(root, "go.mod", &manifest)?,
        write(root, "main.go", &main)?,
    ])
}

fn python(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let module = python_module(name);
    let manifest = format!(
        "[project]\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"A new project created with Koda\"\nrequires-python = \">=3.9\"\n\n[build-system]\nrequires = [\"setuptools>=61\"]\nbuild-backend = \"setuptools.build_meta\"\n"
    );
    let init = format!("\"\"\"{name} package.\"\"\"\n\n__version__ = \"0.1.0\"\n");
    let entry = "def main() -> None:\n    print(\"Hello from your new project!\")\n\n\nif __name__ == \"__main__\":\n    main()\n";
    let readme = format!("# {name}\n\nA Python project created with Koda.\n");
    Ok(vec![
        write(root, "pyproject.toml", &manifest)?,
        write(root, &format!("src/{module}/__init__.py"), &init)?,
        write(root, &format!("src/{module}/__main__.py"), entry)?,
        write(root, "README.md", &readme)?,
    ])
}

fn shell(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n\nmain() {{\n    echo \"Hello from {name}!\"\n}}\n\nmain \"$@\"\n"
    );
    let readme = format!("# {name}\n\nA shell project created with Koda.\n");
    let script_path = write(root, &format!("{name}.sh"), &script)?;
    make_executable(&script_path);
    Ok(vec![script_path, write(root, "README.md", &readme)?])
}

/// Best-effort `chmod +x` for shell entry points on Unix.
fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// A valid Cargo package name derived from the directory name.
pub fn crate_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    out = out.trim_matches('-').to_string();
    if out.is_empty() {
        out = "app".to_string();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out = format!("app-{out}");
    }
    out
}

/// A valid Go module path derived from the directory name.
pub fn module_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    out = out.trim_matches('-').to_string();
    if out.is_empty() {
        out = "app".to_string();
    }
    out
}

/// A valid Python package name derived from the directory name.
pub fn python_module(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    out = out.trim_matches('_').to_string();
    if out.is_empty() {
        out = "app".to_string();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out = format!("_{out}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-create-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rejects_invalid_names() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
        assert!(validate_name("nul").is_ok()); // only reserved on Windows
        assert!(validate_name("my project").is_ok());
        assert!(validate_name("プロジェクト").is_ok());
    }

    #[test]
    fn refuses_an_existing_target() {
        let dir = scratch("exists");
        let parent = dir.join("parent");
        std::fs::create_dir_all(parent.join("taken")).unwrap();
        let outcome = create(&parent, "taken", LanguageId::Rust);
        assert!(matches!(outcome, CreateOutcome::Failed { root: None, .. }));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_rust() {
        let dir = scratch("rust");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let outcome = create(&parent, "my app", LanguageId::Rust);
        let CreateOutcome::Created { root, files } = outcome else {
            panic!("expected success, got {outcome:?}");
        };
        assert!(files.iter().any(|f| f.ends_with("Cargo.toml")));
        assert!(root.join("src/main.rs").is_file());
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("name = \"my-app\""));
        assert!(manifest.contains("edition = \"2024\""));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_go() {
        let dir = scratch("go");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let outcome = create(&parent, "hello", LanguageId::Go);
        let CreateOutcome::Created { root, .. } = outcome else {
            panic!("expected success");
        };
        let manifest = std::fs::read_to_string(root.join("go.mod")).unwrap();
        assert_eq!(manifest, "module hello\n\ngo 1.18\n");
        assert!(root.join("main.go").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_python_with_an_importable_module() {
        let dir = scratch("python");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let outcome = create(&parent, "My Service", LanguageId::Python);
        let CreateOutcome::Created { root, .. } = outcome else {
            panic!("expected success");
        };
        assert!(root.join("pyproject.toml").is_file());
        assert!(root.join("src/my_service/__init__.py").is_file());
        assert!(root.join("src/my_service/__main__.py").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_shell_and_marks_it_executable() {
        let dir = scratch("shell");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let outcome = create(&parent, "deploy", LanguageId::Shell);
        let CreateOutcome::Created { root, .. } = outcome else {
            panic!("expected success");
        };
        let script = root.join("deploy.sh");
        assert!(script.is_file());
        let contents = std::fs::read_to_string(&script).unwrap();
        assert!(contents.starts_with("#!/usr/bin/env bash"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&script).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "script should be executable");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn handles_names_with_spaces_and_unicode() {
        let dir = scratch("names");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let outcome = create(&parent, "проект", LanguageId::Rust);
        let CreateOutcome::Created { root, .. } = outcome else {
            panic!("expected success");
        };
        assert!(root.join("Cargo.toml").is_file());
        assert_eq!(crate_name("проект"), "app");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn helpers_produce_safe_identifiers() {
        assert_eq!(crate_name("My App!"), "my-app");
        assert_eq!(crate_name("2cool"), "app-2cool");
        assert_eq!(module_name("My Go App"), "my-go-app");
        assert_eq!(python_module("My-Service 2"), "my_service_2");
        assert_eq!(python_module("2fast"), "_2fast");
    }
}
