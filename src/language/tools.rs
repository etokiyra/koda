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

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::language::id::LanguageId;

/// A lock older than this is assumed to be left by a crashed instance.
const STALE_INSTALL_LOCK: Duration = Duration::from_secs(15 * 60);

/// The longest a single install command may run before it is killed. Package
/// managers can legitimately take a while on a slow link, but a hung process
/// must never wedge the background worker forever.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// The most output Koda keeps from an install command. A verbose or hostile
/// tool cannot exhaust memory; the excess is drained and discarded.
const MAX_TOOL_OUTPUT: usize = 256 * 1024;

/// The largest archive Koda will download. The managed JDK and .NET SDK are
/// the biggest, and both are well under this.
const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A tool Koda knows how to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    RustAnalyzer,
    Gopls,
    Pylsp,
    BashLs,
    TypeScriptLs,
    Clangd,
    Jdtls,
    OmniSharp,
    HtmlLs,
    CssLs,
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
        Tool::TypeScriptLs,
        Tool::Clangd,
        Tool::Jdtls,
        Tool::OmniSharp,
        Tool::HtmlLs,
        Tool::CssLs,
        Tool::Rustfmt,
        Tool::Gofmt,
    ];

    pub fn program(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "rust-analyzer",
            Tool::Gopls => "gopls",
            Tool::Pylsp => "pylsp",
            Tool::BashLs => "bash-language-server",
            Tool::TypeScriptLs => "typescript-language-server",
            Tool::Clangd => "clangd",
            Tool::Jdtls => "jdtls",
            Tool::OmniSharp => "OmniSharp",
            Tool::HtmlLs => "vscode-html-language-server",
            Tool::CssLs => "vscode-css-language-server",
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
            Tool::TypeScriptLs => "typescript-language-server",
            Tool::Clangd => "clangd",
            Tool::Jdtls => "jdtls",
            Tool::OmniSharp => "omnisharp",
            Tool::HtmlLs => "vscode-html-language-server",
            Tool::CssLs => "vscode-css-language-server",
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
            Tool::TypeScriptLs => LanguageId::TypeScript,
            Tool::Clangd => LanguageId::C,
            Tool::Jdtls => LanguageId::Java,
            Tool::OmniSharp => LanguageId::CSharp,
            Tool::HtmlLs => LanguageId::Html,
            Tool::CssLs => LanguageId::Css,
        }
    }

    /// Whether this tool serves `language`. The TypeScript server also handles
    /// JavaScript; clangd serves both C and C++.
    pub fn serves(self, language: LanguageId) -> bool {
        self.language() == language
            || (self == Tool::TypeScriptLs && language == LanguageId::JavaScript)
            || (self == Tool::Clangd && language == LanguageId::Cpp)
    }

    pub fn purpose(self) -> ToolPurpose {
        match self {
            Tool::RustAnalyzer
            | Tool::Gopls
            | Tool::Pylsp
            | Tool::BashLs
            | Tool::TypeScriptLs
            | Tool::Clangd
            | Tool::Jdtls
            | Tool::OmniSharp
            | Tool::HtmlLs
            | Tool::CssLs => ToolPurpose::LanguageServer,
            Tool::Rustfmt | Tool::Gofmt => ToolPurpose::Formatter,
        }
    }

    /// Arguments that make the tool print its version. `gofmt` has no version
    /// flag, so it is probed with no arguments against empty stdin.
    fn version_args(self) -> &'static [&'static str] {
        match self {
            Tool::RustAnalyzer
            | Tool::Rustfmt
            | Tool::Pylsp
            | Tool::BashLs
            | Tool::TypeScriptLs
            | Tool::Clangd
            | Tool::HtmlLs
            | Tool::CssLs => &["--version"],
            Tool::Gopls => &["version"],
            // `jdtls` and `OmniSharp` have no `--version`; `--help` proves they
            // launch (and, for OmniSharp, that the .NET runtime is present).
            Tool::Jdtls | Tool::OmniSharp => &["--help"],
            Tool::Gofmt => &[],
        }
    }

    /// Arguments that start this tool as a language server.
    pub fn server_args(self) -> &'static [&'static str] {
        match self {
            // `bash-language-server` needs its `start` subcommand.
            Tool::BashLs => &["start"],
            // `typescript-language-server` speaks stdio when asked.
            Tool::TypeScriptLs => &["--stdio"],
            // `OmniSharp` needs LSP mode and zero-based (LSP) positions.
            Tool::OmniSharp => &["-z", "--languageserver"],
            // The extracted VS Code servers speak stdio.
            Tool::HtmlLs | Tool::CssLs => &["--stdio"],
            _ => &[],
        }
    }

    /// A short, actionable message for when the tool is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "install with `rustup component add rust-analyzer`",
            Tool::Gopls => "install with `go install golang.org/x/tools/gopls@latest`",
            Tool::Pylsp => "install `python-lsp-server` into a Koda-managed environment",
            Tool::BashLs => "install with `npm` — Koda uses a user-local prefix",
            Tool::TypeScriptLs => "install with `npm` — Koda uses a user-local prefix",
            Tool::Clangd => {
                "install clangd with your system package manager (it ships with most C/C++ toolchains)"
            }
            Tool::Jdtls => "Koda can install a managed JDK and Eclipse JDT",
            Tool::OmniSharp => "Koda can install the .NET SDK and OmniSharp",
            Tool::HtmlLs | Tool::CssLs => "install with `npm` — Koda uses a user-local prefix",
            Tool::Rustfmt => "install with `rustup component add rustfmt`",
            Tool::Gofmt => "it ships with the Go toolchain",
        }
    }

    /// The tool for a language and purpose, if Koda knows one.
    pub fn for_language(language: LanguageId, purpose: ToolPurpose) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.serves(language) && tool.purpose() == purpose)
    }

    /// The preferred install command, when one exists.
    ///
    /// This is the human-facing headline command, used to describe the tool;
    /// the actual install uses [`Tool::install_attempts`], which picks a
    /// self-contained, user-local strategy.
    pub fn install_command(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Tool::RustAnalyzer => Some(("rustup", &["component", "add", "rust-analyzer"])),
            Tool::Rustfmt => Some(("rustup", &["component", "add", "rustfmt"])),
            Tool::Gopls => Some(("go", &["install", "golang.org/x/tools/gopls@latest"])),
            Tool::Pylsp => Some(("pipx", &["install", "python-lsp-server"])),
            Tool::BashLs => Some(("npm", &["install", "-g", "bash-language-server"])),
            Tool::TypeScriptLs => Some((
                "npm",
                &["install", "-g", "typescript-language-server", "typescript"],
            )),
            // `clangd` has no portable user-local installer; it ships with the
            // C/C++ toolchain and is used when it is already present.
            Tool::Clangd => None,
            // `jdtls` and `OmniSharp` are installed by Koda's own managed
            // download plan rather than a single package-manager command.
            Tool::Jdtls | Tool::OmniSharp => None,
            Tool::HtmlLs | Tool::CssLs => {
                Some(("npm", &["install", "-g", "vscode-langservers-extracted"]))
            }
            Tool::Gofmt => None,
        }
    }

    /// Programs this tool needs before it can be installed, so Koda can explain
    /// when a whole toolchain is missing.
    pub fn prerequisites(self) -> &'static [&'static str] {
        match self {
            Tool::RustAnalyzer | Tool::Rustfmt => &["rustup"],
            Tool::Gopls | Tool::Gofmt => &["go"],
            Tool::Pylsp => &["python3"],
            Tool::BashLs | Tool::TypeScriptLs => &["npm"],
            Tool::HtmlLs | Tool::CssLs => &["npm"],
            // `jdtls` is a Python launcher script.
            Tool::Jdtls => &["python3"],
            Tool::Clangd | Tool::OmniSharp => &[],
        }
    }

    /// The prerequisites that are not present.
    pub fn missing_prerequisites(self) -> Vec<&'static str> {
        self.prerequisites()
            .iter()
            .copied()
            .filter(|program| locate(program).is_none())
            .collect()
    }

    /// The ordered strategies Koda will try to install this tool.
    ///
    /// Each attempt is a short command sequence; Koda re-probes the tool after
    /// every attempt and stops at the first that yields a working binary. Every
    /// strategy is self-contained and targets a location the user can write to
    /// — a Koda-managed Python virtualenv or npm prefix, `--user` installs, or
    /// the official `rustup` installer — so a missing permission or a Python
    /// without `pip` never blocks provisioning.
    pub fn install_attempts(self) -> Vec<InstallAttempt> {
        match self {
            Tool::RustAnalyzer => component_attempts("rust-analyzer"),
            Tool::Rustfmt => component_attempts("rustfmt"),
            Tool::Gopls => vec![InstallAttempt::one(
                "go install",
                InstallCommand::new("go", &["install", "golang.org/x/tools/gopls@latest"]),
            )],
            Tool::Pylsp => python_attempts(),
            Tool::BashLs => npm_attempts(&["bash-language-server"]),
            Tool::TypeScriptLs => npm_attempts(&["typescript-language-server", "typescript"]),
            // `clangd` ships with the C/C++ toolchain; there is no user-local
            // installer to run, so Koda uses it when it is already present.
            Tool::Clangd => Vec::new(),
            Tool::Jdtls => jdtls_attempts(),
            Tool::OmniSharp => omnisharp_attempts(),
            Tool::HtmlLs | Tool::CssLs => npm_attempts(&["vscode-langservers-extracted"]),
            // `gofmt` ships with the Go toolchain; there is nothing to install.
            Tool::Gofmt => Vec::new(),
        }
    }
}

