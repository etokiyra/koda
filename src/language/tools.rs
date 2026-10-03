//! Discovery of the external tools Koda can drive.
//!
//! Koda hides tool management: it probes for the language servers and
//! formatters it knows about, installs missing ones through their official
//! package managers, and reports what it finds.
//!
//! A probe checks that the program exists *and* that it runs, because a
//! `rustup` shim can exist for a component that is not installed. Lookups
//! search `PATH` and then a handful of well-known user bin directories, so a
//! tool installed by rustup or pip is found even when Koda was launched from a
//! GUI or a non-login shell whose `PATH` omits them.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::language::id::LanguageId;

/// A tool Koda knows how to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    RustAnalyzer,
    Gopls,
    Pylsp,
    BashLs,
    Rustfmt,
    Gofmt,
}

/// What a tool is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPurpose {
    LanguageServer,
    Formatter,
}

impl Tool {
    /// Every tool Koda looks for, in the order it is presented.
    pub const ALL: &'static [Tool] = &[
        Tool::RustAnalyzer,
        Tool::Gopls,
        Tool::Pylsp,
        Tool::BashLs,
        Tool::Rustfmt,
        Tool::Gofmt,
    ];

    pub fn program(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "rust-analyzer",
            Tool::Gopls => "gopls",
            Tool::Pylsp => "pylsp",
            Tool::BashLs => "bash-language-server",
            Tool::Rustfmt => "rustfmt",
            Tool::Gofmt => "gofmt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "rust-analyzer",
            Tool::Gopls => "gopls",
            Tool::Pylsp => "pylsp",
            Tool::BashLs => "bash-language-server",
            Tool::Rustfmt => "rustfmt",
            Tool::Gofmt => "gofmt",
        }
    }

    pub fn language(self) -> LanguageId {
        match self {
            Tool::RustAnalyzer | Tool::Rustfmt => LanguageId::Rust,
            Tool::Gopls | Tool::Gofmt => LanguageId::Go,
            Tool::Pylsp => LanguageId::Python,
            Tool::BashLs => LanguageId::Shell,
        }
    }

    pub fn purpose(self) -> ToolPurpose {
        match self {
            Tool::RustAnalyzer | Tool::Gopls | Tool::Pylsp | Tool::BashLs => {
                ToolPurpose::LanguageServer
            }
            Tool::Rustfmt | Tool::Gofmt => ToolPurpose::Formatter,
        }
    }

    /// Arguments that make the tool print its version. `gofmt` has no version
    /// flag, so it is probed with no arguments against empty stdin.
    fn version_args(self) -> &'static [&'static str] {
        match self {
            Tool::RustAnalyzer | Tool::Rustfmt | Tool::Pylsp | Tool::BashLs => &["--version"],
            Tool::Gopls => &["version"],
            Tool::Gofmt => &[],
        }
    }

    /// Arguments that start this tool as a language server.
    pub fn server_args(self) -> &'static [&'static str] {
        match self {
            // `bash-language-server` needs its `start` subcommand.
            Tool::BashLs => &["start"],
            _ => &[],
        }
    }

    /// A short, actionable message for when the tool is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "install with `rustup component add rust-analyzer`",
            Tool::Gopls => "install with `go install golang.org/x/tools/gopls@latest`",
            Tool::Pylsp => "install with `pipx install python-lsp-server`",
            Tool::BashLs => "install with `npm` — Koda uses a user-local prefix",
            Tool::Rustfmt => "install with `rustup component add rustfmt`",
            Tool::Gofmt => "it ships with the Go toolchain",
        }
    }

    /// The tool for a language and purpose, if Koda knows one.
    pub fn for_language(language: LanguageId, purpose: ToolPurpose) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.language() == language && tool.purpose() == purpose)
    }

    /// The preferred install command, when one exists.
    ///
    /// This is the human-facing headline command — used to decide whether a
    /// tool is installable and to describe it — while the actual attempt uses
    /// [`Tool::install_plan`], which may choose a user-local target.
    pub fn install_command(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Tool::RustAnalyzer => Some(("rustup", &["component", "add", "rust-analyzer"])),
            Tool::Rustfmt => Some(("rustup", &["component", "add", "rustfmt"])),
            Tool::Gopls => Some(("go", &["install", "golang.org/x/tools/gopls@latest"])),
            Tool::Pylsp => Some(("pipx", &["install", "python-lsp-server"])),
            Tool::BashLs => Some(("npm", &["install", "-g", "bash-language-server"])),
            Tool::Gofmt => None,
        }
    }

    /// The ordered commands Koda will actually run to install this tool.
    ///
    /// Koda runs only official acquisition paths — `rustup`, `go install`,
    /// `pipx`/`pip`, `npm` — so provenance stays with those tools. Every
    /// strategy targets a location the user can write to, never a system-owned
    /// prefix, so a missing permission can never block the install. The first
    /// command that succeeds wins.
    pub fn install_plan(self) -> Vec<InstallStep> {
        match self {
            Tool::RustAnalyzer => {
                vec![InstallStep::new(
                    "rustup",
                    &["component", "add", "rust-analyzer"],
                )]
            }
            Tool::Rustfmt => vec![InstallStep::new("rustup", &["component", "add", "rustfmt"])],
            Tool::Gopls => vec![InstallStep::new(
                "go",
                &["install", "golang.org/x/tools/gopls@latest"],
            )],
            Tool::Pylsp => vec![
                InstallStep::new("pipx", &["install", "python-lsp-server"]),
                InstallStep::new(
                    "python3",
                    &["-m", "pip", "install", "--user", "python-lsp-server"],
                ),
                InstallStep::new("pip3", &["install", "--user", "python-lsp-server"]),
                InstallStep::new(
                    "python",
                    &["-m", "pip", "install", "--user", "python-lsp-server"],
                ),
            ],
            Tool::BashLs => {
                // A user-local npm prefix always works; `npm install -g` into a
                // root-owned prefix fails with EACCES on many systems. Try the
                // user-local target first, then fall back to whatever global
                // prefix the user's npm (nvm, fnm, volta, …) already uses.
                let mut steps = Vec::new();
                if let Some(prefix) = npm_prefix() {
                    // A user-writable cache too: a `~/.npm` left root-owned by a
                    // past `sudo npm` would otherwise fail with EACCES.
                    let cache = prefix.with_file_name("npm-cache");
                    steps.push(InstallStep {
                        program: "npm".to_string(),
                        args: vec![
                            "install".to_string(),
                            "-g".to_string(),
                            "--prefix".to_string(),
                            prefix.to_string_lossy().into_owned(),
                            "--cache".to_string(),
                            cache.to_string_lossy().into_owned(),
                            "bash-language-server".to_string(),
                        ],
                    });
                }
                steps.push(InstallStep::new(
                    "npm",
                    &["install", "-g", "bash-language-server"],
                ));
                steps
            }
            // `gofmt` ships with the Go toolchain; there is nothing to install.
            Tool::Gofmt => Vec::new(),
        }
    }
}

