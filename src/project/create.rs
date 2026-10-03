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
    LanguageId::TypeScript,
    LanguageId::JavaScript,
    LanguageId::Java,
    LanguageId::CSharp,
    LanguageId::Php,
    LanguageId::Lua,
    LanguageId::Html,
    LanguageId::Shell,
    LanguageId::C,
    LanguageId::Cpp,
];

/// Whether Koda can generate a conventional project for `language`.
pub fn is_creatable(language: LanguageId) -> bool {
    CREATABLE.contains(&language)
}

/// A short description of what a language's template generates.
pub fn describe(language: LanguageId) -> &'static str {
    match language {
        LanguageId::Rust => "Cargo.toml + src/main.rs",
        LanguageId::Go => "go.mod + main.go",
        LanguageId::Python => "pyproject.toml + src/<package>",
        LanguageId::TypeScript => "package.json + tsconfig.json",
        LanguageId::JavaScript => "package.json + src/index.js",
        LanguageId::Java => "pom.xml + src/main/java/<package>",
        LanguageId::CSharp => "a .csproj + Program.cs",
        LanguageId::Php => "composer.json + index.php",
        LanguageId::Lua => "init.lua + .luarc.json",
        LanguageId::Html => "index.html + style.css",
        LanguageId::Shell => "an executable <name>.sh",
        LanguageId::C => "CMakeLists.txt + src/main.c",
        LanguageId::Cpp => "CMakeLists.txt + src/main.cpp",
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
    // The name is interpolated into generated source, shell scripts and HTML;
    // reject characters that could break out of those contexts.
    const UNSAFE: &[char] = &['"', '\\', '`', '$', '<', '>', '&'];
    if let Some(bad) = trimmed.chars().find(|c| UNSAFE.contains(c)) {
        return Err(format!("A name cannot contain `{bad}`"));
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
        LanguageId::TypeScript => typescript(&root, name),
        LanguageId::JavaScript => javascript(&root, name),
        LanguageId::Java => java(&root, name),
        LanguageId::CSharp => csharp(&root, name),
        LanguageId::Php => php(&root, name),
        LanguageId::Lua => lua(&root, name),
        LanguageId::Html => html(&root, name),
        LanguageId::Shell => shell(&root, name),
        LanguageId::C => c(&root, name),
        LanguageId::Cpp => cpp(&root, name),
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

fn typescript(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let package = module_name(name);
    let manifest = format!(
        "{{\n  \"name\": \"{package}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"scripts\": {{\n    \"build\": \"tsc\",\n    \"start\": \"node dist/index.js\"\n  }},\n  \"devDependencies\": {{\n    \"typescript\": \"^5.0.0\"\n  }}\n}}\n"
    );
    let tsconfig = "{\n  \"compilerOptions\": {\n    \"target\": \"ES2022\",\n    \"module\": \"NodeNext\",\n    \"moduleResolution\": \"NodeNext\",\n    \"outDir\": \"dist\",\n    \"rootDir\": \"src\",\n    \"strict\": true,\n    \"esModuleInterop\": true,\n    \"skipLibCheck\": true\n  },\n  \"include\": [\"src\"]\n}\n";
    let index = format!(
        "function main(): void {{\n  console.log(\"Hello from {name}!\");\n}}\n\nmain();\n"
    );
    Ok(vec![
        write(root, "package.json", &manifest)?,
        write(root, "tsconfig.json", tsconfig)?,
        write(root, "src/index.ts", &index)?,
        write(root, ".gitignore", "node_modules\ndist\n")?,
    ])
}

fn javascript(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let package = module_name(name);
    let manifest = format!(
        "{{\n  \"name\": \"{package}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"scripts\": {{\n    \"start\": \"node src/index.js\"\n  }}\n}}\n"
    );
    let index =
        format!("function main() {{\n  console.log(\"Hello from {name}!\");\n}}\n\nmain();\n");
    Ok(vec![
        write(root, "package.json", &manifest)?,
        write(root, "src/index.js", &index)?,
        write(root, ".gitignore", "node_modules\n")?,
    ])
}

fn c(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let project = crate_name(name);
    let cmake = format!(
        "cmake_minimum_required(VERSION 3.16)\nproject({project} C)\n\nset(CMAKE_C_STANDARD 17)\nset(CMAKE_C_STANDARD_REQUIRED ON)\n\nadd_executable({project} src/main.c)\n"
    );
    let main = "#include <stdio.h>\n\nint main(void) {\n    printf(\"Hello from your new project!\\n\");\n    return 0;\n}\n";
    Ok(vec![
        write(root, "CMakeLists.txt", &cmake)?,
        write(root, "src/main.c", main)?,
        write(root, ".gitignore", "build/\n")?,
    ])
}

fn cpp(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let project = crate_name(name);
    let cmake = format!(
        "cmake_minimum_required(VERSION 3.16)\nproject({project} CXX)\n\nset(CMAKE_CXX_STANDARD 20)\nset(CMAKE_CXX_STANDARD_REQUIRED ON)\n\nadd_executable({project} src/main.cpp)\n"
    );
    let main = "#include <iostream>\n\nint main() {\n    std::cout << \"Hello from your new project!\\n\";\n    return 0;\n}\n";
    Ok(vec![
        write(root, "CMakeLists.txt", &cmake)?,
        write(root, "src/main.cpp", main)?,
        write(root, ".gitignore", "build/\n")?,
    ])
}

fn java(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let artifact = module_name(name);
    let package = python_module(name);
    let pom = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <project xmlns=\"http://maven.apache.org/POM/4.0.0\"\n\
         \x20        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n\
         \x20        xsi:schemaLocation=\"http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd\">\n\
         \x20 <modelVersion>4.0.0</modelVersion>\n\
         \x20 <groupId>com.example</groupId>\n\
         \x20 <artifactId>{artifact}</artifactId>\n\
         \x20 <version>0.1.0</version>\n\
         \x20 <properties>\n\
         \x20   <maven.compiler.release>21</maven.compiler.release>\n\
         \x20   <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>\n\
         \x20 </properties>\n\
         </project>\n"
    );
    let app = format!(
        "package com.example.{package};\n\npublic final class App {{\n    public static void main(String[] args) {{\n        System.out.println(\"Hello from {name}!\");\n    }}\n}}\n"
    );
    let path = format!("src/main/java/com/example/{package}/App.java");
    Ok(vec![
        write(root, "pom.xml", &pom)?,
        write(root, &path, &app)?,
        write(root, ".gitignore", "target/\n*.class\n")?,
    ])
}

fn csharp(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let project = crate_name(name);
    let csproj = "<Project Sdk=\"Microsoft.NET.Sdk\">\n\
         \x20 <PropertyGroup>\n\
         \x20   <OutputType>Exe</OutputType>\n\
         \x20   <TargetFramework>net10.0</TargetFramework>\n\
         \x20   <ImplicitUsings>enable</ImplicitUsings>\n\
         \x20   <Nullable>enable</Nullable>\n\
         \x20 </PropertyGroup>\n\
         </Project>\n";
    let program = format!("Console.WriteLine(\"Hello from {name}!\");\n");
    Ok(vec![
        write(root, &format!("{project}.csproj"), csproj)?,
        write(root, "Program.cs", &program)?,
        write(root, ".gitignore", "bin/\nobj/\n")?,
    ])
}

fn php(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let package = crate_name(name);
    let composer = format!(
        "{{\n  \"name\": \"koda/{package}\",\n  \"description\": \"A new project created with Koda\",\n  \"type\": \"project\",\n  \"require\": {{}}\n}}\n"
    );
    let index = format!(
        "<?php\n\ndeclare(strict_types=1);\n\nfunction main(): void\n{{\n    echo \"Hello from {name}!\\n\";\n}}\n\nmain();\n"
    );
    Ok(vec![
        write(root, "composer.json", &composer)?,
        write(root, "index.php", &index)?,
        write(root, ".gitignore", "/vendor/\n")?,
    ])
}

fn lua(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let script = format!(
        "local M = {{}}\n\nfunction M.greet()\n  print(\"Hello from {name}!\")\nend\n\nreturn M\n"
    );
    let config = "{\n  \"runtime.version\": \"Lua 5.4\"\n}\n";
    Ok(vec![
        write(root, "init.lua", &script)?,
        write(root, ".luarc.json", config)?,
        write(root, ".gitignore", "*.luac\n")?,
    ])
}

fn html(root: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let page = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    <title>{name}</title>\n    <link rel=\"stylesheet\" href=\"style.css\" />\n  </head>\n  <body>\n    <main class=\"card\">\n      <h1>Hello from {name}!</h1>\n      <p>Edit <code>index.html</code> to get started.</p>\n    </main>\n  </body>\n</html>\n"
    );
    let stylesheet = ":root {\n  color-scheme: light dark;\n}\n\nbody {\n  margin: 0;\n  font-family: system-ui, sans-serif;\n  display: grid;\n  place-items: center;\n  min-height: 100vh;\n}\n\n.card {\n  padding: 2rem;\n  border-radius: 0.75rem;\n  border: 1px solid #8884;\n}\n";
    Ok(vec![
        write(root, "index.html", &page)?,
        write(root, "style.css", stylesheet)?,
    ])
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
    fn rejects_names_unsafe_for_generated_files() {
        // These characters would break out of the string literals or shell
        // snippets the templates interpolate the name into.
        for bad in ["a\"b", "a$b", "a`b", "<tag>", "a&b", "a\\b"] {
            assert!(validate_name(bad).is_err(), "`{bad}` should be rejected");
        }
        // Ordinary punctuation and unicode remain allowed.
        assert!(validate_name("my.project").is_ok());
        assert!(validate_name("café-münchen").is_ok());
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
    fn scaffolds_typescript_and_javascript() {
        let dir = scratch("web-projects");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();

        let CreateOutcome::Created { root, .. } =
            create(&parent, "My Web App", LanguageId::TypeScript)
        else {
            panic!("expected TypeScript success");
        };
        assert!(root.join("package.json").is_file());
        assert!(root.join("tsconfig.json").is_file());
        assert!(root.join("src/index.ts").is_file());
        let manifest = std::fs::read_to_string(root.join("package.json")).unwrap();
        assert!(manifest.contains("\"name\": \"my-web-app\""));

        let CreateOutcome::Created { root, .. } =
            create(&parent, "my-js-app", LanguageId::JavaScript)
        else {
            panic!("expected JavaScript success");
        };
        assert!(root.join("src/index.js").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_c_and_cpp() {
        let dir = scratch("c-projects");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();

        let CreateOutcome::Created { root, .. } = create(&parent, "so cool", LanguageId::C) else {
            panic!("expected C success");
        };
        assert!(root.join("CMakeLists.txt").is_file());
        assert!(root.join("src/main.c").is_file());
        let cmake = std::fs::read_to_string(root.join("CMakeLists.txt")).unwrap();
        assert!(cmake.contains("project(so-cool C)"));

        let CreateOutcome::Created { root, .. } = create(&parent, "app", LanguageId::Cpp) else {
            panic!("expected C++ success");
        };
        assert!(root.join("src/main.cpp").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_java_and_csharp() {
        let dir = scratch("jvm-dotnet");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();

        let CreateOutcome::Created { root, .. } = create(&parent, "My Service", LanguageId::Java)
        else {
            panic!("expected Java success");
        };
        assert!(root.join("pom.xml").is_file());
        assert!(
            root.join("src/main/java/com/example/my_service/App.java")
                .is_file()
        );
        let pom = std::fs::read_to_string(root.join("pom.xml")).unwrap();
        assert!(pom.contains("<artifactId>my-service</artifactId>"));
        assert!(pom.contains("maven.compiler.release>21"));

        let CreateOutcome::Created { root, .. } = create(&parent, "My App", LanguageId::CSharp)
        else {
            panic!("expected C# success");
        };
        assert!(root.join("my-app.csproj").is_file());
        assert!(root.join("Program.cs").is_file());
        let csproj = std::fs::read_to_string(root.join("my-app.csproj")).unwrap();
        assert!(csproj.contains("<TargetFramework>net10.0</TargetFramework>"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_html() {
        let dir = scratch("html");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let CreateOutcome::Created { root, .. } = create(&parent, "My Site", LanguageId::Html)
        else {
            panic!("expected HTML success");
        };
        assert!(root.join("index.html").is_file());
        assert!(root.join("style.css").is_file());
        let page = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(page.contains("<link rel=\"stylesheet\" href=\"style.css\""));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_php() {
        let dir = scratch("php");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let CreateOutcome::Created { root, .. } = create(&parent, "My Service", LanguageId::Php)
        else {
            panic!("expected PHP success");
        };
        assert!(root.join("composer.json").is_file());
        assert!(root.join("index.php").is_file());
        let index = std::fs::read_to_string(root.join("index.php")).unwrap();
        assert!(index.starts_with("<?php"));
        assert!(index.contains("Hello from My Service!"));
        let composer = std::fs::read_to_string(root.join("composer.json")).unwrap();
        assert!(composer.contains("\"name\": \"koda/my-service\""));
        // The generated project is recognised as a PHP project by its marker.
        assert_eq!(crate::project::ProjectKind::Php.language(), LanguageId::Php);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaffolds_lua() {
        let dir = scratch("lua");
        let parent = dir.join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        let CreateOutcome::Created { root, .. } = create(&parent, "my mod", LanguageId::Lua) else {
            panic!("expected Lua success");
        };
        assert!(root.join("init.lua").is_file());
        assert!(root.join(".luarc.json").is_file());
        let script = std::fs::read_to_string(root.join("init.lua")).unwrap();
        assert!(script.contains("Hello from my mod!"));
        assert_eq!(crate::project::ProjectKind::Lua.language(), LanguageId::Lua);
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