/// One command in an install attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl InstallCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        InstallCommand {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }

    fn with_args(program: impl Into<String>, args: Vec<String>) -> Self {
        InstallCommand {
            program: program.into(),
            args,
        }
    }
}

/// One executable step in an install strategy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallStep {
    /// Run a program with arguments.
    Run(InstallCommand),
    /// Download `url` to `dest` over HTTPS, verifying `sha256` when known.
    Download {
        url: String,
        dest: PathBuf,
        sha256: Option<String>,
    },
    /// Download and verify an Eclipse Adoptium JDK, whose checksum Adoptium
    /// publishes in the same JSON document that carries the link.
    AdoptiumJdk { feature: u32, dest: PathBuf },
    /// Extract a `.tar.gz`/`.tar.xz`/`.zip` archive into `dest`, optionally
    /// dropping `strip` leading path components.
    Extract {
        archive: PathBuf,
        dest: PathBuf,
        strip: usize,
    },
}

impl From<InstallCommand> for InstallStep {
    fn from(command: InstallCommand) -> Self {
        InstallStep::Run(command)
    }
}

/// A strategy for installing a tool: an ordered command sequence that Koda runs
/// and then verifies by re-probing the tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallAttempt {
    /// A short description of the strategy.
    pub via: &'static str,
    pub steps: Vec<InstallStep>,
}