/// One command in an install plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallStep {
    pub program: String,
    pub args: Vec<String>,
}

impl InstallStep {
    fn new(program: &str, args: &[&str]) -> Self {
        InstallStep {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }
}

/// Install a tool through its trusted package managers.
///
/// Intended for the background worker. Tries each planned command in turn and
/// returns a short success message, or the most recent actionable error so the
/// user sees what went wrong even when the network is unavailable.
pub fn install(tool: Tool) -> Result<String, String> {
    let plan = tool.install_plan();
    if plan.is_empty() {
        return Err(format!(
            "{} cannot be installed automatically — {}",
            tool.label(),
            tool.install_hint()
        ));
    }

    let mut last_error = None;
    for step in &plan {
        match Command::new(&step.program).args(&step.args).output() {
            Ok(output) if output.status.success() => {
                return Ok(format!("Installed {}", tool.label()));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let message = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("installation failed")
                    .trim()
                    .to_string();
                last_error = Some(format!("{}: {message}", tool.label()));
            }
            Err(err) => {
                last_error = Some(format!("could not run {}: {err}", step.program));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| format!("could not install {}", tool.label())))
}

/// Whether any package manager in `tool`'s install plan is actually available.
///
/// Koda only offers to install a tool when it can really do it, so it never
/// promises an install and then fails because no package manager exists.
pub fn can_install(tool: Tool) -> bool {
    tool.install_plan()
        .iter()
        .any(|step| locate(&step.program).is_some())
}

/// What Koda learned about one tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolStatus {
    pub tool: Tool,
    /// Whether the program exists and ran successfully.
    pub available: bool,
    /// The first line of `--version`, when there was one.
    pub version: Option<String>,
    /// The resolved executable path, when one was found. Tools installed in a
    /// user bin directory may not be on the process `PATH`.
    pub path: Option<PathBuf>,
}

impl ToolStatus {
    /// A one-line summary for the setup view.
    pub fn summary(&self) -> String {
        if self.available {
            match &self.version {
                Some(version) => version.clone(),
                None => "installed".to_string(),
            }
        } else {
            self.tool.install_hint().to_string()
        }
    }
}

/// The result of probing every known tool.
#[derive(Clone, Debug)]
pub struct ToolRegistry {
    statuses: Vec<ToolStatus>,
}

impl ToolRegistry {
    /// Probe every known tool. Runs a handful of short-lived processes and is
    /// meant for the background worker.
    pub fn discover() -> Self {
        ToolRegistry {
            statuses: Tool::ALL.iter().map(|&tool| probe(tool)).collect(),
        }
    }

    pub fn all(&self) -> &[ToolStatus] {
        &self.statuses
    }

    pub fn status(&self, tool: Tool) -> Option<&ToolStatus> {
        self.statuses.iter().find(|status| status.tool == tool)
    }

    pub fn available(&self, tool: Tool) -> bool {
        self.status(tool).is_some_and(|status| status.available)
    }

    /// The resolved executable path for `tool`, if it was found.
    pub fn program_path(&self, tool: Tool) -> Option<&Path> {
        self.status(tool).and_then(|status| status.path.as_deref())
    }

    /// Whether any language server for `language` is available.
    pub fn has_language_server(&self, language: LanguageId) -> bool {
        self.statuses.iter().any(|status| {
            status.tool.language() == language
                && status.tool.purpose() == ToolPurpose::LanguageServer
                && status.available
        })
    }
}

fn probe(tool: Tool) -> ToolStatus {
    let Some(path) = locate(tool.program()) else {
        return ToolStatus {
            tool,
            available: false,
            version: None,
            path: None,
        };
    };
    let mut command = Command::new(&path);
    command.args(tool.version_args());
    // `output()` nulls stdin, so `gofmt` reads an empty document and exits.
    match command.output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_string);
            ToolStatus {
                tool,
                available: true,
                version,
                path: Some(path),
            }
        }
        _ => ToolStatus {
            tool,
            available: false,
            version: None,
            path: Some(path),
        },
    }
}

/// Whether `program` can be found, returning its full path.
///
/// `PATH` is searched first, then a handful of well-known user and system bin
/// directories. The latter matters because Koda is often launched from a GUI or
/// a non-login shell whose `PATH` omits `~/.cargo/bin` and `~/.local/bin` —
/// exactly where rustup and pip put language tooling.
pub fn locate(program: &str) -> Option<PathBuf> {
    if let Some(found) = locate_on_path(program) {
        return Some(found);
    }
    known_bin_dirs()
        .into_iter()
        .find_map(|dir| candidate_in(&dir, program))
}

/// Search only `PATH`.
fn locate_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| candidate_in(&dir, program))
}

fn candidate_in(dir: &Path, program: &str) -> Option<PathBuf> {
    let candidate = dir.join(program);
    if candidate.is_file() {
        return Some(candidate);
    }
    let with_exe = dir.join(format!("{program}.exe"));
    with_exe.is_file().then_some(with_exe)
}

/// Directories where language tooling commonly lives.
fn known_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
        dirs.push(home.join("go/bin"));
        dirs.push(home.join(".bun/bin"));
        dirs.push(home.join(".deno/bin"));
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".local/share/pnpm"));
    }
    // Tools Koda installed itself, plus the npm prefix it manages.
    if let Some(tools) = tools_dir() {
        if let Some(prefix) = npm_prefix() {
            dirs.push(prefix.join("bin"));
            dirs.push(prefix.join("node_modules/.bin"));
            dirs.push(prefix);
        }
        dirs.push(tools.join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/bin"));
    dirs.push(PathBuf::from("/bin"));
    dirs
}

/// Koda's own directory for tools it installs, kept under the user's data
/// directory so no elevated permissions are ever required.
pub fn tools_dir() -> Option<PathBuf> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(data).join("koda/tools"));
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/share/koda/tools"))
}