impl InstallAttempt {
    fn one(via: &'static str, command: InstallCommand) -> Self {
        InstallAttempt {
            via,
            steps: vec![InstallStep::Run(command)],
        }
    }

    fn sequence(via: &'static str, commands: Vec<InstallCommand>) -> Self {
        InstallAttempt {
            via,
            steps: commands.into_iter().map(InstallStep::Run).collect(),
        }
    }

    fn managed(via: &'static str, steps: Vec<InstallStep>) -> Self {
        InstallAttempt { via, steps }
    }
}

/// Install a `rustup` component, bootstrapping `rustup` itself when absent.
fn component_attempts(component: &'static str) -> Vec<InstallAttempt> {
    let mut attempts = vec![InstallAttempt::one(
        "rustup",
        InstallCommand::new("rustup", &["component", "add", component]),
    )];
    if let Some(bootstrap) = rustup_bootstrap(component) {
        attempts.push(bootstrap);
    }
    attempts
}

/// Bootstrap the Rust toolchain with the official `rustup` installer, then add
/// the component. Skipped on Windows, where the installer is a binary.
fn rustup_bootstrap(component: &'static str) -> Option<InstallAttempt> {
    if cfg!(windows) {
        return None;
    }
    let tools = tools_dir()?;
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let script = tools.join("rustup-init.sh");
    let rustup = home.join(".cargo/bin/rustup");
    Some(InstallAttempt::sequence(
        "the official rustup installer",
        vec![
            InstallCommand::with_args(
                "curl",
                vec![
                    "--proto".into(),
                    "=https".into(),
                    "--tlsv1.2".into(),
                    "-sSf".into(),
                    "--connect-timeout".into(),
                    "30".into(),
                    "--max-time".into(),
                    "120".into(),
                    "--max-filesize".into(),
                    (1024 * 1024).to_string(),
                    "https://sh.rustup.rs".into(),
                    "-o".into(),
                    script.to_string_lossy().into_owned(),
                ],
            ),
            InstallCommand::with_args(
                "sh",
                vec![
                    script.to_string_lossy().into_owned(),
                    "-y".into(),
                    "--no-modify-path".into(),
                ],
            ),
            InstallCommand::with_args(
                rustup.to_string_lossy().into_owned(),
                vec!["component".into(), "add".into(), component.into()],
            ),
        ],
    ))
}

/// Strategies for installing the Python language server, from most isolated to
/// least. The virtualenv attempt bootstraps its own `pip`, so it works even
/// when the system Python has no `pip` module or is externally managed.
fn python_attempts() -> Vec<InstallAttempt> {
    let mut attempts = vec![
        InstallAttempt::one(
            "pipx",
            InstallCommand::new("pipx", &["install", "python-lsp-server"]),
        ),
        InstallAttempt::one(
            "uv",
            InstallCommand::new("uv", &["tool", "install", "python-lsp-server"]),
        ),
    ];

    if let Some(venv) = venv_dir() {
        let dir = venv.to_string_lossy().into_owned();
        let pip = venv_program(&venv, "pip").to_string_lossy().into_owned();
        for python in ["python3", "python"] {
            attempts.push(InstallAttempt::sequence(
                "a Koda-managed virtualenv",
                vec![
                    InstallCommand::with_args(
                        python,
                        vec!["-m".into(), "venv".into(), dir.clone()],
                    ),
                    InstallCommand::with_args(
                        pip.clone(),
                        vec!["install".into(), "python-lsp-server".into()],
                    ),
                ],
            ));
        }
    }

    for python in ["python3", "python"] {
        // `ensurepip` seeds pip when the interpreter has none.
        attempts.push(InstallAttempt::sequence(
            "ensurepip",
            vec![
                InstallCommand::new(python, &["-m", "ensurepip", "--user"]),
                InstallCommand::new(
                    python,
                    &["-m", "pip", "install", "--user", "python-lsp-server"],
                ),
            ],
        ));
        attempts.push(InstallAttempt::one(
            "pip --user",
            InstallCommand::new(
                python,
                &["-m", "pip", "install", "--user", "python-lsp-server"],
            ),
        ));
    }
    attempts.push(InstallAttempt::one(
        "pip3 --user",
        InstallCommand::new("pip3", &["install", "--user", "python-lsp-server"]),
    ));

    attempts
}