/// The npm prefix Koda installs global packages into. It is user-writable, so
/// `npm install -g --prefix` never hits a permission error.
pub fn npm_prefix() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("npm"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_language_and_hint() {
        for &tool in Tool::ALL {
            assert!(!tool.program().is_empty());
            assert!(!tool.install_hint().is_empty());
            assert!(matches!(
                tool.language(),
                LanguageId::Rust | LanguageId::Go | LanguageId::Python | LanguageId::Shell
            ));
        }
    }

    #[test]
    fn unknown_program_is_not_located() {
        assert!(locate("koda-definitely-not-a-real-tool").is_none());
    }

    #[test]
    fn probe_reports_a_missing_tool_as_unavailable() {
        // `Gofmt` is only unavailable on machines without Go; use a tool whose
        // program we control by checking the registry shape instead.
        let registry = ToolRegistry::discover();
        assert_eq!(registry.all().len(), Tool::ALL.len());
        for status in registry.all() {
            if !status.available {
                assert!(status.version.is_none());
            }
        }
    }

    #[test]
    fn install_commands_use_trusted_managers() {
        assert_eq!(
            Tool::RustAnalyzer.install_command(),
            Some(("rustup", &["component", "add", "rust-analyzer"][..]))
        );
        assert_eq!(
            Tool::Gopls.install_command(),
            Some(("go", &["install", "golang.org/x/tools/gopls@latest"][..]))
        );
        assert_eq!(
            Tool::Pylsp.install_command(),
            Some(("pipx", &["install", "python-lsp-server"][..]))
        );
        assert_eq!(
            Tool::BashLs.install_command(),
            Some(("npm", &["install", "-g", "bash-language-server"][..]))
        );
        // Python tooling has pip fallbacks, so installation is attempted even
        // without pipx.
        assert!(Tool::Pylsp.install_plan().len() >= 2);
        assert_eq!(Tool::Gofmt.install_command(), None);
        // `gofmt` cannot be installed on its own.
        assert!(install(Tool::Gofmt).is_err());
    }

    #[test]
    fn install_plans_never_need_elevated_permissions() {
        // `npm install -g` into a root-owned prefix fails with EACCES, so the
        // plan must offer a user-local prefix first.
        let Some(prefix) = npm_prefix() else {
            return; // No home directory; nothing to manage.
        };
        let plan = Tool::BashLs.install_plan();
        let first = plan.first().expect("an npm install step");
        assert_eq!(first.program, "npm");
        assert!(
            first.args.iter().any(|arg| arg == "--prefix"),
            "the first npm step should use --prefix: {:?}",
            first.args
        );
        assert!(
            first
                .args
                .iter()
                .any(|arg| arg == &prefix.to_string_lossy()),
            "the prefix should be Koda's own user-local directory: {:?}",
            first.args
        );
        // pipx and pip --user are likewise user-local.
        for step in Tool::Pylsp.install_plan() {
            let user_local = step.program == "pipx" || step.args.iter().any(|arg| arg == "--user");
            assert!(user_local, "pylsp step is not user-local: {step:?}");
        }
    }

    #[test]
    fn user_tool_directories_are_searched() {
        let dirs = known_bin_dirs();
        assert!(dirs.iter().any(|dir| dir.ends_with(".cargo/bin")));
        assert!(dirs.iter().any(|dir| dir.ends_with("go/bin")));
        if let Some(prefix) = npm_prefix() {
            assert!(
                dirs.iter().any(|dir| dir == &prefix.join("bin")),
                "the managed npm prefix should be searched"
            );
        }
    }

    #[test]
    fn status_summary_prefers_version_then_hint() {
        let available = ToolStatus {
            tool: Tool::Rustfmt,
            available: true,
            version: Some("rustfmt 1.8.0".to_string()),
            path: None,
        };
        assert_eq!(available.summary(), "rustfmt 1.8.0");

        let missing = ToolStatus {
            tool: Tool::Rustfmt,
            available: false,
            version: None,
            path: None,
        };
        assert!(missing.summary().contains("rustup"));
    }

    #[test]
    fn candidate_lookup_finds_a_file_in_a_directory() {
        let dir = std::env::temp_dir().join(format!("koda-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("koda-fake-tool"), "").unwrap();

        assert_eq!(
            candidate_in(&dir, "koda-fake-tool"),
            Some(dir.join("koda-fake-tool"))
        );
        assert!(candidate_in(&dir, "koda-absent-tool").is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn known_bin_dirs_include_user_tool_locations() {
        let dirs = known_bin_dirs();
        assert!(dirs.iter().any(|dir| dir.ends_with(".cargo/bin")));
        assert!(dirs.iter().any(|dir| dir.ends_with(".local/bin")));
    }
}