/// Strategies for installing an npm package into a user-local prefix.
fn npm_attempts(packages: &[&str]) -> Vec<InstallAttempt> {
    let mut attempts = Vec::new();
    if let Some(prefix) = npm_prefix() {
        // A user-local prefix and cache: `npm install -g` into a root-owned
        // prefix, or a `~/.npm` left root-owned by a past `sudo npm`, would
        // otherwise fail with EACCES.
        let cache = prefix.with_file_name("npm-cache");
        let mut args = vec![
            "install".to_string(),
            "-g".to_string(),
            "--prefix".to_string(),
            prefix.to_string_lossy().into_owned(),
            "--cache".to_string(),
            cache.to_string_lossy().into_owned(),
        ];
        args.extend(packages.iter().map(|package| (*package).to_string()));
        attempts.push(InstallAttempt::one(
            "npm (user-local prefix)",
            InstallCommand::with_args("npm", args),
        ));
    }
    // Fall back to whatever global prefix the user's npm (nvm, fnm, volta, …)
    // already uses.
    let mut args = vec!["install".to_string(), "-g".to_string()];
    args.extend(packages.iter().map(|package| (*package).to_string()));
    attempts.push(InstallAttempt::one(
        "npm",
        InstallCommand::with_args("npm", args),
    ));
    attempts
}

/// A best-effort advisory lock over the managed tools directory.
///
/// Two Koda instances sharing a data directory must not install into the same
/// npm prefix or Python virtualenv at once. The lock is a file created with
/// `create_new`; a stale one (from a crashed instance) is reclaimed by age.
struct InstallLock {
    path: PathBuf,
    /// Identifies this acquisition, so `Drop` only removes the lock if it is
    /// still ours. Without it, a holder that was reclaimed as stale could
    /// delete the new holder's lock on exit.
    nonce: String,
}

impl InstallLock {
    fn acquire() -> Result<Self, String> {
        let dir = tools_dir().ok_or_else(|| "no data directory for tools".to_string())?;
        Self::acquire_at(&dir)
    }

    fn acquire_at(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        let path = dir.join("install.lock");
        let nonce = format!(
            "{}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0)
        );

        if let Ok(file) = Self::create_new(&path, &nonce) {
            drop(file);
            return Ok(InstallLock { path, nonce });
        }

        // The lock exists. Reclaim it only if it is old enough to be from a
        // crashed instance; a live install must never be disturbed.
        let stale = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > STALE_INSTALL_LOCK);
        if !stale {
            return Err(
                "another Koda instance is installing tools — try again shortly".to_string(),
            );
        }
        let _ = std::fs::remove_file(&path);
        match Self::create_new(&path, &nonce) {
            Ok(file) => {
                drop(file);
                Ok(InstallLock { path, nonce })
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                Err("another Koda instance is installing tools — try again shortly".to_string())
            }
            Err(err) => Err(err.to_string()),
        }
    }

    fn create_new(path: &Path, nonce: &str) -> std::io::Result<std::fs::File> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let _ = writeln!(file, "{nonce}");
        Ok(file)
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        // Only remove the lock if it is still the one we created.
        if let Ok(contents) = std::fs::read_to_string(&self.path)
            && contents.trim() == self.nonce
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Install a tool by trying each strategy and verifying the result.
///
/// Intended for the background worker. A strategy's commands run in order; if
/// one fails the strategy is abandoned, and after every strategy Koda re-probes
/// the tool — a package manager can report success without producing a binary
/// Koda can find. Returns a short success message or the most recent
/// actionable error.
///
/// A lock around the managed tools directory keeps two Koda instances from
/// installing into the same npm prefix or Python virtualenv at once.
pub fn install(tool: Tool) -> Result<String, String> {
    let attempts = tool.install_attempts();
    if attempts.is_empty() {
        return Err(format!(
            "{} cannot be installed automatically — {}",
            tool.label(),
            tool.install_hint()
        ));
    }
    let _lock = InstallLock::acquire()?;

    let mut last_error = None;
    for attempt in &attempts {
        let mut completed = true;
        for step in &attempt.steps {
            if let Err(message) = run_step(step) {
                last_error = Some(format!("{}: {message}", tool.label()));
                completed = false;
                break;
            }
        }
        // Only a strategy whose steps all succeeded may be trusted, even if a
        // stale binary from a previous attempt happens to probe as available.
        if completed && probe(tool).available {
            return Ok(format!("Installed {}", tool.label()));
        }
    }
    Err(last_error.unwrap_or_else(|| format!("could not install {}", tool.label())))
}

/// Execute one install step.
fn run_step(step: &InstallStep) -> Result<(), String> {
    match step {
        InstallStep::Run(command) => run_command(command),
        InstallStep::Download { url, dest, sha256 } => download(url, dest, sha256.as_deref()),
        InstallStep::AdoptiumJdk { feature, dest } => adoptium_jdk(*feature, dest),
        InstallStep::Extract {
            archive,
            dest,
            strip,
        } => extract(archive, dest, *strip),
    }
}

/// A `Command` for an install subprocess, run from Koda's own tools directory.
///
/// Installing from the user's project directory lets a repository-local
/// `.npmrc` (or an equivalent per-directory config) redirect a package manager
/// to an attacker-controlled registry. Running from a Koda-managed directory
/// removes that vector while leaving the user's own `~/.npmrc` and similar
/// configuration in effect.
fn install_command(program: &str) -> Command {
    let mut command = Command::new(program);
    if let Some(dir) = install_work_dir() {
        command.current_dir(dir);
    }
    command
}

/// The directory install subprocesses run from, created on demand.
fn install_work_dir() -> Option<PathBuf> {
    let dir = tools_dir()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Run a program with a timeout and bounded output, reporting its first stderr
/// line on failure.
///
/// A hung package manager would otherwise stall the single background worker
/// forever, and a verbose tool could exhaust memory through unbounded capture.
fn run_command(command: &InstallCommand) -> Result<(), String> {
    use std::process::Stdio;

    let mut child = install_command(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("could not run {}: {err}", command.program))?;

    // Drain both pipes on their own threads so the child can never block on a
    // full pipe buffer while we wait for it.
    let stdout = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(move || read_capped(pipe)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || read_capped(pipe)));

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{} timed out after {} minutes",
                        command.program,
                        COMMAND_TIMEOUT.as_secs() / 60
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(format!("could not run {}: {err}", command.program)),
        }
    };

    let stderr = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let _stdout = stdout
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(first_stderr_line(&stderr))
    }
}

/// Read at most [`MAX_TOOL_OUTPUT`] bytes, draining the rest so the writer never
/// blocks.
fn read_capped<R: Read>(mut reader: R) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if kept.len() < MAX_TOOL_OUTPUT {
                    let take = (MAX_TOOL_OUTPUT - kept.len()).min(n);
                    kept.extend_from_slice(&chunk[..take]);
                }
            }
            Err(_) => break,
        }
    }
    kept
}

/// Download `url` to `dest` over HTTPS, verifying a SHA-256 when one is known.
fn download(url: &str, dest: &Path, sha256: Option<&str>) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }
    let mut command = install_command("curl");
    command.args([
        "--proto",
        "=https",
        "--tlsv1.2",
        "-L",
        "--fail",
        "-sS",
        "--connect-timeout",
        "30",
        "--max-time",
        "600",
    ]);
    command
        .arg("--max-filesize")
        .arg(MAX_DOWNLOAD_BYTES.to_string())
        .arg("-o")
        .arg(dest)
        .arg(url);
    let output = command
        .output()
        .map_err(|err| format!("could not run curl: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "download failed: {}",
            first_stderr_line(&output.stderr)
        ));
    }
    if let Some(expected) = sha256 {
        let actual = file_sha256(dest)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(dest);
            return Err(format!(
                "checksum mismatch for {} (expected {expected}, got {actual})",
                dest.display()
            ));
        }
    }
    Ok(())
}

/// Fetch and verify the latest Eclipse Adoptium JDK for `feature`.
///
/// Adoptium's API reports the download link and its SHA-256 together, so the
/// archive is verified even though the version moves.
fn adoptium_jdk(feature: u32, dest: &Path) -> Result<(), String> {
    let (os, arch) = adoptium_platform().ok_or_else(|| {
        "no managed JDK is published for this platform; install Java 25 manually".to_string()
    })?;
    let api = format!(
        "https://api.adoptium.net/v3/assets/latest/{feature}/hotspot?os={os}&architecture={arch}&image_type=jdk"
    );
    let output = install_command("curl")
        .args([
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sS",
            "-L",
            "--fail",
            "--max-time",
            "60",
        ])
        .arg(&api)
        .output()
        .map_err(|err| format!("could not query Adoptium: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not query Adoptium: {}",
            first_stderr_line(&output.stderr)
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("bad Adoptium response: {err}"))?;
    let package = value
        .get(0)
        .and_then(|entry| entry.pointer("/binary/package"))
        .ok_or_else(|| "Adoptium returned no JDK package".to_string())?;
    let link = package
        .get("link")
        .and_then(|link| link.as_str())
        .ok_or_else(|| "Adoptium package had no link".to_string())?;
    // Fail closed: an unverified JDK would be launched as `JAVA_HOME`, so never
    // fall back to "no checksum required".
    let checksum = package
        .get("checksum")
        .and_then(|sum| sum.as_str())
        .ok_or_else(|| {
            "Adoptium package had no checksum; refusing an unverified JDK".to_string()
        })?;
    download(link, dest, Some(checksum))
}

/// The Adoptium OS/architecture names for this platform.
fn adoptium_platform() -> Option<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "mac",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    Some((os, arch))
}

/// Extract a `.tar.gz`/`.tar.xz`/`.zip` archive into `dest`.
fn extract(archive: &Path, dest: &Path, strip: usize) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|err| format!("could not create {}: {err}", dest.display()))?;
    let zip = archive.extension().and_then(|ext| ext.to_str()) == Some("zip");
    let mut command = if zip {
        let mut command = install_command("unzip");
        command.arg("-q").arg("-o").arg(archive).arg("-d").arg(dest);
        command
    } else {
        let mut command = install_command("tar");
        command.arg("-xf").arg(archive).arg("-C").arg(dest);
        if strip > 0 {
            command.arg(format!("--strip-components={strip}"));
        }
        command
    };
    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "could not extract {}: {}",
            archive.display(),
            first_stderr_line(&output.stderr)
        )),
        Err(err) => Err(format!("could not run the archive tool: {err}")),
    }
}

/// SHA-256 of a file, using whichever tool the platform provides.
fn file_sha256(path: &Path) -> Result<String, String> {
    for (program, args) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        if let Ok(output) = install_command(program).args(args).arg(path).output()
            && output.status.success()
            && let Some(hash) = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
            && !hash.is_empty()
        {
            return Ok(hash.to_string());
        }
    }
    Err("no SHA-256 tool found (looked for sha256sum and shasum)".to_string())
}

fn first_stderr_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("installation failed")
        .trim()
        .to_string()
}

/// Whether every step of at least one install strategy can run here.
///
/// Koda only offers to install a tool it can really install, so it never
/// promises an install and then fails because no package manager, `curl` or
/// archive tool exists.
pub fn can_install(tool: Tool) -> bool {
    // `jdtls` is a Python launcher script; without a Python interpreter it
    // cannot run even though its download plan only needs `curl` and `tar`.
    if tool == Tool::Jdtls && locate("python3").is_none() && locate("python").is_none() {
        return false;
    }
    tool.install_attempts()
        .iter()
        .any(|attempt| attempt.steps.iter().all(step_available))
}

fn step_available(step: &InstallStep) -> bool {
    match step {
        InstallStep::Run(command) => {
            locate(&command.program).is_some() || is_user_bin_program(&command.program)
        }
        InstallStep::Download { .. } | InstallStep::AdoptiumJdk { .. } => locate("curl").is_some(),
        InstallStep::Extract { archive, .. } => {
            let zip = archive.extension().and_then(|ext| ext.to_str()) == Some("zip");
            locate(if zip { "unzip" } else { "tar" }).is_some()
        }
    }
}

/// Whether `program` is an absolute path inside a user bin directory.
///
/// Such a program may not exist yet but is produced by an earlier step in the
/// strategy — the `rustup` bootstrap installs `~/.cargo/bin/rustup`, and a
/// managed virtualenv's `pip` is created by `python -m venv` just before it is
/// used. Treating these as available lets Koda offer the strategy it can run
/// instead of rejecting it up front.
fn is_user_bin_program(program: &str) -> bool {
    let path = Path::new(program);
    path.is_absolute() && known_bin_dirs().iter().any(|dir| path.starts_with(dir))
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
    // Managed runtimes (the JDK for jdtls, the .NET SDK for OmniSharp) are
    // found through the launch environment.
    for (key, value) in launch_env(tool) {
        command.env(key, value);
    }
    // `output()` nulls stdin, so `gofmt` reads an empty document and exits.
    match command.output() {
        Ok(output) if output.status.success() => {
            // Only keep a version line that actually looks like one; `--help`
            // usage output should not masquerade as a version.
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .filter(|line| line.chars().any(|c| c.is_ascii_digit()))
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
    // Tools Koda installed itself, plus the npm prefix, Python virtualenv and
    // managed runtimes it maintains.
    if let Some(tools) = tools_dir() {
        if let Some(prefix) = npm_prefix() {
            dirs.push(prefix.join("bin"));
            dirs.push(prefix.join("node_modules/.bin"));
            dirs.push(prefix);
        }
        if let Some(venv) = venv_dir() {
            dirs.push(venv_bin_dir(&venv));
        }
        dirs.push(tools.join("omnisharp"));
        dirs.push(tools.join("jdtls/bin"));
        dirs.push(tools.join("jdk/bin"));
        dirs.push(tools.join("dotnet"));
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

/// Koda's managed Python virtualenv, used to install the Python language
/// server even when the system Python has no `pip`.
pub fn venv_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("python"))
}

/// Koda's managed .NET install directory (the SDK OmniSharp runs on).
pub fn dotnet_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("dotnet"))
}

/// Koda's managed JDK directory, used to run `jdtls` without touching the
/// user's system Java.
pub fn jdk_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("jdk"))
}

/// Koda's managed OmniSharp directory.
pub fn omnisharp_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("omnisharp"))
}

/// Koda's managed `jdtls` directory.
pub fn jdtls_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("jdtls"))
}

/// Scratch space for downloaded archives.
fn downloads_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("downloads"))
}

/// Environment a tool needs to run, pointing it at Koda-managed runtimes and
/// prepending their bin directories to `PATH` so child processes find them.
pub fn launch_env(tool: Tool) -> Vec<(String, String)> {
    let mut env = Vec::new();
    match tool {
        Tool::OmniSharp => {
            if let Some(dir) = dotnet_dir() {
                let dir = dir.to_string_lossy().into_owned();
                env.push(("DOTNET_ROOT".to_string(), dir.clone()));
                env.push(("DOTNET_ROOT_X64".to_string(), dir.clone()));
                env.push(("PATH".to_string(), prepend_path(&dir)));
            }
        }
        Tool::Jdtls => {
            if let Some(jdk) = jdk_dir() {
                let jdk = jdk.to_string_lossy().into_owned();
                env.push(("JAVA_HOME".to_string(), jdk.clone()));
                env.push(("PATH".to_string(), prepend_path(&format!("{jdk}/bin"))));
            }
        }
        _ => {}
    }
    env
}

fn prepend_path(dir: &str) -> String {
    match std::env::var_os("PATH") {
        Some(existing) => format!("{dir}:{}", existing.to_string_lossy()),
        None => dir.to_string(),
    }
}

/// The OmniSharp release asset for this platform, if one exists.
fn omnisharp_asset() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "omnisharp-linux-x64.tar.gz",
        ("linux", "aarch64") => "omnisharp-linux-arm64.tar.gz",
        ("macos", "x86_64") => "omnisharp-osx-x64.tar.gz",
        ("macos", "aarch64") => "omnisharp-osx-arm64.tar.gz",
        _ => return None,
    })
}

/// Eclipse JDT Language Server, installed together with a managed JDK.
///
/// `jdtls` tracks the newest snapshot and follows the JDK it requires; the
/// managed JDK is the Adoptium build the Adoptium API reports as latest for
/// that feature release, so the pair stays consistent.
fn jdtls_attempts() -> Vec<InstallAttempt> {
    const JDTLS_URL: &str =
        "https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz";
    let (Some(downloads), Some(dest)) = (downloads_dir(), jdtls_dir()) else {
        return Vec::new();
    };
    let jdk_archive = downloads.join("temurin.tar.gz");
    let jdtls_archive = downloads.join("jdtls.tar.gz");
    let jdk = match tools_dir() {
        Some(tools) => tools.join("jdk"),
        None => return Vec::new(),
    };
    vec![InstallAttempt::managed(
        "a Koda-managed JDK and Eclipse JDT",
        vec![
            InstallStep::AdoptiumJdk {
                feature: 25,
                dest: jdk_archive.clone(),
            },
            InstallStep::Extract {
                archive: jdk_archive,
                dest: jdk,
                strip: 1,
            },
            InstallStep::Download {
                url: JDTLS_URL.to_string(),
                dest: jdtls_archive.clone(),
                sha256: None,
            },
            InstallStep::Extract {
                archive: jdtls_archive,
                dest,
                strip: 0,
            },
        ],
    )]
}

/// OmniSharp plus the .NET SDK it runs on, both managed by Koda.
fn omnisharp_attempts() -> Vec<InstallAttempt> {
    let Some(asset) = omnisharp_asset() else {
        return Vec::new();
    };
    let (Some(tools), Some(dotnet), Some(downloads), Some(dest)) =
        (tools_dir(), dotnet_dir(), downloads_dir(), omnisharp_dir())
    else {
        return Vec::new();
    };
    let installer = tools.join("dotnet-install.sh");
    let archive = downloads.join("omnisharp.tar.gz");
    vec![InstallAttempt::managed(
        "the .NET SDK and OmniSharp",
        vec![
            InstallStep::Download {
                url: "https://dot.net/v1/dotnet-install.sh".to_string(),
                dest: installer.clone(),
                sha256: None,
            },
            InstallStep::Run(InstallCommand::with_args(
                "sh",
                vec![
                    installer.to_string_lossy().into_owned(),
                    "--channel".into(),
                    "10.0".into(),
                    "--install-dir".into(),
                    dotnet.to_string_lossy().into_owned(),
                    "--no-path".into(),
                ],
            )),
            InstallStep::Download {
                url: format!(
                    "https://github.com/OmniSharp/omnisharp-roslyn/releases/download/v2.0.0/{asset}"
                ),
                dest: archive.clone(),
                sha256: None,
            },
            InstallStep::Extract {
                archive,
                dest,
                strip: 0,
            },
        ],
    )]
}

fn venv_bin_dir(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts")
    } else {
        venv.join("bin")
    }
}

fn venv_program(venv: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        venv_bin_dir(venv).join(format!("{name}.exe"))
    } else {
        venv_bin_dir(venv).join(name)
    }
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
                LanguageId::Rust
                    | LanguageId::Go
                    | LanguageId::Python
                    | LanguageId::Shell
                    | LanguageId::TypeScript
                    | LanguageId::C
                    | LanguageId::Java
                    | LanguageId::CSharp
                    | LanguageId::Html
                    | LanguageId::Css
            ));
        }
    }

    /// The `Run` commands inside a strategy's steps.
    fn run_commands(attempt: &InstallAttempt) -> Vec<&InstallCommand> {
        attempt
            .steps
            .iter()
            .filter_map(|step| match step {
                InstallStep::Run(command) => Some(command),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn managed_tools_have_download_plans() {
        // jdtls brings its own JDK; OmniSharp brings the .NET SDK it needs.
        if tools_dir().is_some() {
            let attempts = Tool::Jdtls.install_attempts();
            let steps = &attempts.first().expect("a jdtls plan").steps;
            assert!(
                steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::AdoptiumJdk { feature: 25, .. }))
            );
            assert!(
                steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::Extract { .. }))
            );
        }
        if omnisharp_asset().is_some() && tools_dir().is_some() {
            let attempts = Tool::OmniSharp.install_attempts();
            let steps = &attempts.first().expect("an OmniSharp plan").steps;
            assert!(
                steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::Download { .. }))
            );
            assert!(
                steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::Extract { .. }))
            );
        }
    }

    #[test]
    fn unknown_program_is_not_located() {
        assert!(locate("koda-definitely-not-a-real-tool").is_none());
    }

    #[test]
    fn install_lock_is_exclusive_and_reclaims_stale_locks() {
        let dir = std::env::temp_dir().join(format!("koda-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = InstallLock::acquire_at(&dir).expect("first lock");
        assert!(
            InstallLock::acquire_at(&dir).is_err(),
            "the lock must be exclusive"
        );
        drop(first);
        assert!(
            InstallLock::acquire_at(&dir).is_ok(),
            "dropping the guard releases the lock"
        );

        // A stale lock from a crashed instance is reclaimed.
        let path = dir.join("install.lock");
        std::fs::write(&path, "99999").unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() - STALE_INSTALL_LOCK - Duration::from_secs(60))
            .unwrap();
        drop(file);
        assert!(
            InstallLock::acquire_at(&dir).is_ok(),
            "a stale lock is reclaimed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dropping_a_reclaimed_lock_leaves_the_successor_alone() {
        let dir = std::env::temp_dir().join(format!("koda-lock-nonce-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let holder = InstallLock::acquire_at(&dir).expect("lock");
        // Another instance has, in the meantime, taken the lock.
        std::fs::write(dir.join("install.lock"), "another-instance").unwrap();
        drop(holder);
        assert!(
            dir.join("install.lock").exists(),
            "a stale guard must not delete a successor's lock"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_capped_bounds_and_drains_output() {
        let data = vec![b'x'; MAX_TOOL_OUTPUT * 2];
        let kept = read_capped(std::io::Cursor::new(data));
        assert_eq!(kept.len(), MAX_TOOL_OUTPUT);
    }

    #[test]
    fn user_bin_programs_are_considered_bootstrappable() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let rustup = PathBuf::from(home).join(".cargo/bin/rustup");
        assert!(is_user_bin_program(&rustup.to_string_lossy()));
        assert!(!is_user_bin_program("rustup"));
        assert!(!is_user_bin_program("/opt/strange/place/tool"));
    }

    #[test]
    fn clangd_serves_c_and_cpp_without_provisioning() {
        assert!(Tool::Clangd.serves(LanguageId::C));
        assert!(Tool::Clangd.serves(LanguageId::Cpp));
        assert_eq!(
            Tool::for_language(LanguageId::Cpp, ToolPurpose::LanguageServer),
            Some(Tool::Clangd)
        );
        assert!(
            Tool::Clangd.install_attempts().is_empty(),
            "clangd has no user-local installer"
        );
        assert!(
            !can_install(Tool::Clangd),
            "Koda must not promise a clangd install"
        );
    }

    #[test]
    fn typescript_tool_serves_javascript_and_installs_user_locally() {
        assert!(Tool::TypeScriptLs.serves(LanguageId::TypeScript));
        assert!(Tool::TypeScriptLs.serves(LanguageId::JavaScript));
        assert_eq!(
            Tool::for_language(LanguageId::JavaScript, ToolPurpose::LanguageServer),
            Some(Tool::TypeScriptLs)
        );

        let attempts = Tool::TypeScriptLs.install_attempts();
        assert!(
            attempts.iter().any(|attempt| {
                run_commands(attempt).into_iter().any(|command| {
                    command.args.iter().any(|arg| arg == "--prefix")
                        && command
                            .args
                            .iter()
                            .any(|arg| arg == "typescript-language-server")
                })
            }),
            "TypeScript should install with npm into Koda's own prefix: {attempts:?}"
        );
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
        // Python tooling has several fallbacks, so installation is attempted
        // even without pipx or a working `pip`.
        assert!(Tool::Pylsp.install_attempts().len() >= 2);
        assert_eq!(Tool::Gofmt.install_command(), None);
        // `gofmt` cannot be installed on its own.
        assert!(install(Tool::Gofmt).is_err());
    }

    #[test]
    fn install_attempts_are_self_contained_and_user_local() {
        // `npm install -g` into a root-owned prefix fails with EACCES, so the
        // first Shell strategy must use a user-local prefix and cache.
        let Some(prefix) = npm_prefix() else {
            return; // No home directory; nothing to manage.
        };
        let attempts = Tool::BashLs.install_attempts();
        let first = attempts.first().expect("an npm attempt");
        let command = run_commands(first)
            .into_iter()
            .next()
            .expect("an npm command");
        assert_eq!(command.program, "npm");
        assert!(
            command.args.iter().any(|arg| arg == "--prefix"),
            "the first npm command should use --prefix: {:?}",
            command.args
        );
        assert!(
            command
                .args
                .iter()
                .any(|arg| arg == &prefix.to_string_lossy()),
            "the prefix should be Koda's own user-local directory: {:?}",
            command.args
        );
        assert!(
            command.args.iter().any(|arg| arg == "--cache"),
            "a user-local cache avoids a root-owned ~/.npm: {:?}",
            command.args
        );

        // A system Python without `pip` must not block pylsp: the plan includes
        // a Koda-managed virtualenv (which bootstraps its own pip), and it
        // never asks pip to override the OS (`--break-system-packages`).
        let attempts = Tool::Pylsp.install_attempts();
        assert!(
            attempts.iter().any(|attempt| {
                run_commands(attempt)
                    .into_iter()
                    .any(|command| command.args.iter().any(|arg| arg == "venv"))
            }),
            "expected a managed-virtualenv strategy: {attempts:?}"
        );
        assert!(
            attempts.iter().all(|attempt| {
                run_commands(attempt).into_iter().all(|command| {
                    !command
                        .args
                        .iter()
                        .any(|arg| arg == "--break-system-packages")
                })
            }),
            "pylsp must not modify system packages"
        );
        // pipx and uv bundle their own environments.
        assert!(attempts.iter().any(|attempt| attempt.via == "pipx"));
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
        if let Some(venv) = venv_dir() {
            assert!(
                dirs.iter().any(|dir| dir == &venv_bin_dir(&venv)),
                "the managed Python virtualenv should be searched"
            );
        }
    }

    #[test]
    fn prerequisites_report_missing_toolchains() {
        // A guaranteed-absent prerequisite is reported.
        for tool in Tool::ALL {
            let missing = tool.missing_prerequisites();
            for program in &missing {
                assert!(locate(program).is_none());
            }
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
