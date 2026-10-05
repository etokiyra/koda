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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::language::id::LanguageId;
use crate::process::wait_captured;

/// A lock older than this is assumed to be left by a crashed instance.
const STALE_INSTALL_LOCK: Duration = Duration::from_secs(15 * 60);

/// The longest a `--version` probe may run. A first-run shim can be slow, but a
/// probe must never hang the background worker.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a stdio language server is given to prove it starts. A server that
/// is still running when the window closes is treated as usable.
const PROBE_ALIVE_WINDOW: Duration = Duration::from_millis(600);

/// The Node.js LTS release Koda provisions when the user has no `node`/`npm`.
///
/// Pinned so the download and the SHA-256 Node.js publishes for it are
/// reproducible; the npm bundled with this release installs the npm-based
/// language servers into Koda's managed prefix.
const NODE_VERSION: &str = "24.21.0";

/// The `rustup-init` release Koda downloads when the user has no `rustup`.
///
/// rustup publishes a per-target binary and its SHA-256 under a versioned
/// archive URL, so the bootstrap is a verified download rather than the moving
/// `sh.rustup.rs` script.
const RUSTUP_VERSION: &str = "1.29.1";

/// The exact official Go toolchain release Koda provisions. `go.dev/dl` publishes
/// the SHA-256 of every release's archives, so the version is pinned and the
/// download is still verified against upstream metadata.
const GO_VERSION: &str = "go1.27.1";

/// The exact Eclipse Adoptium JDK releases for jdtls (25) and Kotlin (21).
///
/// The Adoptium API publishes the checksum in the same response as the download
/// link; pinning `release_name` keeps the version from drifting.
const ADOPTIUM_JDK25: &str = "jdk-25.0.4.1+1";
const ADOPTIUM_JDK21: &str = "jdk-21.0.12.1+1";

/// The `lua-language-server` release Koda provisions. It ships a self-contained
/// archive per platform (no runtime required), so the version is pinned for a
/// stable download URL.
const LUA_LS_VERSION: &str = "3.19.1";

/// The `kotlin-language-server` release Koda provisions. Pinned because its
/// bundled Kotlin compiler must run on the dedicated JDK 21 Koda installs.
const KOTLIN_LS_VERSION: &str = "1.3.13";

/// The Dart SDK release Koda provisions. Pinned so the archive and Google's
/// published `.sha256sum` stay in step.
const DART_SDK_VERSION: &str = "3.13.5";

/// The Swift toolchain release Koda provisions on supported Linux systems.
///
/// Swift ships one signed toolchain tarball per distribution release; the exact
/// asset is resolved from `/etc/os-release` at install time. The version is
/// pinned so the download URL, the signature URL and the verified key stay in
/// step.
const SWIFT_VERSION: &str = "6.4.0";

/// The ElixirLS release Koda provisions. Its launcher needs a matching
/// Erlang/OTP and Elixir runtime, which Koda installs alongside it.
const ELIXIR_LS_VERSION: &str = "0.31.1";

/// The Erlang/OTP and Elixir releases Koda provisions. They are pinned as a
/// compatible pair: the Elixir build is compiled for this OTP major.
const OTP_VERSION: &str = "27.3.4";
const ELIXIR_VERSION: &str = "1.18.4";

/// The Hex release Koda installs into the private Mix home. `mix local.hex`
/// accepts an exact version and Hex verifies the archive against the checksum
/// in `builds.hex.pm/installs/hex.csv`, so this is pinned rather than "latest".
const HEX_VERSION: &str = "2.5.1";

/// The rebar3 release Koda installs into the private Mix home.
///
/// The GitHub release publishes a single escript and Koda runs it through
/// `mix local.rebar rebar3 <path>`. GitHub reports no asset digest for this
/// release, so the SHA-256 is a Koda-computed pin of the immutable tagged asset
/// (verified to run on the pinned Erlang/OTP). It is the newest rebar3
/// compatible with OTP 27.
const REBAR3_VERSION: &str = "3.24.0";
const REBAR3_SHA256: &str = "d2d31cfb98904b8e4917300a75f870de12cb5167cd6214d1043e973a56668a54";

/// `App::cpanminus`, the non-interactive CPAN client Koda bootstraps so Perl
/// modules can be installed without an interactive `cpan` first run. The
/// archive and its SHA-256 (from MetaCPAN) are pinned together.
const CPANM_VERSION: &str = "1.7049";
const CPANM_SHA256: &str = "b9ffb88e62a06aa91bd7d5a28ef6bdbb942608aea90e3969aa29b33640035214";

/// The standalone `clangd` release Koda provisions. The clangd project publishes
/// a small, self-contained bundle per platform (much smaller than a full LLVM
/// toolchain), with a SHA-256 digest on the GitHub asset.
const CLANGD_VERSION: &str = "23.1.0";

/// The prebuilt `asm-lsp` release Koda provisions where one exists, with
/// `cargo install` as the fallback.
const ASM_LSP_VERSION: &str = "0.10.1";

/// The `phpactor` release whose `phpactor.phar` Koda provisions. phpactor is a
/// single executable phar, so Composer is not required.
const PHPACTOR_VERSION: &str = "2026.06.23.0";

/// The SHA-256 of the pinned `kotlin-language-server` `server.zip`.
///
/// GitHub publishes no asset digest for this release, so the value is a
/// Koda-computed pin of the immutable release asset (recorded from a fresh
/// download and reviewed). A replaced or corrupted asset fails closed.
const KOTLIN_LS_SHA256: &str = "4fe7d71d087b307c7869036171bd9d8c6a4284cd7c25b89098b0a24eb2d9b6d2";

/// The Eclipse JDT Language Server milestone Koda provisions.
///
/// Eclipse publishes a `.sha256` beside each milestone archive, and the
/// snapshot timestamp is part of the immutable filename, so the pinned version
/// and digest cannot drift.
const JDTLS_VERSION: &str = "1.61.0";
const JDTLS_SNAPSHOT: &str = "1.61.0-202609031315";
const JDTLS_SHA256: &str = "338e7e73d61836651ba2453919a0d34fa763eb4e7c03342092309bffb8934c64";

/// The OmniSharp release Koda provisions, pinned to an exact tag.
const OMNISHARP_VERSION: &str = "v2.0.0";

/// The exact .NET SDK version Koda provisions for OmniSharp.
///
/// Microsoft publishes a SHA-512 for every SDK archive in the channel's
/// `releases.json`, so a pinned version can be verified without the moving
/// `dotnet-install.sh` script.
const DOTNET_SDK_VERSION: &str = "10.0.401";
const DOTNET_CHANNEL: &str = "10.0";

// Pinned package-manager inputs. A package manager still resolves its own
// dependencies, but Koda always names an exact version, so an install is
// reproducible and a new upstream release cannot arrive silently.
const GOPLS_SPEC: &str = "golang.org/x/tools/gopls@v0.23.0";
const SQLS_SPEC: &str = "github.com/sqls-server/sqls@v0.2.48";
const SHFMT_SPEC: &str = "mvdan.cc/sh/v3/cmd/shfmt@v3.14.1";
const BASH_LS_SPEC: &str = "bash-language-server@5.8.1";
const TYPESCRIPT_LS_SPEC: &str = "typescript-language-server@6.0.1";
const TYPESCRIPT_SPEC: &str = "typescript@7.0.2";
const VSCODE_LANGSERVERS_SPEC: &str = "vscode-langservers-extracted@4.10.0";
const PRETTIER_SPEC: &str = "prettier@3.9.9";
const PYTHON_LSP_SPEC: &str = "python-lsp-server==1.15.0";
const SOLARGRAPH_VERSION: &str = "0.60.4";
const PLS_SPEC: &str = "PLS@0.906";
const PERL_LS_SPEC: &str = "Perl::LanguageServer@2.6.2";

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
    KotlinLs,
    Phpactor,
    LuaLs,
    Sqls,
    RubyLs,
    AsmLsp,
    PerlLs,
    Pls,
    DartAnalyzer,
    ElixirLs,
    SwiftLs,
    HtmlLs,
    CssLs,
    Rustfmt,
    Gofmt,
    Prettier,
    ClangFormat,
    Shfmt,
    PerlTidy,
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
        Tool::KotlinLs,
        Tool::Phpactor,
        Tool::LuaLs,
        Tool::Sqls,
        Tool::RubyLs,
        Tool::AsmLsp,
        // PLS is preferred over Perl::LanguageServer: it has no `Coro`
        // dependency, so it builds on current Perls.
        Tool::Pls,
        Tool::PerlLs,
        Tool::DartAnalyzer,
        Tool::ElixirLs,
        Tool::SwiftLs,
        Tool::HtmlLs,
        Tool::CssLs,
        Tool::Rustfmt,
        Tool::Gofmt,
        Tool::Prettier,
        Tool::ClangFormat,
        Tool::Shfmt,
        Tool::PerlTidy,
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
            Tool::KotlinLs => "kotlin-language-server",
            Tool::Phpactor => "phpactor",
            Tool::LuaLs => "lua-language-server",
            Tool::Sqls => "sqls",
            Tool::RubyLs => "solargraph",
            Tool::AsmLsp => "asm-lsp",
            // `Perl::LanguageServer` has no installed script; it is launched
            // through `perl`. PLS is a normal executable.
            Tool::PerlLs => "perl",
            Tool::Pls => "pls",
            Tool::DartAnalyzer => "dart",
            Tool::ElixirLs => "elixir-ls",
            Tool::SwiftLs => "sourcekit-lsp",
            Tool::HtmlLs => "vscode-html-language-server",
            Tool::CssLs => "vscode-css-language-server",
            Tool::Rustfmt => "rustfmt",
            Tool::Gofmt => "gofmt",
            Tool::Prettier => "prettier",
            Tool::ClangFormat => "clang-format",
            Tool::Shfmt => "shfmt",
            Tool::PerlTidy => "perltidy",
        }
    }

    /// Additional executable names a tool may be installed under.
    ///
    /// Some ecosystems ship different launcher names depending on how the tool
    /// was installed (for example ElixirLS's `language_server.sh` release
    /// script versus the `elixir-ls` binary packaged by Homebrew/Mason). Koda
    /// probes every name so an existing install is still found.
    pub fn candidates(self) -> &'static [&'static str] {
        match self {
            Tool::ElixirLs => &["language_server.sh", "language_server.bat"],
            _ => &[],
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
            Tool::KotlinLs => "kotlin-language-server",
            Tool::Phpactor => "phpactor",
            Tool::LuaLs => "lua-language-server",
            Tool::Sqls => "sqls",
            Tool::RubyLs => "solargraph",
            Tool::AsmLsp => "asm-lsp",
            Tool::PerlLs => "perl-language-server",
            Tool::Pls => "pls",
            Tool::DartAnalyzer => "dart",
            Tool::ElixirLs => "elixir-ls",
            Tool::SwiftLs => "sourcekit-lsp",
            Tool::HtmlLs => "vscode-html-language-server",
            Tool::CssLs => "vscode-css-language-server",
            Tool::Rustfmt => "rustfmt",
            Tool::Gofmt => "gofmt",
            Tool::Prettier => "prettier",
            Tool::ClangFormat => "clang-format",
            Tool::Shfmt => "shfmt",
            Tool::PerlTidy => "perltidy",
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
            Tool::KotlinLs => LanguageId::Kotlin,
            Tool::Phpactor => LanguageId::Php,
            Tool::LuaLs => LanguageId::Lua,
            Tool::Sqls => LanguageId::Sql,
            Tool::RubyLs => LanguageId::Ruby,
            Tool::AsmLsp => LanguageId::Assembly,
            Tool::PerlLs => LanguageId::Perl,
            Tool::Pls => LanguageId::Perl,
            Tool::DartAnalyzer => LanguageId::Dart,
            Tool::ElixirLs => LanguageId::Elixir,
            Tool::SwiftLs => LanguageId::Swift,
            Tool::HtmlLs => LanguageId::Html,
            Tool::CssLs => LanguageId::Css,
            Tool::Prettier => LanguageId::JavaScript,
            Tool::ClangFormat => LanguageId::C,
            Tool::Shfmt => LanguageId::Shell,
            Tool::PerlTidy => LanguageId::Perl,
        }
    }

    /// Whether this tool serves `language`. The TypeScript server also handles
    /// JavaScript; clangd serves both C and C++.
    pub fn serves(self, language: LanguageId) -> bool {
        self.language() == language
            || (self == Tool::TypeScriptLs && language == LanguageId::JavaScript)
            || (self == Tool::Clangd && language == LanguageId::Cpp)
            || (self == Tool::ClangFormat && language == LanguageId::Cpp)
            || (self == Tool::Prettier
                && matches!(
                    language,
                    LanguageId::TypeScript
                        | LanguageId::JavaScript
                        | LanguageId::Html
                        | LanguageId::Css
                        | LanguageId::Json
                        | LanguageId::Yaml
                        | LanguageId::Markdown
                ))
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
            | Tool::KotlinLs
            | Tool::Phpactor
            | Tool::LuaLs
            | Tool::Sqls
            | Tool::RubyLs
            | Tool::AsmLsp
            | Tool::PerlLs
            | Tool::Pls
            | Tool::DartAnalyzer
            | Tool::ElixirLs
            | Tool::SwiftLs
            | Tool::HtmlLs
            | Tool::CssLs => ToolPurpose::LanguageServer,
            Tool::Rustfmt
            | Tool::Gofmt
            | Tool::Prettier
            | Tool::ClangFormat
            | Tool::Shfmt
            | Tool::PerlTidy => ToolPurpose::Formatter,
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
            | Tool::KotlinLs
            | Tool::Phpactor
            | Tool::LuaLs
            | Tool::Sqls
            | Tool::RubyLs
            | Tool::AsmLsp
            | Tool::PerlLs
            | Tool::Pls
            | Tool::DartAnalyzer
            | Tool::ElixirLs
            | Tool::SwiftLs
            | Tool::HtmlLs
            | Tool::CssLs
            | Tool::Prettier
            | Tool::ClangFormat
            | Tool::Shfmt
            | Tool::PerlTidy => &["--version"],
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
            // `phpactor` speaks LSP through its `language-server` subcommand.
            Tool::Phpactor => &["language-server"],
            // `solargraph` needs its `stdio` subcommand.
            Tool::RubyLs => &["stdio"],
            // The Dart SDK's analysis server is launched as a subcommand.
            Tool::DartAnalyzer => &["language-server"],
            // `Perl::LanguageServer` has no installed script; it runs as a Perl
            // one-liner over stdio.
            Tool::PerlLs => &["-MPerl::LanguageServer", "-e", "Perl::LanguageServer->run"],
            // The extracted VS Code servers speak stdio.
            Tool::HtmlLs | Tool::CssLs => &["--stdio"],
            _ => &[],
        }
    }

    /// Whether this tool can only be probed by launching its stdio server.
    ///
    /// The extracted VS Code servers reject `--version` and `--help` — they
    /// require a connection mode — so a version probe reports a perfectly good
    /// install as missing. They are verified by starting the server and
    /// confirming it stays up instead.
    fn probe_as_server(self) -> bool {
        matches!(
            self,
            Tool::HtmlLs
                | Tool::CssLs
                | Tool::KotlinLs
                | Tool::Sqls
                | Tool::PerlLs
                | Tool::Pls
                | Tool::ElixirLs
                | Tool::SwiftLs
                | Tool::AsmLsp
        )
    }

    /// A short, actionable message for when the tool is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "install with `rustup component add rust-analyzer`",
            Tool::Gopls => "Koda installs gopls with `go install`, provisioning Go if missing",
            Tool::Pylsp => "install `python-lsp-server` into a Koda-managed environment",
            Tool::BashLs => "install with npm — Koda provisions Node.js if missing",
            Tool::TypeScriptLs => "install with npm — Koda provisions Node.js if missing",
            Tool::Clangd => "Koda can install the official clangd release",
            Tool::Jdtls => "Koda can install a managed JDK and Eclipse JDT",
            Tool::OmniSharp => "Koda can install the .NET SDK and OmniSharp",
            Tool::KotlinLs => "Koda can install a managed JDK 21 and kotlin-language-server",
            Tool::Phpactor => "Koda can install phpactor.phar (needs a PHP runtime)",
            Tool::LuaLs => "Koda can install a self-contained lua-language-server",
            Tool::Sqls => "Koda installs sqls with `go install`, provisioning Go if missing",
            Tool::RubyLs => "Koda installs solargraph into an isolated gem home (needs Ruby)",
            Tool::AsmLsp => "Koda can install the prebuilt asm-lsp release",
            Tool::PerlLs => {
                "Koda bootstraps cpanm and installs Perl::LanguageServer into an isolated local::lib"
            }
            Tool::Pls => "Koda bootstraps cpanm and installs PLS into an isolated local::lib",
            Tool::DartAnalyzer => "Koda can install a managed Dart SDK for the analysis server",
            Tool::ElixirLs => "Koda can install Erlang/OTP, Elixir and ElixirLS",
            Tool::SwiftLs => {
                "Koda can install a GPG-verified Swift toolchain on supported Linux distributions"
            }
            Tool::HtmlLs | Tool::CssLs => "install with npm — Koda provisions Node.js if missing",
            Tool::Rustfmt => "install with `rustup component add rustfmt`",
            Tool::Gofmt => "Koda can install the official Go toolchain, which includes gofmt",
            Tool::Prettier => "install with npm — Koda provisions Node.js if missing",
            Tool::ClangFormat => "it ships with the Clang/LLVM toolchain",
            Tool::Shfmt => "install it with `go install mvdan.cc/sh/v3/cmd/shfmt@v3.14.1`",
            Tool::PerlTidy => "install it with `cpan Perl::Tidy`",
        }
    }

    /// A rough download size for a managed install, when Koda runs one.
    ///
    /// Used to warn about a large download before it starts. It is an estimate,
    /// not a promise: the archive a distribution serves can change.
    pub fn estimated_download_bytes(self) -> Option<u64> {
        match self {
            Tool::SwiftLs if swift_toolchain().is_some() => Some(1_150_000_000),
            Tool::DartAnalyzer if dart_sdk_asset().is_some() => Some(240_000_000),
            Tool::ElixirLs if bob_platform().is_some() => Some(90_000_000),
            Tool::KotlinLs => Some(260_000_000),
            Tool::Jdtls => Some(260_000_000),
            Tool::OmniSharp => Some(280_000_000),
            Tool::LuaLs => Some(15_000_000),
            // clangd's self-contained bundle, including its clang resource
            // headers, is about 120 MB per platform.
            Tool::Clangd if clangd_asset().is_some() && !libc_is_musl() => Some(120_000_000),
            // Go's official archive is about 70 MB.
            Tool::Gopls | Tool::Sqls | Tool::Gofmt if go_platform().is_some() => Some(70_000_000),
            _ => None,
        }
    }

    /// A precise, actionable reason a tool is not usable, for Language Setup.
    ///
    /// Unlike [`Tool::install_hint`] this distinguishes "Koda can install this",
    /// "this platform is unsupported", and "a prerequisite is missing", so the
    /// UI never tells the user to install something Koda would do itself.
    pub fn setup_reason(self) -> String {
        if can_install(self) {
            let mut reason = self.install_hint().to_string();
            if let Some(bytes) = self.estimated_download_bytes() {
                reason.push_str(&format!(" (about {})", human_bytes(bytes)));
            }
            return reason;
        }
        match self {
            Tool::SwiftLs if swift_toolchain().is_none() => "no Swift toolchain is published for \
                 this platform (musl or a non-Linux host); install the Swift toolchain manually \
                 and Koda will use it"
                .to_string(),
            Tool::SwiftLs if locate("gpg").is_none() => {
                "install `gnupg` so Koda can verify the Swift toolchain signature".to_string()
            }
            Tool::SwiftLs => {
                // `can_install` was false for another reason: a library the
                // toolchain needs is missing.
                let missing = swift_missing_libs();
                let hint = if distro_is_arch() {
                    " (on Arch: `libxml2-legacy`)"
                } else {
                    ""
                };
                format!(
                    "the Swift toolchain needs {}{hint} from your distribution; install it and \
                     Koda can install Swift",
                    missing.join(", ")
                )
            }
            Tool::ElixirLs => "no Erlang/Elixir build is published for this platform; install \
                 Erlang and Elixir and Koda will use ElixirLS"
                .to_string(),
            Tool::PerlLs | Tool::Pls if locate("perl").is_none() => {
                "install Perl and Koda can set up its language server automatically".to_string()
            }
            _ => {
                let missing = self.missing_prerequisites();
                if missing.is_empty() {
                    self.install_hint().to_string()
                } else {
                    format!("needs {} — {}", missing.join(" or "), self.install_hint())
                }
            }
        }
    }

    /// The tool for a language and purpose, if Koda knows one.
    pub fn for_language(language: LanguageId, purpose: ToolPurpose) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.serves(language) && tool.purpose() == purpose)
    }

    /// The first language server for `language` that is installed and usable.
    ///
    /// A language may have more than one candidate server — Perl has PLS and
    /// `Perl::LanguageServer` — so Koda picks the first *available* one in
    /// preference order rather than committing to a single tool.
    pub fn available_server(language: LanguageId, tools: &ToolRegistry) -> Option<Tool> {
        Tool::ALL.iter().copied().find(|tool| {
            tool.serves(language)
                && tool.purpose() == ToolPurpose::LanguageServer
                && tools.available(*tool)
        })
    }

    /// The first language server for `language` that Koda could install.
    pub fn installable_server(language: LanguageId, tools: &ToolRegistry) -> Option<Tool> {
        Tool::ALL.iter().copied().find(|tool| {
            tool.serves(language)
                && tool.purpose() == ToolPurpose::LanguageServer
                && !tools.available(*tool)
                && can_install(*tool)
        })
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
            Tool::Pylsp => Some(("pipx", &["install", PYTHON_LSP_SPEC])),
            Tool::BashLs => Some(("npm", &["install", "-g", BASH_LS_SPEC])),
            Tool::TypeScriptLs => Some((
                "npm",
                &["install", "-g", TYPESCRIPT_LS_SPEC, TYPESCRIPT_SPEC],
            )),
            // `clangd`, `phpactor`, `lua-language-server`, Kotlin, Dart, Elixir,
            // Swift and Go tooling are installed by Koda's own managed plans.
            Tool::Clangd => None,
            Tool::Phpactor => None,
            Tool::LuaLs => None,
            Tool::KotlinLs => None,
            // Go tooling is installed by Koda's managed plan, which provisions
            // Go itself when it is missing.
            Tool::Gopls | Tool::Sqls => None,
            // `solargraph` installs into a Koda-private gem home.
            Tool::RubyLs => None,
            // `asm-lsp` uses a prebuilt release, falling back to `cargo`.
            Tool::AsmLsp => None,
            // Perl, Dart, Elixir and Swift servers are installed by Koda's own
            // plans rather than a single package-manager command.
            Tool::PerlLs | Tool::Pls | Tool::DartAnalyzer | Tool::ElixirLs | Tool::SwiftLs => None,
            // `jdtls` and `OmniSharp` are installed by Koda's own managed
            // download plan rather than a single package-manager command.
            Tool::Jdtls | Tool::OmniSharp => None,
            Tool::HtmlLs | Tool::CssLs => {
                Some(("npm", &["install", "-g", VSCODE_LANGSERVERS_SPEC]))
            }
            Tool::Gofmt => None,
            Tool::Prettier => Some(("npm", &["install", "-g", PRETTIER_SPEC])),
            Tool::Shfmt => None,
            // `clang-format` and `perltidy` ship with their language toolchains.
            Tool::ClangFormat | Tool::PerlTidy => None,
        }
    }

    /// Programs this tool needs before it can be installed, so Koda can explain
    /// when a whole toolchain is missing.
    pub fn prerequisites(self) -> &'static [&'static str] {
        match self {
            Tool::RustAnalyzer | Tool::Rustfmt => &["rustup"],
            // `gopls`/`sqls`/`shfmt` are installed by Koda's own plan, which
            // provisions the official Go toolchain when none is present.
            Tool::Gopls | Tool::Gofmt | Tool::Sqls | Tool::Shfmt => &[],
            Tool::Pylsp => &["python3"],
            // Koda can provision Node.js itself, so npm is not a hard
            // prerequisite. The install plan checks for `curl` and an archive
            // tool before offering the managed runtime.
            Tool::BashLs | Tool::TypeScriptLs => &[],
            Tool::HtmlLs | Tool::CssLs => &[],
            // `jdtls` is a Python launcher script.
            Tool::Jdtls => &["python3"],
            // `clangd` uses a self-contained bundle; `phpactor` needs only a PHP
            // runtime; `asm-lsp` uses a prebuilt release.
            Tool::Clangd | Tool::OmniSharp => &[],
            Tool::Phpactor => &["php"],
            Tool::LuaLs => &[],
            // The dedicated JDK 21 is provided by Koda's managed download plan,
            // so no system Java is required.
            Tool::KotlinLs => &[],
            // `solargraph` installs into a Koda-private gem home but still needs
            // a system Ruby.
            Tool::RubyLs => &["gem"],
            Tool::AsmLsp => &[],
            Tool::PerlLs | Tool::Pls => &["perl"],
            // Dart, Erlang/Elixir and the Swift toolchain are provided by
            // Koda's managed download plans.
            Tool::DartAnalyzer | Tool::ElixirLs | Tool::SwiftLs => &[],
            // Formatters.
            Tool::Prettier => &[],
            Tool::ClangFormat | Tool::PerlTidy => &[],
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
            // Go tooling installs with `go install`, provisioning the official
            // Go toolchain when none is present.
            Tool::Gopls => go_attempts(&[GOPLS_SPEC]),
            Tool::Pylsp => python_attempts(),
            Tool::BashLs => npm_attempts(&[BASH_LS_SPEC]),
            Tool::TypeScriptLs => npm_attempts(&[TYPESCRIPT_LS_SPEC, TYPESCRIPT_SPEC]),
            // The clangd project publishes a small, self-contained bundle, so
            // Koda does not need a full LLVM toolchain.
            Tool::Clangd => clangd_attempts(),
            // `phpactor` is a single phar; Koda downloads it directly and only
            // needs a PHP runtime, not Composer.
            Tool::Phpactor => phpactor_attempts(),
            // `lua-language-server` ships a self-contained, runtime-free archive
            // per platform, so Koda manages it like the JDK and OmniSharp.
            Tool::LuaLs => lua_ls_attempts(),
            // `kotlin-language-server` needs a JDK whose version its bundled
            // compiler understands, so Koda installs a dedicated JDK 21.
            Tool::KotlinLs => kotlin_ls_attempts(),
            Tool::Sqls => go_attempts(&[SQLS_SPEC]),
            // `solargraph` installs into a Koda-private gem home when a system
            // Ruby is available.
            Tool::RubyLs => ruby_attempts(),
            // A prebuilt `asm-lsp` release where one exists, otherwise `cargo`
            // (bootstrapping the Rust toolchain when needed).
            Tool::AsmLsp => asm_lsp_attempts(),
            // The Dart SDK is self-contained and managed by Koda.
            Tool::DartAnalyzer => dart_sdk_attempts(),
            // `Perl::LanguageServer` bootstraps cpanm and installs into an
            // isolated local::lib.
            Tool::PerlLs => perl_attempts(),
            // PLS has no `Coro` dependency, so it is the preferred server.
            Tool::Pls => pls_attempts(),
            // Erlang/OTP, Elixir and ElixirLS are installed as one toolchain.
            Tool::ElixirLs => elixir_ls_attempts(),
            // The Swift toolchain is GPG-verified and only offered where
            // swift.org publishes a build for the running distribution.
            Tool::SwiftLs => swift_attempts(),
            Tool::Jdtls => jdtls_attempts(),
            Tool::OmniSharp => omnisharp_attempts(),
            Tool::HtmlLs | Tool::CssLs => npm_attempts(&[VSCODE_LANGSERVERS_SPEC]),
            // `gofmt` ships with Go, so Koda provisions the toolchain.
            Tool::Gofmt => go_toolchain_attempts(),
            Tool::Prettier => npm_attempts(&[PRETTIER_SPEC]),
            Tool::Shfmt => go_attempts(&[SHFMT_SPEC]),
            // `clang-format` and `perltidy` ship with their toolchains.
            Tool::ClangFormat | Tool::PerlTidy => Vec::new(),
        }
    }
}

/// One command in an install attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Extra environment variables for this command (for example the managed
    /// `MIX_HOME`/`HEX_HOME` an Elixir build needs).
    pub env: Vec<(String, String)>,
    /// Working directory for this command, when it must run somewhere specific
    /// (the ElixirLS release directory its installer expects).
    pub cwd: Option<PathBuf>,
}

impl InstallCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        InstallCommand {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            env: Vec::new(),
            cwd: None,
        }
    }

    fn with_args(program: impl Into<String>, args: Vec<String>) -> Self {
        InstallCommand {
            program: program.into(),
            args,
            env: Vec::new(),
            cwd: None,
        }
    }

    /// Add an environment variable to this command.
    fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Run this command from `dir`.
    fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }
}

/// One executable step in an install strategy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallStep {
    /// Run a program with arguments.
    Run(InstallCommand),
    /// Download `url` to `dest` over HTTPS, verifying `sha256`.
    ///
    /// The digest is required: there is no way to construct this step without
    /// verification, so a new managed download must supply one (or use a step
    /// that verifies by another mechanism, such as [`InstallStep::GithubRelease`]).
    Download {
        url: String,
        dest: PathBuf,
        sha256: String,
    },
    /// Download and verify an Erlang/OTP or Elixir build from `builds.hex.pm`,
    /// whose SHA-256 the same service publishes in `builds.txt`. The archive
    /// still needs an [`InstallStep::Extract`].
    BobBuild { package: BobPackage, dest: PathBuf },
    /// Download and verify an Eclipse Adoptium JDK release, whose checksum
    /// Adoptium publishes in the same JSON document that carries the link. The
    /// `release` pins the exact version.
    AdoptiumJdk {
        release: &'static str,
        dest: PathBuf,
    },
    /// Download and verify the pinned .NET SDK for OmniSharp, whose SHA-512
    /// Microsoft publishes in the channel's `releases.json`. The archive still
    /// needs an [`InstallStep::Extract`].
    DotnetSdk { dest: PathBuf },
    /// Download and verify a Koda-managed Node.js runtime, whose checksum the
    /// Node.js project publishes in `SHASUMS256.txt`. The archive still needs
    /// an [`InstallStep::Extract`].
    NodeRuntime { dest: PathBuf },
    /// Download and verify a Koda-managed Dart SDK, whose checksum Google
    /// publishes beside the archive. The archive still needs an
    /// [`InstallStep::Extract`].
    DartSdk { dest: PathBuf },
    /// Download, verify and run the pinned `rustup-init` binary, bootstrapping
    /// the Rust toolchain without the unverified `sh.rustup.rs` script.
    RustupInit { dest: PathBuf },
    /// Download `url` and verify it against a detached GPG signature using the
    /// public keys at `keys_url`. Used for toolchains whose only published
    /// integrity data is a signature (the Swift toolchain). Fails closed: if
    /// `gpg` is missing or the signature does not verify, nothing is kept.
    DownloadGpg {
        url: String,
        dest: PathBuf,
        signature_url: String,
        keys_url: String,
    },
    /// Download a GitHub release asset and verify its SHA-256, which GitHub's
    /// API now reports as an asset `digest`. The archive still needs an
    /// [`InstallStep::Extract`].
    GithubRelease {
        repo: String,
        tag: String,
        asset: String,
        dest: PathBuf,
    },
    /// Create Koda's Swift compatibility directory: a `libncurses.so.6` alias to
    /// the system's wide library and a link to the system's `libxml2.so.2`,
    /// without modifying the system.
    SwiftCompat { dest: PathBuf },
    /// Download and verify the latest stable official Go toolchain, whose
    /// version and SHA-256 `go.dev/dl` publishes in one JSON document. The
    /// archive still needs an [`InstallStep::Extract`].
    GoToolchain { dest: PathBuf },
    /// Mark a file executable. Used for a downloaded single-file program (a
    /// `.phar`, a prebuilt binary) that must run directly.
    MakeExecutable { path: PathBuf },
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

/// An Erlang/Elixir build distributed by `builds.hex.pm` (the Erlang Ecosystem
/// Foundation's `bob` build service, which also backs `setup-beam`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BobPackage {
    /// An Erlang/OTP runtime.
    Erlang,
    /// An Elixir distribution compiled for a specific OTP major.
    Elixir,
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
    if let Some(home) = home_dir() {
        let rustup = home.join(".cargo/bin/rustup");
        if let Some(bootstrap) = rustup_bootstrap_with(InstallCommand::with_args(
            rustup.to_string_lossy().into_owned(),
            vec!["component".into(), "add".into(), component.into()],
        )) {
            attempts.push(bootstrap);
        }
    }
    attempts
}

/// Install a Rust tool with `cargo`, bootstrapping the toolchain when absent.
fn cargo_attempts(package: &str, version: &str) -> Vec<InstallAttempt> {
    let args = cargo_install_args(package, version);
    let mut attempts = vec![InstallAttempt::one(
        "cargo install",
        InstallCommand::with_args("cargo", args.clone()),
    )];
    if let Some(home) = home_dir() {
        let cargo = home.join(".cargo/bin/cargo");
        if let Some(bootstrap) = rustup_bootstrap_with(InstallCommand::with_args(
            cargo.to_string_lossy().into_owned(),
            args,
        )) {
            attempts.push(bootstrap);
        }
    }
    attempts
}

/// The pinned `cargo install <crate> --version <version>` arguments.
fn cargo_install_args(package: &str, version: &str) -> Vec<String> {
    vec![
        "install".to_string(),
        package.to_string(),
        "--version".to_string(),
        version.to_string(),
    ]
}

fn home_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?))
}

/// The `rustup-init` build target for this platform, or `None` where rustup
/// publishes no build.
fn rustup_target() -> Option<&'static str> {
    let musl = libc_is_musl();
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if musl => "x86_64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") if musl => "aarch64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    })
}

/// The first SHA-256 in a checksum body (`<hex> *./name` or `<hex>  name`).
fn first_sha256(body: &str) -> Option<String> {
    body.split_whitespace()
        .find(|token| is_sha256_hex(token))
        .map(str::to_string)
}

/// Download, verify and run the pinned `rustup-init` binary.
///
/// The old bootstrap ran the moving `sh.rustup.rs` script with no integrity
/// check. rustup publishes a per-target `rustup-init` binary and its SHA-256
/// under a versioned archive URL, so Koda downloads the exact binary, verifies
/// it, and runs it non-interactively; rustup then verifies the toolchain it
/// installs through its own signed manifests.
fn rustup_init(dest: &Path) -> Result<(), String> {
    let target =
        rustup_target().ok_or_else(|| "rustup publishes no build for this platform".to_string())?;
    let base = format!("https://static.rust-lang.org/rustup/archive/{RUSTUP_VERSION}/{target}");
    let sums = curl_text(&format!("{base}/rustup-init.sha256"))
        .map_err(|err| format!("could not fetch the rustup checksum: {err}"))?;
    let checksum = first_sha256(&sums)
        .ok_or_else(|| "rustup published no checksum for rustup-init".to_string())?;
    download(&format!("{base}/rustup-init"), dest, Some(&checksum))?;
    make_executable(dest)?;
    run_command(&InstallCommand::with_args(
        dest.to_string_lossy().into_owned(),
        vec![
            "-y".into(),
            "--no-modify-path".into(),
            "--profile".into(),
            "minimal".into(),
        ],
    ))
}

/// Bootstrap the Rust toolchain with the verified `rustup-init` binary, then run
/// `final_command`. Skipped on Windows, where Koda has no supported path.
fn rustup_bootstrap_with(final_command: InstallCommand) -> Option<InstallAttempt> {
    if cfg!(windows) || home_dir().is_none() || rustup_target().is_none() {
        return None;
    }
    let dest = downloads_dir()?.join("rustup-init");
    Some(InstallAttempt::managed(
        "the official rustup installer",
        vec![
            InstallStep::RustupInit { dest },
            InstallStep::Run(final_command),
        ],
    ))
}

/// Install Go tooling with `go install`, provisioning Go itself when absent.
///
/// `go install` writes to a Koda-private `GOBIN`, so the tools never land in
/// the user's `~/go` and never mix with a system Go's installations. The first
/// attempt uses whatever `go` the user has; the second downloads the official
/// Go toolchain into Koda's data directory.
fn go_attempts(packages: &[&str]) -> Vec<InstallAttempt> {
    let install = go_install_command(packages);
    let mut attempts = vec![InstallAttempt::one("go install", install.clone())];
    if let Some(managed) = managed_go_attempt(Some(install)) {
        attempts.push(managed);
    }
    attempts
}

/// Download the official Go toolchain (and optionally run a final command with
/// it), when Koda can provision Go on this platform.
fn managed_go_attempt(final_command: Option<InstallCommand>) -> Option<InstallAttempt> {
    go_platform()?;
    let downloads = downloads_dir()?;
    let dest = go_dir()?;
    let archive = downloads.join("go.tar.gz");
    let mut steps = vec![
        InstallStep::GoToolchain {
            dest: archive.clone(),
        },
        InstallStep::Extract {
            archive,
            dest,
            strip: 1,
        },
    ];
    if let Some(command) = final_command {
        steps.push(InstallStep::Run(command));
    }
    Some(InstallAttempt::managed("the official Go toolchain", steps))
}

/// Install the Go toolchain alone; `gofmt` ships with it.
fn go_toolchain_attempts() -> Vec<InstallAttempt> {
    managed_go_attempt(None).into_iter().collect()
}

/// The `go install` command for a set of pinned `module@version` specs, with a
/// Koda-private `GOPATH`/`GOBIN` so installations stay isolated.
fn go_install_command(specs: &[&str]) -> InstallCommand {
    let mut args = vec!["install".to_string()];
    for spec in specs {
        args.push((*spec).to_string());
    }
    let mut command = InstallCommand::with_args("go", args);
    if let (Some(gopath), Some(gobin)) = (go_path(), go_bin_dir()) {
        command = command
            .env("GOPATH", gopath.to_string_lossy().into_owned())
            .env("GOBIN", gobin.to_string_lossy().into_owned())
            .env("GOFLAGS", "-mod=mod");
    }
    // Only pin `GOROOT` to a managed Go that is actually present; an invalid
    // `GOROOT` makes a perfectly good system `go` refuse to run, which would
    // force a needless managed download.
    if let Some(goroot) = go_dir().filter(|dir| dir.join("bin/go").is_file()) {
        command = command.env("GOROOT", goroot.to_string_lossy().into_owned());
    }
    command
}

/// The standalone `clangd` bundle for this platform, or `None`.
fn clangd_asset() -> Option<String> {
    Some(match std::env::consts::OS {
        "linux" => format!("clangd-linux-{CLANGD_VERSION}.zip"),
        "macos" => format!("clangd-mac-{CLANGD_VERSION}.zip"),
        "windows" => format!("clangd-windows-{CLANGD_VERSION}.zip"),
        _ => return None,
    })
}

/// Download the official, self-contained `clangd` release.
fn clangd_attempts() -> Vec<InstallAttempt> {
    let Some(asset) = clangd_asset() else {
        return Vec::new();
    };
    // The prebuilt clangd bundles link against glibc and libstdc++, so they
    // cannot run on musl systems.
    if libc_is_musl() {
        return Vec::new();
    }
    let (Some(downloads), Some(dest)) = (downloads_dir(), clangd_dir()) else {
        return Vec::new();
    };
    let archive = downloads.join(&asset);
    vec![InstallAttempt::managed(
        "the official clangd release",
        vec![
            InstallStep::GithubRelease {
                repo: "clangd/clangd".to_string(),
                tag: CLANGD_VERSION.to_string(),
                asset,
                dest: archive.clone(),
            },
            // The archive root is `clangd_<version>/`.
            InstallStep::Extract {
                archive,
                dest,
                strip: 1,
            },
        ],
    )]
}

/// The prebuilt `asm-lsp` archive for this platform, or `None` when the project
/// publishes none (for example aarch64 Linux).
fn asm_lsp_asset() -> Option<String> {
    // The prebuilt binaries are glibc/macOS builds; musl uses the cargo plan.
    if libc_is_musl() {
        return None;
    }
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "asm-lsp-x86_64-unknown-linux-gnu.tar.gz".to_string(),
        ("macos", "x86_64") => "asm-lsp-x86_64-apple-darwin.tar.gz".to_string(),
        ("macos", "aarch64") => "asm-lsp-aarch64-apple-darwin.tar.gz".to_string(),
        _ => return None,
    })
}

/// Install `asm-lsp`, preferring the prebuilt release over a `cargo` build.
fn asm_lsp_attempts() -> Vec<InstallAttempt> {
    let mut attempts = Vec::new();
    if let Some(asset) = asm_lsp_asset()
        && let (Some(downloads), Some(dest)) = (downloads_dir(), asm_lsp_dir())
    {
        let archive = downloads.join(&asset);
        attempts.push(InstallAttempt::managed(
            "the prebuilt asm-lsp release",
            vec![
                InstallStep::GithubRelease {
                    repo: "bergercookie/asm-lsp".to_string(),
                    tag: format!("v{ASM_LSP_VERSION}"),
                    asset,
                    dest: archive.clone(),
                },
                // The archive contains a single `asm-lsp` binary at its root.
                InstallStep::Extract {
                    archive,
                    dest,
                    strip: 0,
                },
            ],
        ));
    }
    attempts.extend(cargo_attempts("asm-lsp", ASM_LSP_VERSION));
    attempts
}

/// Install `phpactor.phar`, which needs only a PHP runtime.
fn phpactor_attempts() -> Vec<InstallAttempt> {
    if locate("php").is_none() {
        return Vec::new();
    }
    let Some(dest) = phpactor_dir() else {
        return Vec::new();
    };
    let phar = dest.join("phpactor");
    vec![InstallAttempt::managed(
        "the official phpactor.phar",
        vec![
            InstallStep::GithubRelease {
                repo: "phpactor/phpactor".to_string(),
                tag: PHPACTOR_VERSION.to_string(),
                asset: "phpactor.phar".to_string(),
                dest: phar.clone(),
            },
            // The phar starts with `#!/usr/bin/env php`; make it runnable under
            // the name `phpactor`.
            InstallStep::MakeExecutable { path: phar },
        ],
    )]
}

/// Install `solargraph` into a Koda-private gem home.
///
/// A system Ruby is still required; Koda only isolates the gems so the user's
/// global gem environment is untouched.
fn ruby_attempts() -> Vec<InstallAttempt> {
    if locate("gem").is_none() {
        return Vec::new();
    }
    let Some(gems) = gem_home() else {
        return Vec::new();
    };
    let bindir = gems.join("bin");
    vec![InstallAttempt::one(
        "gem install (isolated gem home)",
        InstallCommand::with_args(
            "gem",
            vec![
                "install".to_string(),
                "--no-document".to_string(),
                "--install-dir".to_string(),
                gems.to_string_lossy().into_owned(),
                "--bindir".to_string(),
                bindir.to_string_lossy().into_owned(),
                "--version".to_string(),
                SOLARGRAPH_VERSION.to_string(),
                "solargraph".to_string(),
            ],
        ),
    )]
}

/// Strategies for installing the Python language server, from most isolated to
/// least. The virtualenv attempt bootstraps its own `pip`, so it works even
/// when the system Python has no `pip` module or is externally managed.
fn python_attempts() -> Vec<InstallAttempt> {
    let mut attempts = vec![
        InstallAttempt::one(
            "pipx",
            InstallCommand::new("pipx", &["install", PYTHON_LSP_SPEC]),
        ),
        InstallAttempt::one(
            "uv",
            InstallCommand::new("uv", &["tool", "install", PYTHON_LSP_SPEC]),
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
                        vec!["install".into(), PYTHON_LSP_SPEC.into()],
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
                InstallCommand::new(python, &["-m", "pip", "install", "--user", PYTHON_LSP_SPEC]),
            ],
        ));
        attempts.push(InstallAttempt::one(
            "pip --user",
            InstallCommand::new(python, &["-m", "pip", "install", "--user", PYTHON_LSP_SPEC]),
        ));
    }
    attempts.push(InstallAttempt::one(
        "pip3 --user",
        InstallCommand::new("pip3", &["install", "--user", PYTHON_LSP_SPEC]),
    ));

    attempts
}

/// Strategies for installing an npm package into a user-local prefix.
///
/// The first two use whatever `npm` the user already has; the last provisions a
/// Koda-managed Node.js runtime (with its bundled npm) so the install still
/// succeeds on a machine with no Node.js at all.
fn npm_attempts(packages: &[&str]) -> Vec<InstallAttempt> {
    let mut attempts = Vec::new();
    if let Some(prefix) = npm_prefix() {
        // A user-local prefix and cache: `npm install -g` into a root-owned
        // prefix, or a `~/.npm` left root-owned by a past `sudo npm`, would
        // otherwise fail with EACCES.
        let cache = prefix.with_file_name("npm-cache");
        attempts.push(InstallAttempt::one(
            "npm (user-local prefix)",
            InstallCommand::with_args("npm", npm_args(Some((&prefix, &cache)), packages)),
        ));
    }
    // Fall back to whatever global prefix the user's npm (nvm, fnm, volta, …)
    // already uses.
    attempts.push(InstallAttempt::one(
        "npm",
        InstallCommand::with_args("npm", npm_args(None, packages)),
    ));

    // No Node.js at all: install one under Koda's data directory and use its
    // bundled npm, so zero-configuration still holds.
    if node_provisionable()
        && let (Some(prefix), Some(archive), Some(dest), Some(npm)) =
            (npm_prefix(), node_archive(), node_dir(), managed_npm())
    {
        let cache = prefix.with_file_name("npm-cache");
        attempts.push(InstallAttempt::managed(
            "a Koda-managed Node.js runtime",
            vec![
                InstallStep::NodeRuntime {
                    dest: archive.clone(),
                },
                InstallStep::Extract {
                    archive,
                    dest,
                    strip: 1,
                },
                InstallStep::Run(InstallCommand::with_args(
                    npm.to_string_lossy().into_owned(),
                    npm_args(Some((&prefix, &cache)), packages),
                )),
            ],
        ));
    }
    attempts
}

/// The `npm install -g` arguments for a package list, optionally targeting a
/// user-local prefix and cache (so a system-owned prefix cannot make it fail).
fn npm_args(prefix_and_cache: Option<(&Path, &Path)>, packages: &[&str]) -> Vec<String> {
    let mut args = vec!["install".to_string(), "-g".to_string()];
    if let Some((prefix, cache)) = prefix_and_cache {
        args.push("--prefix".to_string());
        args.push(prefix.to_string_lossy().into_owned());
        args.push("--cache".to_string());
        args.push(cache.to_string_lossy().into_owned());
    }
    args.extend(packages.iter().map(|package| (*package).to_string()));
    args
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
            tool.setup_reason()
        ));
    }
    // Pre-flight the plan before any install subprocess runs: keep only
    // strategies whose every step can actually run, so an obvious failure is
    // reported before an earlier tool in the plan is installed and before a
    // large download starts.
    let runnable = filter_runnable(attempts);
    if runnable.is_empty() {
        return Err(format!(
            "{} cannot be installed — {}",
            tool.label(),
            tool.setup_reason()
        ));
    }

    let _lock = InstallLock::acquire()?;
    preflight(tool)?;

    let mut last_error = None;
    for attempt in &runnable {
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
        if completed {
            let status = probe(tool);
            if status.available {
                return Ok(format!("Installed {}", tool.label()));
            }
            // The steps worked but the tool does not run: keep the reason (a
            // missing library, a broken launcher) rather than a generic failure.
            if let Some(detail) = status.error {
                last_error = Some(format!("{}: {detail}", tool.label()));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| format!("could not install {}", tool.label())))
}

/// The strategies whose every step can actually run on this host.
fn filter_runnable(attempts: Vec<InstallAttempt>) -> Vec<InstallAttempt> {
    attempts
        .into_iter()
        .filter(|attempt| attempt.steps.iter().all(step_available))
        .collect()
}

/// Validate an install plan before any destructive work begins.
///
/// This checks only what Koda can know cheaply and reliably: that its tools
/// directory exists and is writable, and that the target filesystem has room for
/// the estimated download. Network availability, proxy reachability and archive
/// contents are necessarily runtime checks and are not predicted here.
fn preflight(tool: Tool) -> Result<(), String> {
    let dir = tools_dir().ok_or_else(|| "no data directory for managed tools".to_string())?;
    ensure_tools_dir_writable(&dir)?;
    if let Some(bytes) = tool.estimated_download_bytes() {
        ensure_disk_space(&dir, bytes)?;
    }
    Ok(())
}

/// Confirm Koda can create and write inside its managed tools directory.
fn ensure_tools_dir_writable(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| {
        format!(
            "could not create Koda's tools directory {}: {err}",
            dir.display()
        )
    })?;
    let probe = temp_sibling(&dir.join("write-probe"))?;
    std::fs::write(&probe, b"").map_err(|err| {
        format!(
            "Koda's tools directory {} is not writable: {err}",
            dir.display()
        )
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Execute one install step.
fn run_step(step: &InstallStep) -> Result<(), String> {
    match step {
        InstallStep::Run(command) => run_command(command),
        InstallStep::Download { url, dest, sha256 } => download(url, dest, Some(sha256)),
        InstallStep::BobBuild { package, dest } => bob_build(*package, dest),
        InstallStep::AdoptiumJdk { release, dest } => adoptium_jdk(release, dest),
        InstallStep::DotnetSdk { dest } => dotnet_sdk(dest),
        InstallStep::NodeRuntime { dest } => node_runtime(dest),
        InstallStep::DartSdk { dest } => dart_sdk(dest),
        InstallStep::RustupInit { dest } => rustup_init(dest),
        InstallStep::DownloadGpg {
            url,
            dest,
            signature_url,
            keys_url,
        } => download_gpg(url, dest, signature_url, keys_url),
        InstallStep::GithubRelease {
            repo,
            tag,
            asset,
            dest,
        } => github_release(repo, tag, asset, dest),
        InstallStep::SwiftCompat { dest } => swift_compat(dest),
        InstallStep::GoToolchain { dest } => go_toolchain(dest),
        InstallStep::MakeExecutable { path } => make_executable(path),
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
///
/// Koda's managed runtimes (Node.js, Erlang/OTP, Elixir, Dart, Perl, Swift) are
/// put on `PATH`, so a managed `elixir`, `npm` or `perl` and any
/// `#!/usr/bin/env` launcher work even before the user has installed anything.
fn install_command(program: &str) -> Command {
    let mut command = Command::new(program);
    if let Some(dir) = install_work_dir() {
        command.current_dir(dir);
    }
    if let Some(path) = managed_path() {
        command.env("PATH", path);
    }
    command
}

/// `PATH` with every Koda-managed runtime bin directory first.
fn managed_path() -> Option<String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(node) = node_bin_dir() {
        dirs.push(node);
    }
    if let Some(tools) = tools_dir() {
        dirs.push(tools.join("otp/bin"));
        dirs.push(tools.join("elixir/bin"));
        dirs.push(tools.join("dart-sdk/bin"));
        dirs.push(tools.join("perl5/bin"));
        dirs.push(tools.join("swift/usr/bin"));
        dirs.push(tools.join("go/bin"));
        dirs.push(tools.join("gopath/bin"));
        dirs.push(tools.join("clangd/bin"));
        dirs.push(tools.join("asm-lsp"));
        dirs.push(tools.join("phpactor"));
        dirs.push(tools.join("gems/bin"));
        dirs.push(tools.join("kotlin-jdk/bin"));
        dirs.push(tools.join("jdk/bin"));
        dirs.push(tools.join("dotnet"));
    }
    if dirs.is_empty() {
        return None;
    }
    let prefix: Vec<String> = dirs
        .iter()
        .map(|dir| dir.to_string_lossy().into_owned())
        .collect();
    let existing = std::env::var("PATH").unwrap_or_default();
    Some(format!("{}:{existing}", prefix.join(":")))
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
/// A hung package manager would otherwise stall background work, and a verbose
/// tool could exhaust memory through unbounded capture.
fn run_command(command: &InstallCommand) -> Result<(), String> {
    use std::process::Stdio;

    let mut child = install_command(&command.program);
    child.args(&command.args);
    if let Some(dir) = &command.cwd {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
        child.current_dir(dir);
    }
    for (key, value) in &command.env {
        child.env(key, value);
    }
    let mut child = child
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("could not run {}: {err}", command.program))?;

    let captured = crate::process::wait_captured(&mut child, COMMAND_TIMEOUT, MAX_TOOL_OUTPUT)
        .map_err(|err| format!("could not run {}: {err}", command.program))?;
    match captured.status {
        None => Err(format!(
            "{} timed out after {} minutes",
            command.program,
            COMMAND_TIMEOUT.as_secs() / 60
        )),
        Some(status) if status.success() => Ok(()),
        // Package managers print the actual error last (the syntax error, the
        // missing module), so the tail line is the useful one.
        Some(status) => Err(format!(
            "{} exited with {status}: {}",
            command.program,
            last_stderr_line(&captured.stderr)
        )),
    }
}

/// Download `url` to `dest`, verifying a SHA-256 when one is known.
///
/// A verified artifact is reused from Koda's cache when present, so a reinstall
/// or repair needs no network. The destination is replaced atomically only after
/// verification, so a failed download never clobbers a known-good file.
fn download(url: &str, dest: &Path, sha256: Option<&str>) -> Result<(), String> {
    match sha256 {
        Some(expected) => download_verified(
            url,
            dest,
            expected,
            HashKind::Sha256,
            cache_dir().as_deref(),
        ),
        // Unverified material (a signing key or a detached signature) is still
        // written atomically so a failed fetch cannot corrupt an existing copy.
        None => {
            let tmp = temp_sibling(dest)?;
            curl_download(url, &tmp)?;
            atomic_replace(&tmp, dest)
        }
    }
}

/// Download `url` to `dest`, verifying a SHA-512.
fn download_sha512(url: &str, dest: &Path, sha512: &str) -> Result<(), String> {
    download_verified(url, dest, sha512, HashKind::Sha512, cache_dir().as_deref())
}

/// Download and verify, reusing a verified cache entry when one exists.
///
/// Cache identity is the expected digest itself, never the filename, and a
/// cached file is verified again before it is used — a corrupt entry is removed
/// and the download retried. A freshly downloaded file is cached only after it
/// has passed verification.
fn download_verified(
    url: &str,
    dest: &Path,
    expected: &str,
    kind: HashKind,
    cache: Option<&Path>,
) -> Result<(), String> {
    if let Some(dir) = cache
        && let Some(cached) = cache_lookup(dir, expected, kind)?
    {
        return link_or_copy(&cached, dest);
    }

    let tmp = temp_sibling(dest)?;
    let result = (|| {
        curl_download(url, &tmp)?;
        verify_hash(&tmp, expected, kind)
    })();
    if let Err(err) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    // Only a verified artifact enters the cache, and a cache failure never
    // fails the install it is accelerating.
    if let Some(dir) = cache {
        let _ = cache_store(dir, expected, &tmp, kind);
    }
    atomic_replace(&tmp, dest)
}

/// Which digest algorithm a verification uses.
#[derive(Clone, Copy)]
enum HashKind {
    Sha256,
    Sha512,
}

impl HashKind {
    fn label(self) -> &'static str {
        match self {
            HashKind::Sha256 => "sha256",
            HashKind::Sha512 => "sha512",
        }
    }
}

/// Fetch `url` to `dest` with the shared, bounded curl invocation.
///
/// No verification happens here; callers must run [`verify_hash`] on the result.
fn curl_download(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }
    // Tests exercise the real install path (a `Download` step) without the
    // network by copying a local payload, so verification is still the same
    // code the production path runs. Never available in a production build.
    #[cfg(test)]
    if let Some(source) = url.strip_prefix("file://") {
        std::fs::copy(source, dest).map_err(|err| format!("could not copy {source}: {err}"))?;
        return Ok(());
    }
    let mut command = install_command("curl");
    command.args([
        "--proto",
        "=https",
        "--tlsv1.2",
        "-L",
        "--fail",
        "-sS",
        // Decode a gzip transfer encoding. Some CDNs compress text responses
        // (for example a PGP key block) regardless of the file extension; without
        // this the downloaded key file is gzip bytes and the signature check fails.
        "--compressed",
        "--connect-timeout",
        "30",
        // Retry transient failures (a reset connection, a 5xx) a few times
        // before giving up, so a large managed download survives a blip.
        "--retry",
        "4",
        "--retry-delay",
        "2",
        "--retry-max-time",
        "180",
        "--max-time",
        "3600",
        // Abort a genuinely stalled transfer (under 1 KiB/s for a minute) rather
        // than letting it sit until the hour-long cap, while still allowing a
        // large managed toolchain to finish on a slow-but-steady link.
        "--speed-limit",
        "1024",
        "--speed-time",
        "60",
    ]);
    apply_proxy(&mut command);
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
            "download failed: {}{}",
            first_stderr_line(&output.stderr),
            download_failure_hint(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    Ok(())
}

/// A short, actionable hint appended to a failed download, based on curl output.
///
/// The raw cause is kept for the curious; this adds what the user should try.
fn download_failure_hint(stderr: &str) -> &'static str {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("could not resolve host") || lower.contains("name or service not known") {
        " — no network access to the download host; check your connection or DNS"
    } else if lower.contains("proxy")
        || lower.contains("could not connect")
        || lower.contains("connection refused")
    {
        " — the host or proxy refused the connection; check HTTPS_PROXY"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        " — the connection timed out; retry, or check HTTPS_PROXY"
    } else if lower.contains("ssl") || lower.contains("certificate") || lower.contains("tls") {
        " — the TLS connection failed; a proxy or firewall may be intercepting it"
    } else {
        " — retry the install; if it keeps failing, check your network and HTTPS_PROXY"
    }
}

/// Verify `dest` against `expected`, removing the file on any mismatch.
///
/// Fails closed: a wrong or truncated download is deleted before the caller can
/// use it, so a corrupt archive can never become an installed tool.
fn verify_hash(dest: &Path, expected: &str, kind: HashKind) -> Result<(), String> {
    let actual = match kind {
        HashKind::Sha256 => file_sha256(dest)?,
        HashKind::Sha512 => file_sha512(dest)?,
    };
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "checksum mismatch for {} (expected {expected}, got {actual}); the download was discarded and not installed",
            dest.display()
        ));
    }
    Ok(())
}

/// Koda's cache of verified provisioning artifacts.
///
/// A sibling of the managed tools directory, keyed by the artifact's own digest
/// so two different artifacts can never collide and a cached entry is always
/// re-verified before use.
fn cache_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("cache"))
}

/// The cache file name for an expected digest: `<algorithm>-<hex>`.
fn cache_key(expected: &str, kind: HashKind) -> String {
    format!("{}-{}", kind.label(), expected.to_ascii_lowercase())
}

/// Return a verified cache entry for `expected`, removing a corrupt one.
///
/// "Exists in the cache" is never trusted: the digest is recomputed and
/// compared before the path is returned.
fn cache_lookup(dir: &Path, expected: &str, kind: HashKind) -> Result<Option<PathBuf>, String> {
    let path = dir.join(cache_key(expected, kind));
    if !path.is_file() {
        return Ok(None);
    }
    match verify_hash(&path, expected, kind) {
        Ok(()) => Ok(Some(path)),
        Err(_) => {
            // A wrong or truncated entry is discarded, never used.
            let _ = std::fs::remove_file(&path);
            Ok(None)
        }
    }
}

/// Store a verified artifact under its digest, atomically and best-effort.
fn cache_store(dir: &Path, expected: &str, source: &Path, kind: HashKind) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
    let dest = dir.join(cache_key(expected, kind));
    if dest.is_file() {
        return Ok(());
    }
    let tmp = temp_sibling(&dest)?;
    if std::fs::hard_link(source, &tmp).is_err() {
        std::fs::copy(source, &tmp)
            .map_err(|err| format!("could not cache the download: {err}"))?;
    }
    atomic_replace(&tmp, &dest)
}

/// Place `src` at `dest` without destroying `dest` if the move fails.
///
/// Prefers a hard link (the cache and the managed tools share a filesystem), so
/// a cached artifact costs no extra disk; falls back to a copy.
fn link_or_copy(src: &Path, dest: &Path) -> Result<(), String> {
    let tmp = temp_sibling(dest)?;
    if std::fs::hard_link(src, &tmp).is_err() {
        std::fs::copy(src, &tmp)
            .map_err(|err| format!("could not use the cached artifact: {err}"))?;
    }
    atomic_replace(&tmp, dest)
}

/// A unique sibling path for a temporary file or directory beside `path`.
fn temp_sibling(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    Ok(parent.join(format!(".{name}.koda-{}-{unique}", std::process::id())))
}

/// Move `src` over `dest` in one step, creating `dest`'s parent if needed.
fn atomic_replace(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }
    std::fs::rename(src, dest).map_err(|err| {
        format!(
            "could not move {} into place at {}: {err}",
            src.display(),
            dest.display()
        )
    })
}

/// The proxy to pass to `curl`, chosen from the standard environment variables.
///
/// `HTTPS_PROXY`/`https_proxy` win for our all-HTTPS downloads, then
/// `ALL_PROXY`, then `HTTP_PROXY` (some proxies serve HTTPS tunnelling through
/// the plain proxy variable). An empty value is ignored.
fn proxy_from_env(env: impl Fn(&str) -> Option<String>) -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Some(value) = env(key)
            && !value.is_empty()
        {
            return Some(value);
        }
    }
    None
}

/// Forward the selected proxy environment variable to `curl`.
///
/// `curl` honours these variables itself, but passing the selected one
/// explicitly keeps Koda's intent visible and testable. This forwards the
/// variable; it does not establish or validate a live proxy.
fn apply_proxy(command: &mut Command) {
    if let Some(proxy) = proxy_from_env(|key| std::env::var(key).ok()) {
        command.arg("--proxy").arg(proxy);
    }
}

/// Free space in bytes on the filesystem containing `path`, or `None` when it
/// cannot be determined.
///
/// Uses `df -Pk` (POSIX output, 1024-byte blocks), which is available on Linux
/// and macOS. A not-yet-created path is resolved to its nearest existing
/// ancestor so a first install still gets a check.
fn free_disk_bytes(path: &Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent()?;
    }
    let output = install_command("df").arg("-Pk").arg(probe).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // The POSIX format keeps one filesystem on one line; the last line is the
    // data row and its fourth column is the available block count.
    let line = text.lines().last()?;
    let available = line.split_whitespace().nth(3)?.parse::<u64>().ok()?;
    available.checked_mul(1024)
}

/// Refuse an install when the target filesystem cannot hold it.
///
/// A clear error beats a half-written toolchain. When free space cannot be
/// measured (`df` missing or unparsable) the check is skipped rather than
/// blocking a valid install, and a size Koda does not know is not guessed.
fn ensure_disk_space(dest: &Path, needed: u64) -> Result<(), String> {
    let Some(free) = free_disk_bytes(dest) else {
        return Ok(());
    };
    if free < needed {
        return Err(format!(
            "not enough free disk space under {}: need about {}, only {} is available",
            dest.display(),
            human_bytes(needed),
            human_bytes(free)
        ));
    }
    Ok(())
}

/// Fetch and verify a pinned Eclipse Adoptium JDK release.
///
/// Adoptium's API reports the download link and its SHA-256 together, and the
/// GA list carries the patch history, so `release` pins the exact version while
/// the checksum still comes from upstream. A release Adoptium no longer
/// publishes fails closed rather than moving to a different version.
fn adoptium_jdk(release: &str, dest: &Path) -> Result<(), String> {
    let (os, arch) = adoptium_platform().ok_or_else(|| {
        "no managed JDK is published for this platform; install Java manually".to_string()
    })?;
    let feature = release
        .strip_prefix("jdk-")
        .and_then(|rest| rest.split('.').next())
        .ok_or_else(|| format!("misconfigured Adoptium release {release}"))?;
    let api = format!(
        "https://api.adoptium.net/v3/assets/feature_releases/{feature}/ga?os={os}&architecture={arch}&image_type=jdk&page_size=50&sort_order=DESC"
    );
    let body = curl_text(&api).map_err(|err| format!("could not query Adoptium: {err}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|err| format!("bad Adoptium response: {err}"))?;
    let entry = value
        .as_array()
        .and_then(|releases| {
            releases.iter().find(|entry| {
                entry.get("release_name").and_then(|name| name.as_str()) == Some(release)
            })
        })
        .ok_or_else(|| format!("Adoptium has no {release} release for {os}/{arch}"))?;
    let package = entry
        .pointer("/binaries/0/package")
        .ok_or_else(|| format!("Adoptium {release} had no JDK package for {os}/{arch}"))?;
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

/// The Runtime Identifier (RID) of the .NET SDK archive for this platform.
fn dotnet_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "osx-x64",
        ("macos", "aarch64") => "osx-arm64",
        _ => return None,
    })
}

/// Download and verify the pinned .NET SDK that OmniSharp runs on.
///
/// Microsoft publishes each channel's `releases.json` with a SHA-512 for every
/// SDK archive, so Koda downloads the versioned archive directly instead of
/// fetching and running the moving, unverified `dotnet-install.sh` script. The
/// pinned SDK version must be present in the channel or the install fails
/// closed rather than silently moving to another version.
fn dotnet_sdk(dest: &Path) -> Result<(), String> {
    let rid = dotnet_platform().ok_or_else(|| {
        "no .NET SDK is published for this platform; install the .NET SDK manually".to_string()
    })?;
    let body = curl_text(&format!(
        "https://builds.dotnet.microsoft.com/dotnet/release-metadata/{DOTNET_CHANNEL}/releases.json"
    ))
    .map_err(|err| format!("could not query the .NET release metadata: {err}"))?;
    let releases: serde_json::Value = serde_json::from_str(&body)
        .map_err(|err| format!("could not parse the .NET release metadata: {err}"))?;
    let sdk = releases
        .get("releases")
        .and_then(|value| value.as_array())
        .and_then(|releases| {
            releases.iter().find(|release| {
                release
                    .pointer("/sdk/version")
                    .and_then(|version| version.as_str())
                    == Some(DOTNET_SDK_VERSION)
            })
        })
        .and_then(|release| release.get("sdk"))
        .ok_or_else(|| {
            format!(".NET SDK {DOTNET_SDK_VERSION} is not in the {DOTNET_CHANNEL} channel")
        })?;
    let file = sdk
        .get("files")
        .and_then(|value| value.as_array())
        .and_then(|files| {
            files.iter().find(|file| {
                file.get("rid").and_then(|rid| rid.as_str()) == Some(rid)
                    && file
                        .get("name")
                        .and_then(|name| name.as_str())
                        .is_some_and(|name| name.ends_with(".tar.gz"))
            })
        })
        .ok_or_else(|| format!("the .NET SDK has no {rid} archive"))?;
    let url = file
        .get("url")
        .and_then(|url| url.as_str())
        .ok_or_else(|| "the .NET SDK archive had no URL".to_string())?;
    // Fail closed: a .NET SDK runs OmniSharp, so never install it unverified.
    let hash = file
        .get("hash")
        .and_then(|hash| hash.as_str())
        .filter(|hash| is_sha512_hex(hash))
        .ok_or_else(|| {
            "the .NET SDK archive had no SHA-512; refusing an unverified SDK".to_string()
        })?;
    download_sha512(url, dest, hash)
}

/// The Node.js distribution archive for this platform, or `None` when the
/// project publishes none.
fn node_asset() -> Option<String> {
    let (triple, extension) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "tar.xz"),
        ("linux", "aarch64") => ("linux-arm64", "tar.xz"),
        ("macos", "x86_64") => ("darwin-x64", "tar.gz"),
        ("macos", "aarch64") => ("darwin-arm64", "tar.gz"),
        ("windows", "x86_64") => ("win-x64", "zip"),
        _ => return None,
    };
    Some(format!("node-v{NODE_VERSION}-{triple}.{extension}"))
}

/// Whether Koda can provision a managed Node.js runtime here: a published
/// archive for the platform, plus `curl` and the matching archive tool.
fn node_provisionable() -> bool {
    let Some(asset) = node_asset() else {
        return false;
    };
    let archive_tool = if asset.ends_with(".zip") {
        zip_extractor().is_some()
    } else {
        locate("tar").is_some()
    };
    locate("curl").is_some() && archive_tool
}

/// Download and verify the Node.js runtime archive.
///
/// Node.js publishes one `SHASUMS256.txt` per release, so Koda reads the
/// checksum for *this* platform's asset and verifies it before extraction. The
/// archive is not unpacked here; an [`InstallStep::Extract`] follows.
fn node_runtime(dest: &Path) -> Result<(), String> {
    let asset = node_asset().ok_or_else(|| {
        "no managed Node.js is published for this platform; install Node.js manually".to_string()
    })?;
    let sums_url = format!("https://nodejs.org/dist/v{NODE_VERSION}/SHASUMS256.txt");
    let output = install_command("curl")
        .args([
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sS",
            "-L",
            "--fail",
            "--connect-timeout",
            "30",
            "--max-time",
            "60",
        ])
        .arg(&sums_url)
        .output()
        .map_err(|err| format!("could not query the Node.js checksums: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not query the Node.js checksums: {}",
            first_stderr_line(&output.stderr)
        ));
    }
    // Fail closed: an unverified runtime would be executed to install and run
    // language servers, so never fall back to "no checksum required".
    let checksum = checksum_for(&String::from_utf8_lossy(&output.stdout), &asset)
        .ok_or_else(|| format!("Node.js published no checksum for {asset}"))?;
    let url = format!("https://nodejs.org/dist/v{NODE_VERSION}/{asset}");
    download(&url, dest, Some(&checksum))
}

/// Find the published SHA-256 for `asset` in a `SHASUMS256.txt` body.
///
/// The format is `<sha256>  <name>`, one entry per line. A malformed or
/// truncated line is ignored rather than trusted, and a leading `*` (binary
/// mode) is tolerated.
fn checksum_for(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_string())
    })
}

/// The Dart SDK archive for this platform, or `None` when Google publishes none.
fn dart_sdk_asset() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "dartsdk-linux-x64-release.zip",
        ("linux", "aarch64") => "dartsdk-linux-arm64-release.zip",
        ("macos", "x86_64") => "dartsdk-macos-x64-release.zip",
        ("macos", "aarch64") => "dartsdk-macos-arm64-release.zip",
        ("windows", "x86_64") => "dartsdk-windows-x64-release.zip",
        _ => return None,
    })
}

/// Download and verify a Koda-managed Dart SDK.
///
/// Google publishes a sibling `.sha256sum` for every SDK archive, so Koda reads
/// the checksum for *this* platform's asset and verifies it before extraction.
/// Only the Dart SDK is downloaded; Flutter is not, because the analysis server
/// is part of Dart and a Dart-only project does not need it.
fn dart_sdk(dest: &Path) -> Result<(), String> {
    let asset = dart_sdk_asset().ok_or_else(|| {
        "no managed Dart SDK is published for this platform; install the Dart SDK manually"
            .to_string()
    })?;
    let base = format!(
        "https://storage.googleapis.com/dart-archive/channels/stable/release/{DART_SDK_VERSION}/sdk/{asset}"
    );
    let sums_url = format!("{base}.sha256sum");
    let output = install_command("curl")
        .args([
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sS",
            "-L",
            "--fail",
            "--connect-timeout",
            "30",
            "--max-time",
            "60",
        ])
        .arg(&sums_url)
        .output()
        .map_err(|err| format!("could not query the Dart SDK checksum: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not query the Dart SDK checksum: {}",
            first_stderr_line(&output.stderr)
        ));
    }
    // Fail closed: the SDK runs as the language server, so never install it
    // without a verified checksum.
    let checksum = checksum_for(&String::from_utf8_lossy(&output.stdout), asset)
        .ok_or_else(|| format!("Dart published no checksum for {asset}"))?;
    download(&base, dest, Some(&checksum))
}

/// Fetch a small text/JSON document over HTTPS with `curl`.
///
/// Used to read checksums and API responses; it never writes to disk, and the
/// body is bounded. No `Accept` header is sent, so it works with any HTTPS
/// JSON/txt endpoint; use [`curl_text_github`] for the GitHub API.
fn curl_text(url: &str) -> Result<String, String> {
    curl_text_with_accept(url, None)
}

/// Fetch a GitHub API document with the vendor media type GitHub expects.
fn curl_text_github(url: &str) -> Result<String, String> {
    curl_text_with_accept(url, Some("application/vnd.github+json"))
}

/// Fetch a small document with `curl`, optionally sending an `Accept` header.
///
/// A successful fetch refreshes a URL-keyed cache of the body; if the network is
/// unavailable the cached body is used instead. The digest inside cached
/// metadata is still applied to the artifact, so a cached checksum can only make
/// an offline install possible, never skip verification.
fn curl_text_with_accept(url: &str, accept: Option<&str>) -> Result<String, String> {
    match curl_text_now(url, accept) {
        Ok(body) => {
            let _ = remember_metadata(url, &body);
            Ok(body)
        }
        Err(error) => match cached_metadata(url) {
            Some(body) => Ok(body),
            None => Err(error),
        },
    }
}

/// Fetch a small document with `curl` and no fallback.
fn curl_text_now(url: &str, accept: Option<&str>) -> Result<String, String> {
    let mut command = install_command("curl");
    command.args([
        "--proto",
        "=https",
        "--tlsv1.2",
        "-sS",
        "-L",
        "--fail",
        "--compressed",
        "--connect-timeout",
        "30",
        "--max-time",
        "120",
    ]);
    if let Some(accept) = accept {
        command.arg("-H").arg(format!("Accept: {accept}"));
    }
    // The .NET release metadata is ~1 MB and grows with each patch, so leave
    // headroom while keeping the body bounded.
    command.arg("--max-filesize").arg("4194304").arg(url);
    let output = command
        .output()
        .map_err(|err| format!("could not run curl: {err}"))?;
    if !output.status.success() {
        return Err(first_stderr_line(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The cache path for a metadata document, keyed by its URL.
fn metadata_cache_path(url: &str) -> Option<PathBuf> {
    Some(
        cache_dir()?
            .join("metadata")
            .join(format!("{:016x}", fnv1a(url))),
    )
}

/// Remember a fetched metadata body for offline fallback, atomically.
fn remember_metadata(url: &str, body: &str) -> Result<(), String> {
    let Some(path) = metadata_cache_path(url) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }
    let tmp = temp_sibling(&path)?;
    std::fs::write(&tmp, body).map_err(|err| format!("could not cache metadata: {err}"))?;
    atomic_replace(&tmp, &path)
}

/// A previously fetched metadata body for `url`, if one was cached.
fn cached_metadata(url: &str) -> Option<String> {
    let path = metadata_cache_path(url)?;
    std::fs::read_to_string(path).ok()
}

/// A small, stable, dependency-free hash used to name metadata cache entries.
fn fnv1a(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Download a file and verify it against a detached GPG signature.
///
/// The keys are imported into a private keyring under Koda's tools directory, so
/// the user's own GPG configuration is never touched. Verification is
/// fail-closed: a missing `gpg`, a missing signature, or a signature that does
/// not match leaves no downloaded file behind.
fn download_gpg(url: &str, dest: &Path, signature_url: &str, keys_url: &str) -> Result<(), String> {
    if locate("gpg").is_none() {
        return Err(
            "gpg is required to verify this download — install gnupg and try again".to_string(),
        );
    }
    let Some(home) = gnupg_home() else {
        return Err("could not locate Koda's data directory for a GPG keyring".to_string());
    };
    let Some(downloads) = downloads_dir() else {
        return Err("could not locate Koda's download directory".to_string());
    };
    let keys = downloads.join("release-keys.asc");
    let signature = append_extension(dest, "sig");
    std::fs::create_dir_all(&downloads)
        .map_err(|err| format!("could not create {}: {err}", downloads.display()))?;

    // Offline reuse: if the archive, its signature and the keys are all already
    // present, verify them before making any network request. The signature is
    // kept after a successful install precisely so this path works offline.
    if dest.is_file()
        && signature.is_file()
        && keys.is_file()
        && verify_signature(&home, &keys, &signature, dest).is_ok()
    {
        return Ok(());
    }

    // Fetch the keys and the signature first, then the archive, so a failure
    // never leaves a large unverified file on disk.
    download(keys_url, &keys, None)?;
    download(signature_url, &signature, None)?;
    // Reuse an archive that already verifies, so re-running an install (or
    // recovering from a network failure) does not re-download a large toolchain.
    if dest.is_file() && verify_signature(&home, &keys, &signature, dest).is_ok() {
        return Ok(());
    }
    // A stale or partial file cannot be trusted; start the download fresh.
    let _ = std::fs::remove_file(dest);
    download(url, dest, None)?;

    let result = verify_signature(&home, &keys, &signature, dest);
    if result.is_err() {
        // Never keep an archive whose provenance could not be proven.
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// Import `keys` into the isolated keyring at `home` and check `signature`
/// against `file`.
fn verify_signature(home: &Path, keys: &Path, signature: &Path, file: &Path) -> Result<(), String> {
    std::fs::create_dir_all(home)
        .map_err(|err| format!("could not create {}: {err}", home.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700));
    }

    let imported = install_command("gpg")
        .env("GNUPGHOME", home)
        .args(["--batch", "--quiet", "--import"])
        .arg(keys)
        .output()
        .map_err(|err| format!("could not run gpg: {err}"))?;
    if !imported.status.success() {
        return Err(format!(
            "could not import the release key: {}",
            first_stderr_line(&imported.stderr)
        ));
    }

    let verified = install_command("gpg")
        .env("GNUPGHOME", home)
        .args(["--batch", "--status-fd", "1", "--verify"])
        .arg(signature)
        .arg(file)
        .output()
        .map_err(|err| format!("could not run gpg: {err}"))?;
    let status = String::from_utf8_lossy(&verified.stdout);
    let trusted = status.contains("VALIDSIG") || status.contains("GOODSIG");
    if !verified.status.success() || !trusted {
        return Err(format!(
            "signature verification failed for {}: {}",
            file.display(),
            first_stderr_line(&verified.stderr)
        ));
    }
    Ok(())
}

/// The SHA-256 GitHub's API reports for a release asset, if present.
///
/// GitHub added a `digest` field (`sha256:<hex>`) to release assets, which lets
/// Koda verify an asset without a separately published checksum file.
fn github_asset_sha256(body: &str, asset: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let assets = value.get("assets")?.as_array()?;
    for entry in assets {
        if entry.get("name").and_then(|name| name.as_str()) != Some(asset) {
            continue;
        }
        let digest = entry.get("digest").and_then(|digest| digest.as_str())?;
        let hex = digest.strip_prefix("sha256:")?;
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hex.to_string());
        }
    }
    None
}

/// Download a pinned GitHub release asset, verifying the digest the API reports.
fn github_release(repo: &str, tag: &str, asset: &str, dest: &Path) -> Result<(), String> {
    let api = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    let body = curl_text_github(&api)
        .map_err(|err| format!("could not query the {repo} release metadata: {err}"))?;
    // Fail closed: an unverified asset is never run.
    let checksum = github_asset_sha256(&body, asset)
        .ok_or_else(|| format!("GitHub published no checksum digest for {asset}"))?;
    let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
    download(&url, dest, Some(&checksum))
}

/// Create Koda's Swift compatibility directory.
///
/// The official toolchain's only mismatches with a non-native distribution are
/// the ncurses soname (Koda's build expects `libncurses.so.6`, the system ships
/// `libncursesw.so.6`, the same library) and `libxml2.so.2`, which some rolling
/// distributions provide through a compatibility package. Nothing in the system
/// is copied or modified: the directory holds links that a scoped
/// `LD_LIBRARY_PATH` uses to launch `sourcekit-lsp`.
fn swift_compat(dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|err| format!("could not create {}: {err}", dest.display()))?;
    // Alias the narrow ncurses-family sonames to the wide libraries the system
    // actually ships.
    for narrow in ["libncurses.so.6", "libpanel.so.6", "libform.so.6"] {
        if system_library(&[narrow]).is_some() {
            continue;
        }
        let wide = narrow.replace(".so.6", "w.so.6");
        let Some(target) = system_library(&[&wide]) else {
            return Err(format!(
                "{narrow} is required by the Swift toolchain but neither it nor {wide} exists"
            ));
        };
        link_into(dest, narrow, &target)?;
    }
    let Some(xml) = system_library(&["libxml2.so.2"]) else {
        return Err(
            "libxml2.so.2 is required by the Swift toolchain; install your distribution's \
             libxml2 compatibility package (on Arch: libxml2-legacy)"
                .to_string(),
        );
    };
    link_into(dest, "libxml2.so.2", &xml)?;
    Ok(())
}

/// Create (or refresh) `dir/name` as a symlink to `target`.
#[cfg(unix)]
fn link_into(dir: &Path, name: &str, target: &Path) -> Result<(), String> {
    let link = dir.join(name);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(target, &link).map_err(|err| {
        format!(
            "could not link {} → {}: {err}",
            link.display(),
            target.display()
        )
    })
}

#[cfg(not(unix))]
fn link_into(_dir: &Path, _name: &str, _target: &Path) -> Result<(), String> {
    Err("the Swift compatibility layer is only used on Linux".to_string())
}

/// Mark a downloaded single-file program as executable.
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path)
            .map_err(|err| format!("could not stat {}: {err}", path.display()))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        std::fs::set_permissions(path, permissions)
            .map_err(|err| format!("could not make {} executable: {err}", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// The `go.dev` target names for this host, if Go publishes a build for it.
fn go_platform() -> Option<(&'static str, &'static str)> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux", "amd64"),
        ("linux", "aarch64") => ("linux", "arm64"),
        ("macos", "x86_64") => ("darwin", "amd64"),
        ("macos", "aarch64") => ("darwin", "arm64"),
        ("windows", "x86_64") => ("windows", "amd64"),
        ("windows", "aarch64") => ("windows", "arm64"),
        _ => return None,
    })
}

/// Download the pinned official Go toolchain.
///
/// `go.dev/dl/?mode=json&include=all` reports every release and the SHA-256 of
/// each archive, so Koda selects `GO_VERSION` and verifies against upstream
/// metadata instead of tracking the latest stable. The archive root is `go/`, so
/// a later [`InstallStep::Extract`] with `strip: 1` yields `go/bin/go`.
fn go_toolchain(dest: &Path) -> Result<(), String> {
    let (os, arch) = go_platform()
        .ok_or_else(|| "no official Go build is published for this platform".to_string())?;
    let body = curl_text("https://go.dev/dl/?mode=json&include=all")
        .map_err(|err| format!("could not query the Go release metadata: {err}"))?;
    let releases: serde_json::Value = serde_json::from_str(&body)
        .map_err(|err| format!("could not parse the Go release metadata: {err}"))?;
    let release = releases
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|release| release.get("version").and_then(|v| v.as_str()) == Some(GO_VERSION))
        })
        .ok_or_else(|| format!("go.dev does not publish {GO_VERSION}"))?;
    let files = release
        .get("files")
        .and_then(|files| files.as_array())
        .ok_or_else(|| "the Go release has no files".to_string())?;
    let mut filename = None;
    let mut sha256 = None;
    for file in files {
        let matches = file.get("os").and_then(|v| v.as_str()) == Some(os)
            && file.get("arch").and_then(|v| v.as_str()) == Some(arch)
            && file.get("kind").and_then(|v| v.as_str()) == Some("archive");
        if matches {
            filename = file
                .get("filename")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            sha256 = file
                .get("sha256")
                .and_then(|v| v.as_str())
                .filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string);
        }
    }
    let filename = filename.ok_or_else(|| format!("Go published no {os}/{arch} archive"))?;
    // Fail closed: Go runs Koda's tooling, so never install it unverified.
    let sha256 = sha256.ok_or_else(|| format!("Go published no checksum for {filename}"))?;
    download(
        &format!("https://go.dev/dl/{filename}"),
        dest,
        Some(&sha256),
    )
}

/// A sibling path with an extra extension appended (`a.tar.gz` → `a.tar.gz.sig`).
fn append_extension(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Read `/etc/os-release` into `(ID, ID_LIKE, VERSION_ID)`.
///
/// This is how Koda picks the right toolchain artifact for the running
/// distribution without guessing from a kernel version.
fn os_release() -> Option<(String, String, String)> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let mut id = String::new();
    let mut id_like = String::new();
    let mut version = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key {
            "ID" => id = value,
            "ID_LIKE" => id_like = value,
            "VERSION_ID" => version = value,
            _ => {}
        }
    }
    if id.is_empty() {
        return None;
    }
    Some((id, id_like, version))
}

/// Map a distribution to the swift.org platform tag and artifact suffix.
///
/// swift.org publishes one toolchain per distribution release, and the official
/// binaries link against that distribution's libraries. Mapping the running
/// system (including common derivatives through `ID_LIKE`) is what makes a
/// managed install reliable instead of a shot in the dark.
fn swift_platform(
    id: &str,
    id_like: &str,
    version: &str,
    arch: &str,
) -> Option<(&'static str, &'static str)> {
    if arch != "x86_64" && arch != "aarch64" {
        return None;
    }
    let candidates = std::iter::once(id).chain(id_like.split_whitespace());
    for candidate in candidates {
        let mapped = match (candidate, version) {
            ("ubuntu", "22.04") => ("ubuntu2204", "ubuntu22.04"),
            ("ubuntu", "24.04") => ("ubuntu2404", "ubuntu24.04"),
            ("ubuntu", "26.04") => ("ubuntu2604", "ubuntu26.04"),
            ("debian", "12") => ("debian12", "debian12"),
            ("debian", "13") => ("debian13", "debian13"),
            ("fedora", "39") => ("fedora39", "fedora39"),
            ("fedora", "41") => ("fedora41", "fedora41"),
            ("amzn", "2") => ("amazonlinux2", "amazonlinux2"),
            ("amzn", "2023") => ("amazonlinux2023", "amazonlinux2023"),
            _ => continue,
        };
        return Some(mapped);
    }
    None
}

/// The swift.org platform tag for a distribution it builds for directly.
fn swift_native_asset() -> Option<(&'static str, &'static str)> {
    if std::env::consts::OS != "linux" {
        return None;
    }
    let (id, id_like, version) = os_release()?;
    swift_platform(&id, &id_like, &version, std::env::consts::ARCH)
}

/// The Swift toolchain Koda should provision on this host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SwiftToolchain {
    /// The swift.org release directory (`ubuntu2404`, `ubi10`, …).
    tag: &'static str,
    /// The artifact suffix (`ubuntu24.04`, `ubi10`, …).
    suffix: &'static str,
    /// Whether this is the build for the running distribution (no compat needed
    /// beyond what the distribution provides).
    native: bool,
}

/// Resolve the Swift toolchain for this host.
///
/// A distribution swift.org builds for directly gets its own artifact. Any
/// other glibc Linux system gets the portable UBI10 build plus Koda's
/// compatibility layer (a ncurses soname alias and the distribution's
/// `libxml2.so.2`), which is how the maintained Arch package makes the same
/// toolchain run. musl systems and non-Linux hosts get nothing.
fn swift_toolchain() -> Option<SwiftToolchain> {
    if let Some((tag, suffix)) = swift_native_asset() {
        return Some(SwiftToolchain {
            tag,
            suffix,
            native: true,
        });
    }
    if std::env::consts::OS != "linux" || libc_is_musl() {
        return None;
    }
    let (tag, suffix) = match std::env::consts::ARCH {
        "x86_64" => ("ubi10", "ubi10"),
        "aarch64" => ("ubi10-aarch64", "ubi10-aarch64"),
        _ => return None,
    };
    Some(SwiftToolchain {
        tag,
        suffix,
        native: false,
    })
}

/// Libraries the Swift toolchain needs that this system does not provide.
///
/// `libncurses.so.6` is satisfied by the wide `libncursesw.so.6` (the same
/// library; the soname differs because Koda's build is not the system's).
/// `libxml2.so.2` is a genuine ABI version some rolling distributions moved past
/// and provide through a compatibility package.
fn swift_missing_libs() -> Vec<&'static str> {
    let mut missing = Vec::new();
    if system_library(&["libncurses.so.6", "libncursesw.so.6"]).is_none() {
        missing.push("libncurses.so.6");
    }
    if system_library(&["libxml2.so.2"]).is_none() {
        missing.push("libxml2.so.2");
    }
    missing
}

/// Whether the running distribution is Arch or an Arch derivative.
fn distro_is_arch() -> bool {
    match os_release() {
        Some((id, id_like, _)) => {
            id == "arch" || id_like.split_whitespace().any(|like| like == "arch")
        }
        None => false,
    }
}

/// Find a system shared library by soname in the usual library directories.
fn system_library(names: &[&str]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/lib64"),
        PathBuf::from("/lib"),
        PathBuf::from("/lib64"),
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/usr/local/lib64"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu"),
        PathBuf::from("/usr/lib/aarch64-linux-gnu"),
    ];
    // A user-writable location lets a rootless user supply a compatibility
    // library it cannot install system-wide.
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/lib"));
        dirs.push(PathBuf::from(home).join(".local/lib64"));
    }
    dirs.iter().find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// The `builds.hex.pm` platform directory for this host, if `bob` publishes one.
fn bob_platform() -> Option<&'static str> {
    if std::env::consts::OS != "linux" {
        return None;
    }
    let Some((id, id_like, version)) = os_release() else {
        return bob_fallback_platform();
    };
    bob_platform_for(&id, &id_like, &version).or_else(bob_fallback_platform)
}

/// A best-effort `bob` platform for glibc distributions it does not name.
///
/// `bob`'s Ubuntu 24.04 builds link against a modern glibc and OpenSSL 3, which
/// current rolling distributions ship, so they run on many systems that have no
/// explicit build. Koda still verifies the result by launching the real server,
/// so an incompatible system fails with a clear message instead of a false
/// success. musl systems (Alpine) are excluded because the glibc build cannot
/// run there at all.
fn bob_fallback_platform() -> Option<&'static str> {
    if libc_is_musl() {
        return None;
    }
    Some("ubuntu-24.04")
}

/// Map a distribution to the `bob` build directory (see [`bob_platform`]).
fn bob_platform_for(id: &str, id_like: &str, version: &str) -> Option<&'static str> {
    let candidates = std::iter::once(id).chain(id_like.split_whitespace());
    for candidate in candidates {
        let mapped = match (candidate, version) {
            ("ubuntu", "20.04") => "ubuntu-20.04",
            ("ubuntu", "22.04") => "ubuntu-22.04",
            ("ubuntu", "24.04") => "ubuntu-24.04",
            ("debian", "11") => "debian-11",
            ("debian", "12") => "debian-12",
            ("debian", "13") => "debian-13",
            ("fedora", "39") => "fedora-39",
            ("fedora", "41") => "fedora-41",
            _ => continue,
        };
        return Some(mapped);
    }
    None
}

/// The major version of an OTP release (`27.3.4` → `27`).
fn otp_major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// Download and verify an Erlang/OTP or Elixir build from `builds.hex.pm`.
///
/// The service publishes the SHA-256 next to the build in its `builds.txt`, so
/// the checksum is read for the pinned version and verified before anything is
/// extracted. The archive still has to be extracted by a later step.
fn bob_build(package: BobPackage, dest: &Path) -> Result<(), String> {
    let platform = bob_platform().ok_or_else(|| {
        "no Erlang/Elixir build is published for this platform; install Erlang and Elixir manually"
            .to_string()
    })?;
    let (key, file, sums_url, base) = match package {
        BobPackage::Erlang => (
            format!("OTP-{OTP_VERSION}"),
            format!("OTP-{OTP_VERSION}.tar.gz"),
            format!("https://builds.hex.pm/builds/otp/{platform}/builds.txt"),
            format!("https://builds.hex.pm/builds/otp/{platform}"),
        ),
        BobPackage::Elixir => {
            let version = format!("v{ELIXIR_VERSION}-otp-{}", otp_major(OTP_VERSION));
            (
                version.clone(),
                format!("{version}.zip"),
                "https://builds.hex.pm/builds/elixir/builds.txt".to_string(),
                "https://builds.hex.pm/builds/elixir".to_string(),
            )
        }
    };
    let body = curl_text(&sums_url)?;
    let expected = builds_txt_checksum(&body, &key)
        .ok_or_else(|| format!("builds.hex.pm published no checksum for {key}"))?;
    // Fail closed: an unverified runtime is never extracted or run.
    download(&format!("{base}/{file}"), dest, Some(&expected))
}

/// Find the SHA-256 for `key` in a `builds.hex.pm` `builds.txt` body.
///
/// The body is one build per line, `<name> <git-sha> <timestamp> <sha256>`;
/// older entries omit the checksum column and are treated as unavailable.
fn builds_txt_checksum(body: &str, key: &str) -> Option<String> {
    body.lines()
        .find(|line| line.split_whitespace().next() == Some(key))
        .and_then(|line| line.split_whitespace().nth(3))
        .filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string)
}

/// Refuse an archive whose entries could escape the extraction directory.
///
/// GNU `tar` and Info-ZIP `unzip` sanitize `..` and absolute paths, but a
/// minimal system's tools may not, and a downloaded archive is untrusted input.
/// The listing is bounded by the archive's own entry count and any failure to
/// list is treated as unsafe, so this fails closed.
fn ensure_archive_paths_safe(archive: &Path, zip: bool) -> Result<(), String> {
    let listing = archive_listing(archive, zip)?;
    for line in listing.lines() {
        let entry = line.trim_end_matches('/');
        if entry.starts_with('/') || entry.starts_with('\\') {
            return Err(format!(
                "{} contains an absolute path: {entry}",
                archive.display()
            ));
        }
        if entry.split(['/', '\\']).any(|part| part == "..") {
            return Err(format!(
                "{} contains a parent path: {entry}",
                archive.display()
            ));
        }
    }
    Ok(())
}

/// List an archive's entry names using whichever tool can read it.
///
/// Zip archives are listed with the same extractor [`zip_extractor`] selects, so
/// a system without `unzip` is not refused by the safety check alone. A listing
/// that cannot be produced fails closed.
fn archive_listing(archive: &Path, zip: bool) -> Result<String, String> {
    if !zip {
        return listing_from(
            install_command("tar").arg("-tf").arg(archive).output(),
            archive,
        );
    }
    let extractor = zip_extractor().ok_or_else(|| {
        format!(
            "no zip extractor found (looked for unzip, bsdtar and python3) to inspect {}",
            archive.display()
        )
    })?;
    zip_listing(extractor, archive)
}

/// List a zip archive's entries with a specific extractor.
fn zip_listing(extractor: ZipExtractor, archive: &Path) -> Result<String, String> {
    let output = match extractor {
        ZipExtractor::Unzip => install_command("unzip").arg("-Z1").arg(archive).output(),
        ZipExtractor::BsdTar => install_command("bsdtar").arg("-tf").arg(archive).output(),
        ZipExtractor::Python => install_command("python3")
            .args([
                "-c",
                "import sys,zipfile;print('\\n'.join(zipfile.ZipFile(sys.argv[1]).namelist()))",
            ])
            .arg(archive)
            .output(),
    };
    listing_from(output, archive)
}

/// Return a listing command's stdout, or a failure that names the archive.
fn listing_from(
    output: std::io::Result<std::process::Output>,
    archive: &Path,
) -> Result<String, String> {
    let output = output.map_err(|err| format!("could not list {}: {err}", archive.display()))?;
    if !output.status.success() {
        return Err(format!(
            "could not list {}: {}",
            archive.display(),
            first_stderr_line(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Extract a `.tar.gz`/`.tar.xz`/`.zip` archive into `dest`.
///
/// `tar` supports `--strip-components`; `unzip` does not, so a stripped zip is
/// unpacked into a scratch directory and the leading path components are moved
/// up by hand.
fn extract(archive: &Path, dest: &Path, strip: usize) -> Result<(), String> {
    // Unpack into a staging directory beside the destination, then promote it in
    // one same-filesystem rename. A failed extraction leaves the previous
    // installation untouched, and a failed promotion restores it.
    let staging = temp_sibling(dest)?;
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|err| format!("could not create {}: {err}", staging.display()))?;

    let result = (|| {
        // The archive's own size is a real lower bound for the unpacked tree;
        // there is no separate extracted-size estimate, so do not invent one.
        if let Ok(metadata) = std::fs::metadata(archive) {
            ensure_disk_space(&staging, metadata.len())?;
        }
        let zip = archive.extension().and_then(|ext| ext.to_str()) == Some("zip");
        // Refuse an archive that could write outside the staging directory.
        ensure_archive_paths_safe(archive, zip)?;

        if zip {
            return extract_zip(archive, &staging, strip);
        }
        let mut command = install_command("tar");
        command.arg("-xf").arg(archive).arg("-C").arg(&staging);
        if strip > 0 {
            command.arg(format!("--strip-components={strip}"));
        }
        match command.output() {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!(
                "could not extract {}: {}",
                archive.display(),
                first_stderr_line(&output.stderr)
            )),
            Err(err) => Err(format!("could not run the archive tool: {err}")),
        }
    })();

    if let Err(err) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }
    promote_directory(&staging, dest)
}

/// Move a staged directory into its final location without destroying the
/// previous installation if the move fails.
///
/// The existing directory is renamed aside first, then restored if the promotion
/// fails. On success the backup is removed.
fn promote_directory(staging: &Path, dest: &Path) -> Result<(), String> {
    let backup = temp_sibling(dest)?;
    let _ = std::fs::remove_dir_all(&backup);
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &backup).map_err(|err| {
            let _ = std::fs::remove_dir_all(staging);
            format!("could not set aside the existing {}: {err}", dest.display())
        })?;
    }
    match std::fs::rename(staging, dest) {
        Ok(()) => {
            if had_old {
                let _ = std::fs::remove_dir_all(&backup);
            }
            Ok(())
        }
        Err(err) => {
            if had_old {
                // Put the known-good installation back.
                let _ = std::fs::rename(&backup, dest);
            }
            let _ = std::fs::remove_dir_all(staging);
            Err(format!("could not install into {}: {err}", dest.display()))
        }
    }
}

/// Which tool Koda uses to unpack a zip archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZipExtractor {
    Unzip,
    BsdTar,
    Python,
}

/// The first available zip extractor: Info-ZIP `unzip`, then `bsdtar` (libarchive,
/// the macOS default), then a Python interpreter. This keeps zip-based bundles
/// installable on minimal systems without `unzip`.
fn zip_extractor() -> Option<ZipExtractor> {
    if locate("unzip").is_some() {
        Some(ZipExtractor::Unzip)
    } else if locate("bsdtar").is_some() {
        Some(ZipExtractor::BsdTar)
    } else if locate("python3").is_some() {
        Some(ZipExtractor::Python)
    } else {
        None
    }
}

/// Extract a zip archive, using whichever extractor is available.
fn extract_zip(archive: &Path, dest: &Path, strip: usize) -> Result<(), String> {
    let extractor = zip_extractor().ok_or_else(|| {
        format!(
            "no zip extractor found (looked for unzip, bsdtar and python3) to unpack {}",
            archive.display()
        )
    })?;
    extract_zip_with(extractor, archive, dest, strip)
}

/// Extract a zip archive with a specific extractor.
fn extract_zip_with(
    extractor: ZipExtractor,
    archive: &Path,
    dest: &Path,
    strip: usize,
) -> Result<(), String> {
    std::fs::create_dir_all(dest)
        .map_err(|err| format!("could not create {}: {err}", dest.display()))?;
    if extractor == ZipExtractor::BsdTar {
        // libarchive understands `--strip-components` for zip archives.
        let mut command = install_command("bsdtar");
        command.arg("-xf").arg(archive).arg("-C").arg(dest);
        if strip > 0 {
            command.arg(format!("--strip-components={strip}"));
        }
        return match command.output() {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!(
                "could not extract {}: {}",
                archive.display(),
                first_stderr_line(&output.stderr)
            )),
            Err(err) => Err(format!("could not run bsdtar: {err}")),
        };
    }

    // `unzip` and Python have no `--strip-components`, so unpack into a scratch
    // directory and move the wanted level up by hand.
    let scratch = dest.with_extension("koda-extract");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .map_err(|err| format!("could not create {}: {err}", scratch.display()))?;
    let result = (|| {
        let output = match extractor {
            ZipExtractor::Unzip => install_command("unzip")
                .args(["-q", "-o"])
                .arg(archive)
                .arg("-d")
                .arg(&scratch)
                .output()
                .map_err(|err| format!("could not run unzip: {err}"))?,
            ZipExtractor::Python => install_command("python3")
                .args(["-m", "zipfile", "-e"])
                .arg(archive)
                .arg(&scratch)
                .output()
                .map_err(|err| format!("could not run python3: {err}"))?,
            ZipExtractor::BsdTar => unreachable!(),
        };
        if !output.status.success() {
            return Err(format!(
                "could not extract {}: {}",
                archive.display(),
                first_stderr_line(&output.stderr)
            ));
        }
        let mut root = scratch.clone();
        for _ in 0..strip {
            root = single_child_directory(&root).ok_or_else(|| {
                format!(
                    "archive {} does not have {strip} leading directory level(s)",
                    archive.display()
                )
            })?;
        }
        move_directory_contents(&root, dest)
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// The only child of `dir` when it is a directory, else `None`.
fn single_child_directory(dir: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(dir).ok()?.flatten();
    let first = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    let path = first.path();
    path.is_dir().then_some(path)
}

/// Move every entry of `from` into `to`, replacing same-named entries.
fn move_directory_contents(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to)
        .map_err(|err| format!("could not create {}: {err}", to.display()))?;
    let entries = std::fs::read_dir(from)
        .map_err(|err| format!("could not read {}: {err}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("could not read {}: {err}", from.display()))?;
        let target = to.join(entry.file_name());
        if target.exists() {
            if target.is_dir() {
                std::fs::remove_dir_all(&target)
            } else {
                std::fs::remove_file(&target)
            }
            .map_err(|err| format!("could not replace {}: {err}", target.display()))?;
        }
        std::fs::rename(entry.path(), &target).map_err(|err| {
            format!(
                "could not move {} into {}: {err}",
                entry.path().display(),
                to.display()
            )
        })?;
    }
    Ok(())
}

/// SHA-256 of a file, using whichever tool the platform provides.
fn file_sha256(path: &Path) -> Result<String, String> {
    // `sha256sum` (coreutils, busybox) and `shasum` (macOS) print the hex first.
    for (program, args) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        if let Ok(output) = install_command(program).args(args).arg(path).output()
            && output.status.success()
            && let Some(hash) = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
            && is_sha256_hex(hash)
        {
            return Ok(hash.to_string());
        }
    }
    // `openssl dgst -sha256` prints `SHA256(name)= <hex>`.
    if let Ok(output) = install_command("openssl")
        .args(["dgst", "-sha256"])
        .arg(path)
        .output()
        && output.status.success()
        && let Some(hash) = String::from_utf8_lossy(&output.stdout)
            .split('=')
            .nth(1)
            .map(str::trim)
        && is_sha256_hex(hash)
    {
        return Ok(hash.to_string());
    }
    // A Python interpreter, present on many minimal systems without coreutils.
    if let Ok(output) = install_command("python3")
        .args([
            "-c",
            "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())",
        ])
        .arg(path)
        .output()
        && output.status.success()
        && let Some(hash) = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
        && is_sha256_hex(hash)
    {
        return Ok(hash.to_string());
    }
    Err("no SHA-256 tool found (looked for sha256sum, shasum, openssl and python3)".to_string())
}

/// Whether `value` is a 64-character lowercase-or-uppercase hex SHA-256.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `value` is a 128-character lowercase-or-uppercase hex SHA-512.
fn is_sha512_hex(value: &str) -> bool {
    value.len() == 128 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Compute a file's SHA-512 with whichever hashing tool the system provides.
///
/// Same fail-closed fallback chain as [`file_sha256`].
fn file_sha512(path: &Path) -> Result<String, String> {
    for (program, args) in [("sha512sum", &[][..]), ("shasum", &["-a", "512"][..])] {
        if let Ok(output) = install_command(program).args(args).arg(path).output()
            && output.status.success()
            && let Some(hash) = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
            && is_sha512_hex(hash)
        {
            return Ok(hash.to_string());
        }
    }
    if let Ok(output) = install_command("openssl")
        .args(["dgst", "-sha512"])
        .arg(path)
        .output()
        && output.status.success()
        && let Some(hash) = String::from_utf8_lossy(&output.stdout)
            .split('=')
            .nth(1)
            .map(str::trim)
        && is_sha512_hex(hash)
    {
        return Ok(hash.to_string());
    }
    if let Ok(output) = install_command("python3")
        .args([
            "-c",
            "import hashlib,sys;print(hashlib.sha512(open(sys.argv[1],'rb').read()).hexdigest())",
        ])
        .arg(path)
        .output()
        && output.status.success()
        && let Some(hash) = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
        && is_sha512_hex(hash)
    {
        return Ok(hash.to_string());
    }
    Err("no SHA-512 tool found (looked for sha512sum, shasum, openssl and python3)".to_string())
}

/// Render a byte count for a download warning (`1.1 GB`, `240 MB`).
pub fn human_bytes(bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes.div_ceil(MB))
    }
}

fn first_stderr_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("installation failed")
        .trim()
        .to_string()
}

/// The most useful line of a failed install command's stderr.
///
/// Package managers print the concrete error (the missing module, the compiler
/// error) before a trailing summary, so prefer the last line that names a
/// failure and otherwise fall back to the last line.
fn last_stderr_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines
        .iter()
        .rev()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("error")
                || lower.contains("failed")
                || lower.contains("fatal")
                || lower.contains("cannot")
                || lower.contains("not found")
        })
        .or_else(|| lines.last())
        .map(|line| line.trim().to_string())
        .unwrap_or_else(|| "installation failed".to_string())
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
            locate(&command.program).is_some()
                || is_user_bin_program(&command.program)
                // A program supplied by a Koda-managed runtime an earlier step
                // in the same attempt installs (for example `elixir` before the
                // ElixirLS build).
                || is_koda_provided_program(&command.program)
        }
        InstallStep::Download { .. }
        | InstallStep::AdoptiumJdk { .. }
        | InstallStep::DotnetSdk { .. }
        | InstallStep::NodeRuntime { .. }
        | InstallStep::DartSdk { .. }
        | InstallStep::GithubRelease { .. } => locate("curl").is_some(),
        InstallStep::RustupInit { .. } => locate("curl").is_some() && rustup_target().is_some(),
        // The official Go toolchain is resolved from a published JSON document
        // at install time; only `curl` and a known platform are needed here.
        InstallStep::GoToolchain { .. } => locate("curl").is_some() && go_platform().is_some(),
        // A file can always be marked executable on a Unix host.
        InstallStep::MakeExecutable { .. } => cfg!(unix),
        // A `bob` build also needs a supported platform directory.
        InstallStep::BobBuild { .. } => locate("curl").is_some() && bob_platform().is_some(),
        // The compatibility layer can only be built when every library it needs
        // is available on the system.
        InstallStep::SwiftCompat { .. } => swift_missing_libs().is_empty(),
        // A signature-verified download also needs `gpg`.
        InstallStep::DownloadGpg { .. } => locate("curl").is_some() && locate("gpg").is_some(),
        InstallStep::Extract { archive, .. } => {
            let zip = archive.extension().and_then(|ext| ext.to_str()) == Some("zip");
            if zip {
                // Any of `unzip`, `bsdtar` or Python can unpack a zip.
                zip_extractor().is_some()
            } else {
                locate("tar").is_some()
            }
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

/// Whether a program is supplied by a runtime Koda installs itself.
///
/// An install attempt may run one of these after an earlier step has produced
/// it, so a missing binary is not a reason to withhold the offer.
fn is_koda_provided_program(program: &str) -> bool {
    matches!(
        program,
        "elixir" | "mix" | "erl" | "escript" | "go" | "cargo"
    )
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
    /// Why a present-but-unusable tool failed to run (a missing shared library,
    /// a broken launcher), so install failures can explain themselves.
    pub error: Option<String>,
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

    /// Whether Koda owns this install (it lives under Koda's tools directory).
    ///
    /// Only a managed tool may be updated or removed by Koda; a user's own
    /// `rust-analyzer`, `clangd` or `gem` install is never touched.
    pub fn is_managed(&self) -> bool {
        self.path.as_deref().is_some_and(is_managed_path)
    }

    /// The on-disk size of the Koda-managed component, when Koda owns the
    /// install. User/system installs return `None`.
    pub fn managed_size(&self) -> Option<u64> {
        let dir = managed_component(self.path.as_deref()?)?;
        directory_size(&dir)
    }

    /// The managed component directory Koda would remove for this tool.
    pub fn managed_dir(&self) -> Option<PathBuf> {
        managed_component(self.path.as_deref()?)
    }
}

/// Whether `path` was installed by Koda under its own tools directory.
pub fn is_managed_path(path: &Path) -> bool {
    tools_dir().is_some_and(|tools| path.starts_with(tools))
}

/// The Koda-managed component directory containing `path`: the first path
/// segment under the tools directory.
pub fn managed_component(path: &Path) -> Option<PathBuf> {
    let tools = tools_dir()?;
    let relative = path.strip_prefix(&tools).ok()?;
    let first = relative.components().next()?;
    Some(tools.join(first))
}

/// The total size of the files under `dir`, bounded so a large tree cannot stall
/// the worker. Returns `None` only when `dir` cannot be walked at all.
pub fn directory_size(dir: &Path) -> Option<u64> {
    // A cap on visited entries keeps a pathological tree predictable; a managed
    // component is never expected to approach it.
    const MAX_ENTRIES: usize = 50_000;
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return Some(total);
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Some(total)
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

    /// Discovery for the background worker.
    ///
    /// In tests every `App` would otherwise re-probe every tool, and a parallel
    /// test binary saturates the machine spawning thousands of `--version`
    /// processes at once. The environment does not change during a test run, so
    /// the first result is shared. Production always probes fresh, preserving
    /// Koda's ability to notice a tool installed while it is running.
    pub fn discover_cached() -> Self {
        #[cfg(test)]
        {
            use std::sync::{Mutex, OnceLock};
            static CACHE: OnceLock<Mutex<Option<ToolRegistry>>> = OnceLock::new();
            let cache = CACHE.get_or_init(|| Mutex::new(None));
            let mut guard = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(registry) = guard.as_ref() {
                return registry.clone();
            }
            let registry = Self::discover();
            *guard = Some(registry.clone());
            registry
        }
        #[cfg(not(test))]
        {
            Self::discover()
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
    let Some(path) = locate_tool(tool) else {
        return ToolStatus {
            tool,
            available: false,
            version: None,
            path: None,
            error: None,
        };
    };
    let mut command = Command::new(&path);
    // Managed runtimes (the JDK for jdtls, the .NET SDK for OmniSharp) are
    // found through the launch environment.
    for (key, value) in launch_env(tool) {
        command.env(key, value);
    }
    if tool.probe_as_server() {
        return probe_server(tool, path, command);
    }
    command.args(tool.version_args());
    // A probe is bounded in both time and output so a hung or chatty tool
    // cannot wedge the worker. `stdin` is null, so `gofmt` reads an empty
    // document and exits.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return ToolStatus {
            tool,
            available: false,
            version: None,
            path: Some(path),
            error: None,
        };
    };
    match wait_captured(&mut child, PROBE_TIMEOUT, MAX_TOOL_OUTPUT) {
        Ok(captured) if captured.status.is_some_and(|status| status.success()) => {
            // Only keep a version line that actually looks like one; `--help`
            // usage output should not masquerade as a version.
            let version = String::from_utf8_lossy(&captured.stdout)
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
                error: None,
            }
        }
        Ok(captured) => ToolStatus {
            tool,
            available: false,
            version: None,
            path: Some(path),
            error: probe_error(&captured.stderr),
        },
        Err(_) => ToolStatus {
            tool,
            available: false,
            version: None,
            path: Some(path),
            error: None,
        },
    }
}

/// Verify a stdio language server by starting it and confirming it stays up.
///
/// The extracted VS Code servers reject every version query, so the only honest
/// check is to launch the real server and see that it does not exit with an
/// error. `stdin` is held open so the server initializes and waits; Koda stops
/// it once the window elapses. A server that exits successfully on its own is
/// also accepted, and one that exits with a failure status is not.
fn probe_server(tool: Tool, path: PathBuf, mut command: Command) -> ToolStatus {
    command.args(tool.server_args());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return ToolStatus {
            tool,
            available: false,
            version: None,
            path: Some(path),
            error: None,
        };
    };
    // Hold the write end so the server does not see EOF and shut down at once.
    let _stdin = child.stdin.take();
    let (available, error) = alive_or_clean(&mut child);
    ToolStatus {
        tool,
        available,
        version: None,
        path: Some(path),
        error,
    }
}

/// Whether a launched process is a usable server: still running when the window
/// elapses, or already exited cleanly. A process that fails immediately (the
/// exact symptom of a broken Node launcher or a missing shared library) is
/// rejected, and its first stderr line is kept for the error message.
fn alive_or_clean(child: &mut std::process::Child) -> (bool, Option<String>) {
    match wait_captured(child, PROBE_ALIVE_WINDOW, MAX_TOOL_OUTPUT) {
        Ok(captured) => {
            let available =
                captured.status.is_none() || captured.status.is_some_and(|status| status.success());
            let error = if available {
                None
            } else {
                probe_error(&captured.stderr)
            };
            (available, error)
        }
        Err(_) => (false, None),
    }
}

/// The first meaningful line of a failed probe's stderr, if any.
fn probe_error(stderr: &[u8]) -> Option<String> {
    let line = first_stderr_line(stderr);
    (!line.is_empty() && line != "installation failed").then_some(line)
}

/// Locate a tool's executable, trying its primary name and any alternates.
fn locate_tool(tool: Tool) -> Option<PathBuf> {
    std::iter::once(tool.program())
        .chain(tool.candidates().iter().copied())
        .find_map(locate)
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
        // Toolchains installed under the home directory rather than on the
        // login PATH: ElixirLS escripts, asdf shims, local::lib Perl, Swift and
        // a Flutter-bundled Dart SDK.
        dirs.push(home.join(".mix/escripts"));
        dirs.push(home.join(".asdf/shims"));
        dirs.push(home.join("perl5/bin"));
        dirs.push(home.join(".local/share/swiftly/bin"));
        dirs.push(home.join(".swiftly/bin"));
        dirs.push(home.join("development/flutter/bin/cache/dart-sdk/bin"));
        dirs.push(home.join(".pub-cache/bin"));
    }
    // Tools Koda installed itself, plus the npm prefix, Python virtualenv,
    // managed Node.js and managed runtimes it maintains.
    if let Some(tools) = tools_dir() {
        if let Some(prefix) = npm_prefix() {
            dirs.push(prefix.join("bin"));
            dirs.push(prefix.join("node_modules/.bin"));
            dirs.push(prefix);
        }
        if let Some(venv) = venv_dir() {
            dirs.push(venv_bin_dir(&venv));
        }
        if let Some(node) = node_bin_dir() {
            dirs.push(node);
        }
        dirs.push(tools.join("omnisharp"));
        dirs.push(tools.join("jdtls/bin"));
        dirs.push(tools.join("lua-language-server/bin"));
        dirs.push(tools.join("kotlin-language-server/bin"));
        dirs.push(tools.join("dart-sdk/bin"));
        dirs.push(tools.join("perl5/bin"));
        dirs.push(tools.join("cpanm/bin"));
        dirs.push(tools.join("swift/usr/bin"));
        dirs.push(tools.join("otp/bin"));
        dirs.push(tools.join("elixir/bin"));
        dirs.push(tools.join("elixir-ls"));
        dirs.push(tools.join("go/bin"));
        dirs.push(tools.join("gopath/bin"));
        dirs.push(tools.join("clangd/bin"));
        dirs.push(tools.join("asm-lsp"));
        dirs.push(tools.join("phpactor"));
        dirs.push(tools.join("gems/bin"));
        dirs.push(tools.join("jdk/bin"));
        dirs.push(tools.join("dotnet"));
        dirs.push(tools.join("bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/go/bin"));
    dirs.push(PathBuf::from("/usr/local/swift/usr/bin"));
    dirs.push(PathBuf::from("/usr/lib/swift/bin"));
    dirs.push(PathBuf::from("/snap/bin"));
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

/// Koda's managed Node.js runtime, used to run `npm` and the npm-based language
/// servers without requiring the user to install Node.js themselves.
pub fn node_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("node"))
}

/// The `bin` directory of Koda's managed Node.js runtime (the runtime root on
/// Windows, where `node.exe`/`npm.cmd` sit directly in the extracted folder).
fn node_bin_dir() -> Option<PathBuf> {
    let dir = node_dir()?;
    if cfg!(windows) {
        Some(dir)
    } else {
        Some(dir.join("bin"))
    }
}

/// The `npm` bundled with Koda's managed Node.js, whether or not it exists yet.
fn managed_npm() -> Option<PathBuf> {
    let dir = node_dir()?;
    Some(if cfg!(windows) {
        dir.join("npm.cmd")
    } else {
        dir.join("bin/npm")
    })
}

/// The scratch path for the managed Node.js archive.
fn node_archive() -> Option<PathBuf> {
    let dir = downloads_dir()?;
    let asset = node_asset()?;
    Some(dir.join(asset))
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

/// Koda's managed `lua-language-server` directory.
pub fn lua_ls_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("lua-language-server"))
}

/// Koda's dedicated JDK 21, used only to run `kotlin-language-server`. It is
/// kept separate from the JDK 25 that `jdtls` requires, because the Kotlin
/// compiler bundled with the server rejects the newer version string.
pub fn kotlin_jdk_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("kotlin-jdk"))
}

/// Koda's managed `kotlin-language-server` directory.
pub fn kotlin_ls_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("kotlin-language-server"))
}

/// Koda's managed Dart SDK directory.
pub fn dart_sdk_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("dart-sdk"))
}

/// Koda's isolated Perl `local::lib`, used to install `Perl::LanguageServer`
/// without touching the system Perl.
pub fn perl_local_lib_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("perl5"))
}

/// Koda's managed Swift toolchain directory. The archive extracts a `usr/`
/// subtree, so the launcher is `tools/swift/usr/bin/sourcekit-lsp`.
pub fn swift_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("swift"))
}

/// Koda's Swift compatibility directory: links to the system libraries the
/// official toolchain needs but does not ship, used through a scoped
/// `LD_LIBRARY_PATH` without modifying the system.
pub fn swift_compat_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("swift-compat"))
}

/// Koda's managed Erlang/OTP installation directory.
pub fn otp_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("otp"))
}

/// Koda's managed Elixir installation directory.
pub fn elixir_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("elixir"))
}

/// Koda's managed ElixirLS installation directory (the release launcher).
pub fn elixir_ls_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("elixir-ls"))
}

/// Koda's private GPG keyring, used only to verify downloaded release
/// signatures. Keeping it separate means the user's `~/.gnupg` is untouched and
/// a stale key in it can never be trusted by mistake.
fn gnupg_home() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("gnupg"))
}

/// Koda's private Mix/Hex home for the managed Elixir runtime and ElixirLS.
fn mix_home() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("mix"))
}

/// Where Koda keeps the checksum-verified `cpanm` it bootstraps.
fn cpanm_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("cpanm"))
}

/// Koda's managed Go toolchain (`go/bin/go`), kept isolated from `/usr/local/go`.
pub fn go_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("go"))
}

/// Koda's private `GOPATH`, so `go install` never writes to the user's `~/go`.
pub fn go_path() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("gopath"))
}

/// The `GOBIN` inside Koda's private `GOPATH`, where `gopls`/`sqls`/`shfmt`
/// are installed.
fn go_bin_dir() -> Option<PathBuf> {
    go_path().map(|dir| dir.join("bin"))
}

/// Koda's managed `clangd` directory (`bin/clangd` plus its resource headers).
pub fn clangd_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("clangd"))
}

/// Koda's managed prebuilt `asm-lsp` directory.
pub fn asm_lsp_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("asm-lsp"))
}

/// Koda's managed `phpactor.phar` directory.
pub fn phpactor_dir() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("phpactor"))
}

/// Koda's isolated Ruby gem home, so `solargraph` never pollutes the user's.
pub fn gem_home() -> Option<PathBuf> {
    tools_dir().map(|dir| dir.join("gems"))
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
        // The Kotlin compiler bundled with the server cannot parse JDK 25's
        // four-part version, so the server runs on Koda's dedicated JDK 21.
        Tool::KotlinLs => {
            if let Some(jdk) = kotlin_jdk_dir() {
                let jdk = jdk.to_string_lossy().into_owned();
                env.push(("JAVA_HOME".to_string(), jdk.clone()));
                env.push(("PATH".to_string(), prepend_path(&format!("{jdk}/bin"))));
            }
        }
        // npm-based servers are launched through `#!/usr/bin/env node`, so a
        // Koda-managed Node.js must be on `PATH` when the user has none.
        Tool::BashLs | Tool::TypeScriptLs | Tool::HtmlLs | Tool::CssLs => {
            if let Some(node) = node_bin_dir() {
                env.push(("PATH".to_string(), prepend_path(&node.to_string_lossy())));
            }
        }
        // A server installed into Koda's isolated Perl local::lib needs that
        // library and its bin directory, and nothing from the system Perl.
        Tool::PerlLs | Tool::Pls => {
            if let Some(lib) = perl_local_lib_dir() {
                let lib = lib.to_string_lossy().into_owned();
                env.push(("PERL5LIB".to_string(), format!("{lib}/lib/perl5")));
                env.push(("PATH".to_string(), prepend_path(&format!("{lib}/bin"))));
            }
        }
        // ElixirLS is a shell launcher that runs `elixir`; when Koda installed
        // it, it needs the managed OTP and Elixir runtimes on `PATH` and a
        // private Mix/Hex home so the user's `~/.mix` is never touched. A
        // system-provided ElixirLS is left on the user's own environment.
        Tool::ElixirLs => {
            if elixir_ls_dir().is_some_and(|dir| dir.is_dir()) {
                if let Some(dir) = elixir_ls_dir() {
                    env.push(("PATH".to_string(), prepend_path(&dir.to_string_lossy())));
                }
                env.extend(elixir_env());
            }
        }
        // The managed Swift toolchain's `sourcekit-lsp` finds its sibling
        // binaries through `usr/bin`, and its libraries through the toolchain's
        // own lib directory plus Koda's compatibility directory (which aliases
        // the sonames a non-native distribution does not provide).
        Tool::SwiftLs => {
            if let Some(dir) = swift_dir() {
                env.push((
                    "PATH".to_string(),
                    prepend_path(&dir.join("usr/bin").to_string_lossy()),
                ));
                let mut libs: Vec<String> = Vec::new();
                if let Some(compat) = swift_compat_dir() {
                    libs.push(compat.to_string_lossy().into_owned());
                }
                libs.push(
                    dir.join("usr/lib/swift/linux")
                        .to_string_lossy()
                        .into_owned(),
                );
                env.push((
                    "LD_LIBRARY_PATH".to_string(),
                    prepend_colon(&libs.join(":")),
                ));
            }
        }
        // `solargraph` is installed into a Koda-private gem home; its launcher
        // needs that home and its bin directory on the environment.
        Tool::RubyLs => {
            if let Some(gems) = gem_home() {
                let gems = gems.to_string_lossy().into_owned();
                env.push(("GEM_HOME".to_string(), gems.clone()));
                env.push(("GEM_PATH".to_string(), gems.clone()));
                env.push(("PATH".to_string(), prepend_path(&format!("{gems}/bin"))));
            }
        }
        // `phpactor`'s phar runs through `#!/usr/bin/env php`; put the managed
        // phpactor directory on `PATH` so a bare `phpactor` also resolves.
        Tool::Phpactor => {
            if let Some(dir) = phpactor_dir() {
                env.push(("PATH".to_string(), prepend_path(&dir.to_string_lossy())));
            }
        }
        // Go-installed tools are launched by their resolved path; a Koda-managed
        // Go only needs its bin directories when a child process invokes `go`.
        Tool::Gopls | Tool::Sqls | Tool::Shfmt => {
            if let Some(bin) = go_bin_dir() {
                env.push(("PATH".to_string(), prepend_path(&bin.to_string_lossy())));
            }
            if let Some(goroot) = go_dir().filter(|dir| dir.is_dir()) {
                env.push(("GOROOT".to_string(), goroot.to_string_lossy().into_owned()));
                env.push((
                    "PATH".to_string(),
                    prepend_path(&goroot.join("bin").to_string_lossy()),
                ));
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

/// Prepend a colon-separated list of directories to `PATH`.
fn prepend_colon(dirs: &str) -> String {
    match std::env::var_os("PATH") {
        Some(existing) => format!("{dirs}:{}", existing.to_string_lossy()),
        None => dirs.to_string(),
    }
}

/// The environment the managed Erlang/Elixir runtime needs: its `bin`
/// directories on `PATH` and a private Mix/Hex home.
///
/// Only the directories Koda actually installed are used, so a system Elixir is
/// left on the user's own configuration.
pub fn elixir_env() -> Vec<(String, String)> {
    let mut env = Vec::new();
    let mut bins: Vec<String> = Vec::new();
    if let Some(dir) = elixir_dir().filter(|dir| dir.is_dir()) {
        bins.push(dir.join("bin").to_string_lossy().into_owned());
    }
    if let Some(dir) = otp_dir().filter(|dir| dir.is_dir()) {
        bins.push(dir.join("bin").to_string_lossy().into_owned());
    }
    if !bins.is_empty() {
        env.push(("PATH".to_string(), prepend_colon(&bins.join(":"))));
    }
    if let Some(home) = mix_home().filter(|dir| dir.is_dir()) {
        let home = home.to_string_lossy().into_owned();
        env.push(("MIX_HOME".to_string(), home.clone()));
        env.push(("HEX_HOME".to_string(), home));
    }
    env
}

/// Whether this host uses musl rather than glibc.
///
/// musl systems (Alpine, Void musl) cannot run the glibc-targeted precompiled
/// runtimes, so Koda reports them as unsupported instead of downloading a build
/// that fails.
pub fn libc_is_musl() -> bool {
    let alpine = std::path::Path::new("/etc/alpine-release").exists();
    let musl_loader = ["/lib/ld-musl-x86_64.so.1", "/lib/ld-musl-aarch64.so.1"]
        .iter()
        .any(|loader| std::path::Path::new(loader).exists());
    musl_from_markers(alpine, musl_loader)
}

/// The musl decision from its two markers, so the logic is unit-testable.
fn musl_from_markers(alpine_release: bool, musl_loader: bool) -> bool {
    alpine_release || musl_loader
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
    let jdtls_url = format!(
        "https://download.eclipse.org/jdtls/milestones/{JDTLS_VERSION}/\
         jdt-language-server-{JDTLS_SNAPSHOT}.tar.gz"
    );
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
                release: ADOPTIUM_JDK25,
                dest: jdk_archive.clone(),
            },
            InstallStep::Extract {
                archive: jdk_archive,
                dest: jdk,
                strip: 1,
            },
            InstallStep::Download {
                url: jdtls_url,
                dest: jdtls_archive.clone(),
                sha256: JDTLS_SHA256.to_string(),
            },
            InstallStep::Extract {
                archive: jdtls_archive,
                dest,
                strip: 0,
            },
        ],
    )]
}

/// A self-contained `lua-language-server` release for this platform, or `None`
/// when the project publishes none.
fn lua_ls_asset() -> Option<String> {
    let (triple, extension) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => ("linux-x64", "tar.gz"),
        ("linux", "aarch64") => ("linux-arm64", "tar.gz"),
        ("macos", "x86_64") => ("darwin-x64", "tar.gz"),
        ("macos", "aarch64") => ("darwin-arm64", "tar.gz"),
        ("windows", "x86_64") => ("win32-x64", "zip"),
        _ => return None,
    };
    Some(format!(
        "lua-language-server-{LUA_LS_VERSION}-{triple}.{extension}"
    ))
}

/// Download and unpack the self-contained Lua language server.
///
/// The archive extracts its `bin/`, `main.lua` and `script/` at the top level,
/// so `tools/lua-language-server/bin/lua-language-server` is the launcher. It
/// needs no runtime, which is why Koda can provision it unconditionally.
fn lua_ls_attempts() -> Vec<InstallAttempt> {
    let Some(asset) = lua_ls_asset() else {
        return Vec::new();
    };
    let (Some(downloads), Some(dest)) = (downloads_dir(), lua_ls_dir()) else {
        return Vec::new();
    };
    let archive = downloads.join(&asset);
    vec![InstallAttempt::managed(
        "a self-contained lua-language-server",
        vec![
            InstallStep::GithubRelease {
                repo: "LuaLS/lua-language-server".to_string(),
                tag: LUA_LS_VERSION.to_string(),
                asset,
                dest: archive.clone(),
            },
            InstallStep::Extract {
                archive,
                dest,
                strip: 0,
            },
        ],
    )]
}

/// A dedicated JDK 21 and `kotlin-language-server`, both managed by Koda.
///
/// The server's bundled Kotlin compiler rejects the four-part version string of
/// the JDK 25 that `jdtls` uses, so Koda provisions its own JDK 21 in an
/// isolated directory instead of downgrading anything the user has.
fn kotlin_ls_attempts() -> Vec<InstallAttempt> {
    let (Some(downloads), Some(dest)) = (downloads_dir(), kotlin_ls_dir()) else {
        return Vec::new();
    };
    let Some(jdk) = kotlin_jdk_dir() else {
        return Vec::new();
    };
    let jdk_archive = downloads.join("temurin21.tar.gz");
    let ls_archive = downloads.join("kotlin-language-server.zip");
    let url = format!(
        "https://github.com/fwcd/kotlin-language-server/releases/download/{KOTLIN_LS_VERSION}/server.zip"
    );
    vec![InstallAttempt::managed(
        "a managed JDK 21 and kotlin-language-server",
        vec![
            InstallStep::AdoptiumJdk {
                release: ADOPTIUM_JDK21,
                dest: jdk_archive.clone(),
            },
            InstallStep::Extract {
                archive: jdk_archive,
                dest: jdk,
                strip: 1,
            },
            InstallStep::Download {
                url,
                dest: ls_archive.clone(),
                sha256: KOTLIN_LS_SHA256.to_string(),
            },
            // The distribution's archive root is `server/`.
            InstallStep::Extract {
                archive: ls_archive,
                dest,
                strip: 1,
            },
        ],
    )]
}

/// Install `Perl::LanguageServer` into an isolated `local::lib`.
fn perl_attempts() -> Vec<InstallAttempt> {
    perl_module_attempts(PERL_LS_SPEC)
}

/// Install `PLS` into an isolated `local::lib`.
///
/// PLS is preferred over `Perl::LanguageServer` because it has no `Coro`
/// dependency, so it builds and runs on current Perls.
fn pls_attempts() -> Vec<InstallAttempt> {
    perl_module_attempts(PLS_SPEC)
}

/// Bootstrap `cpanm` and install `package` into an isolated `local::lib`.
///
/// Koda bootstraps `cpanm` from a checksum-verified `App::cpanminus` archive —
/// so an interactive, unconfigured `cpan` is never run — and uses it with
/// `--local-lib`, which never touches the system Perl. A system `perl` is still
/// required to run `cpanm`; Koda does not build a Perl runtime.
fn perl_module_attempts(package: &str) -> Vec<InstallAttempt> {
    if locate("perl").is_none() {
        return Vec::new();
    }
    let (Some(downloads), Some(lib), Some(cpanm)) =
        (downloads_dir(), perl_local_lib_dir(), cpanm_dir())
    else {
        return Vec::new();
    };
    let archive = downloads.join(format!("App-cpanminus-{CPANM_VERSION}.tar.gz"));
    let url = format!(
        "https://cpan.metacpan.org/authors/id/M/MI/MIYAGAWA/App-cpanminus-{CPANM_VERSION}.tar.gz"
    );
    let cpanm_script = cpanm.join("bin/cpanm");
    vec![InstallAttempt::managed(
        "a checksum-verified cpanm and an isolated local::lib",
        vec![
            InstallStep::Download {
                url,
                dest: archive.clone(),
                sha256: CPANM_SHA256.to_string(),
            },
            InstallStep::Extract {
                archive,
                dest: cpanm,
                strip: 1,
            },
            InstallStep::Run(
                InstallCommand::with_args(
                    "perl",
                    vec![
                        cpanm_script.to_string_lossy().into_owned(),
                        "--local-lib".to_string(),
                        lib.to_string_lossy().into_owned(),
                        "--notest".to_string(),
                        package.to_string(),
                    ],
                )
                // Some Makefile.PLs prompt; take the default answer instead of
                // hanging a background install.
                .env("PERL_MM_USE_DEFAULT", "1"),
            ),
        ],
    )]
}

/// The Swift toolchain archive URL and its detached signature URL.
fn swift_toolchain_urls(tag: &str, suffix: &str) -> (String, String) {
    let asset = format!("swift-{SWIFT_VERSION}-RELEASE-{suffix}.tar.gz");
    let url = format!(
        "https://download.swift.org/swift-{SWIFT_VERSION}-release/{tag}/swift-{SWIFT_VERSION}-RELEASE/{asset}"
    );
    (url.clone(), format!("{url}.sig"))
}

/// A GPG-verified Swift toolchain from swift.org.
///
/// swift.org publishes one toolchain per distribution release and signs it with
/// a detached signature, so Koda verifies the signature against swift.org's
/// published keys and extracts the `usr/` tree. A distribution swift.org does
/// not build for gets the portable UBI10 build plus a managed compatibility
/// layer, so the same verified artifact runs without touching the system. When
/// even that cannot run (a missing compatibility library, musl, a non-Linux
/// host) the attempt is empty and Koda explains exactly why.
fn swift_attempts() -> Vec<InstallAttempt> {
    let Some(toolchain) = swift_toolchain() else {
        return Vec::new();
    };
    let (Some(downloads), Some(dest), Some(compat)) =
        (downloads_dir(), swift_dir(), swift_compat_dir())
    else {
        return Vec::new();
    };
    let (url, signature_url) = swift_toolchain_urls(toolchain.tag, toolchain.suffix);
    let archive = downloads.join(format!(
        "swift-{SWIFT_VERSION}-RELEASE-{}.tar.gz",
        toolchain.suffix
    ));
    let mut steps = vec![
        InstallStep::DownloadGpg {
            url,
            signature_url,
            keys_url: "https://www.swift.org/keys/all-keys.asc".to_string(),
            dest: archive.clone(),
        },
        InstallStep::Extract {
            archive,
            dest,
            strip: 1,
        },
    ];
    // A build for another distribution needs the compatibility layer; a native
    // build already links against this distribution's libraries.
    if !toolchain.native {
        steps.push(InstallStep::SwiftCompat { dest: compat });
    }
    vec![InstallAttempt::managed(
        "a GPG-verified Swift toolchain",
        steps,
    )]
}

/// A coordinated Erlang/OTP + Elixir + ElixirLS toolchain.
///
/// Erlang/OTP and Elixir come from `builds.hex.pm` (the Erlang Ecosystem
/// Foundation's build service, which also backs the official `setup-beam`
/// action) and are checksum-verified; the OTP tree gets its `Install -minimal`
/// pass so `erl`/`erlc` exist. ElixirLS is the official release, built once at
/// install time with the managed Mix so the first launch is fast. Everything is
/// isolated: a private `MIX_HOME`/`HEX_HOME`, and runtimes on a scoped `PATH`.
fn elixir_ls_attempts() -> Vec<InstallAttempt> {
    // Only offered where `bob` publishes Erlang/Elixir builds for this platform.
    if bob_platform().is_none() {
        return Vec::new();
    }
    let (Some(downloads), Some(otp), Some(elixir), Some(ls)) =
        (downloads_dir(), otp_dir(), elixir_dir(), elixir_ls_dir())
    else {
        return Vec::new();
    };
    let Some(mix) = mix_home() else {
        return Vec::new();
    };
    let otp_archive = downloads.join(format!("OTP-{OTP_VERSION}.tar.gz"));
    let elixir_archive = downloads.join(format!(
        "v{ELIXIR_VERSION}-otp-{}.zip",
        otp_major(OTP_VERSION)
    ));
    let ls_archive = downloads.join(format!("elixir-ls-v{ELIXIR_LS_VERSION}.zip"));
    let rebar_archive = downloads.join("rebar3");
    let installer = otp.join("Install");
    let quiet_install = ls.join("quiet_install.exs");
    vec![InstallAttempt::managed(
        "Erlang/OTP, Elixir and a built ElixirLS",
        vec![
            InstallStep::BobBuild {
                package: BobPackage::Erlang,
                dest: otp_archive.clone(),
            },
            InstallStep::Extract {
                archive: otp_archive,
                dest: otp.clone(),
                strip: 1,
            },
            // `bob` ships the Erlang source layout; `Install -minimal` creates
            // the `bin/erl` wrappers without compiling anything.
            InstallStep::Run(
                InstallCommand::with_args(
                    "sh",
                    vec![
                        installer.to_string_lossy().into_owned(),
                        "-minimal".to_string(),
                        otp.to_string_lossy().into_owned(),
                    ],
                )
                .cwd(otp.clone()),
            ),
            InstallStep::BobBuild {
                package: BobPackage::Elixir,
                dest: elixir_archive.clone(),
            },
            InstallStep::Extract {
                archive: elixir_archive,
                dest: elixir,
                strip: 0,
            },
            // Install Hex and rebar into the private Mix home so the build never
            // prompts and never writes to the user's `~/.mix`. Hex is pinned to
            // an exact version; rebar3 is a pinned, checksum-verified escript
            // registered from the local file rather than fetched by Mix.
            InstallStep::Run(
                InstallCommand::new("mix", &["local.hex", HEX_VERSION, "--force"])
                    .env("MIX_HOME", mix.to_string_lossy().into_owned()),
            ),
            InstallStep::Download {
                url: format!(
                    "https://github.com/erlang/rebar3/releases/download/{REBAR3_VERSION}/rebar3"
                ),
                dest: rebar_archive.clone(),
                sha256: REBAR3_SHA256.to_string(),
            },
            InstallStep::MakeExecutable {
                path: rebar_archive.clone(),
            },
            InstallStep::Run(
                InstallCommand::with_args(
                    "mix",
                    vec![
                        "local.rebar".into(),
                        "rebar3".into(),
                        rebar_archive.to_string_lossy().into_owned(),
                        "--force".into(),
                    ],
                )
                .env("MIX_HOME", mix.to_string_lossy().into_owned()),
            ),
            InstallStep::GithubRelease {
                repo: "elixir-lsp/elixir-ls".to_string(),
                tag: format!("v{ELIXIR_LS_VERSION}"),
                asset: format!("elixir-ls-v{ELIXIR_LS_VERSION}.zip"),
                dest: ls_archive.clone(),
            },
            InstallStep::Extract {
                archive: ls_archive,
                dest: ls.clone(),
                strip: 0,
            },
            // Build the release once, with the managed runtime and a private
            // Mix/Hex home, so launching the server never has to compile.
            InstallStep::Run(
                InstallCommand::new("elixir", &[quiet_install.to_string_lossy().as_ref()])
                    .cwd(ls.clone())
                    .env("MIX_HOME", mix.to_string_lossy().into_owned())
                    .env("HEX_HOME", mix.to_string_lossy().into_owned())
                    .env("MIX_ENV", "prod"),
            ),
        ],
    )]
}

/// A Koda-managed Dart SDK for the analysis server (`dart language-server`).
///
/// The SDK archive extracts to a `dart-sdk/` root, so it is stripped by one
/// level. Only Dart is installed; Flutter is not downloaded.
fn dart_sdk_attempts() -> Vec<InstallAttempt> {
    let (Some(downloads), Some(dest)) = (downloads_dir(), dart_sdk_dir()) else {
        return Vec::new();
    };
    let Some(asset) = dart_sdk_asset() else {
        return Vec::new();
    };
    let archive = downloads.join(asset);
    vec![InstallAttempt::managed(
        "a managed Dart SDK",
        vec![
            InstallStep::DartSdk {
                dest: archive.clone(),
            },
            InstallStep::Extract {
                archive,
                dest,
                strip: 1,
            },
        ],
    )]
}

/// OmniSharp plus the .NET SDK it runs on, both managed by Koda.
fn omnisharp_attempts() -> Vec<InstallAttempt> {
    let Some(asset) = omnisharp_asset() else {
        return Vec::new();
    };
    let (Some(dotnet), Some(downloads), Some(dest)) =
        (dotnet_dir(), downloads_dir(), omnisharp_dir())
    else {
        return Vec::new();
    };
    let dotnet_archive = downloads.join("dotnet-sdk.tar.gz");
    let archive = downloads.join("omnisharp.tar.gz");
    vec![InstallAttempt::managed(
        "the .NET SDK and OmniSharp",
        vec![
            InstallStep::DotnetSdk {
                dest: dotnet_archive.clone(),
            },
            InstallStep::Extract {
                archive: dotnet_archive,
                dest: dotnet,
                strip: 0,
            },
            InstallStep::GithubRelease {
                repo: "OmniSharp/omnisharp-roslyn".to_string(),
                tag: OMNISHARP_VERSION.to_string(),
                asset: asset.to_string(),
                dest: archive.clone(),
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
                    | LanguageId::JavaScript
                    | LanguageId::C
                    | LanguageId::Java
                    | LanguageId::CSharp
                    | LanguageId::Php
                    | LanguageId::Lua
                    | LanguageId::Kotlin
                    | LanguageId::Sql
                    | LanguageId::Ruby
                    | LanguageId::Assembly
                    | LanguageId::Perl
                    | LanguageId::Dart
                    | LanguageId::Elixir
                    | LanguageId::Swift
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
            assert!(steps.iter().any(|step| matches!(
                step,
                InstallStep::AdoptiumJdk {
                    release: ADOPTIUM_JDK25,
                    ..
                }
            )));
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
                    .any(|step| matches!(step, InstallStep::DotnetSdk { .. }))
            );
            assert!(
                steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::GithubRelease { .. }))
            );
            assert!(
                steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::Extract { .. }))
            );
        }
    }

    /// Package arguments of an `npm install` command, skipping flags and the
    /// values of the flags that take one.
    fn npm_packages(args: &[String]) -> Vec<&str> {
        let mut packages = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            if arg == "--prefix" || arg == "--cache" {
                i += 2;
                continue;
            }
            if arg == "install" || arg.starts_with('-') {
                i += 1;
                continue;
            }
            packages.push(arg);
            i += 1;
        }
        packages
    }

    /// Assert a `Run` step names an exact version for every package-manager
    /// install. The remaining delegation Koda cannot pin independently
    /// (`rustup component add`) is intentionally not matched; Hex is pinned by
    /// version, and rebar3 is a verified `Download` step.
    fn assert_run_is_pinned(tool: Tool, command: &InstallCommand) {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        for arg in &args {
            assert!(
                !arg.contains("@latest") && *arg != "latest",
                "{tool:?} uses a floating install input: {command:?}"
            );
        }
        match command.program.as_str() {
            "go" => {
                if args.first() == Some(&"install") {
                    for spec in &args[1..] {
                        assert!(
                            spec.contains('@'),
                            "{tool:?} must pin its go install input: {command:?}"
                        );
                    }
                }
            }
            "npm" => {
                for package in npm_packages(&command.args) {
                    assert!(
                        package.contains('@'),
                        "{tool:?} must pin its npm input: {command:?}"
                    );
                }
            }
            "pipx" | "uv" | "pip" | "pip3" => {
                assert!(
                    args.iter().any(|arg| arg.contains("==")),
                    "{tool:?} must pin its pip input: {command:?}"
                );
            }
            "python" | "python3" => {
                if args.contains(&"pip") {
                    assert!(
                        args.iter().any(|arg| arg.contains("==")),
                        "{tool:?} must pin its pip input: {command:?}"
                    );
                }
            }
            "gem" => assert!(
                args.contains(&"--version"),
                "{tool:?} must pin its gem input: {command:?}"
            ),
            "cargo" => {
                if args.first() == Some(&"install") {
                    assert!(
                        args.contains(&"--version"),
                        "{tool:?} must pin its cargo input: {command:?}"
                    );
                }
            }
            "perl" if args.iter().any(|arg| arg.ends_with("cpanm")) => {
                let spec = args.last().copied().unwrap_or("");
                assert!(
                    spec.contains('@'),
                    "{tool:?} must pin its cpan input: {command:?}"
                );
            }
            _ => {}
        }
    }

    /// Every managed install input is either exactly versioned or verified.
    ///
    /// This iterates the real installer plans, so adding a new managed installer
    /// without a pinned version or a digest fails here (and a plain `Download`
    /// without a digest cannot even be constructed).
    #[test]
    fn every_managed_install_is_pinned_and_verified() {
        for &tool in Tool::ALL {
            for attempt in tool.install_attempts() {
                assert!(!attempt.via.is_empty(), "{tool:?} has an unnamed strategy");
                for step in &attempt.steps {
                    match step {
                        InstallStep::Download { url, sha256, .. } => {
                            assert!(
                                !url.contains("latest"),
                                "{tool:?} downloads a floating URL: {url}"
                            );
                            assert!(
                                is_sha256_hex(sha256),
                                "{tool:?} has an invalid SHA-256 for {url}"
                            );
                        }
                        InstallStep::GithubRelease {
                            repo, tag, asset, ..
                        } => {
                            assert!(!repo.is_empty() && !asset.is_empty());
                            assert!(
                                !tag.is_empty() && tag != "latest",
                                "{tool:?} uses a floating GitHub tag: {tag}"
                            );
                        }
                        InstallStep::DownloadGpg {
                            url,
                            signature_url,
                            keys_url,
                            ..
                        } => {
                            assert!(!url.contains("latest"));
                            assert!(!signature_url.is_empty() && !keys_url.is_empty());
                        }
                        InstallStep::Run(command) => assert_run_is_pinned(tool, command),
                        // These verify internally (a checksum, signature or
                        // digest read from upstream metadata) or touch no
                        // download at all.
                        InstallStep::BobBuild { .. }
                        | InstallStep::AdoptiumJdk { .. }
                        | InstallStep::DotnetSdk { .. }
                        | InstallStep::NodeRuntime { .. }
                        | InstallStep::DartSdk { .. }
                        | InstallStep::RustupInit { .. }
                        | InstallStep::GoToolchain { .. }
                        | InstallStep::SwiftCompat { .. }
                        | InstallStep::MakeExecutable { .. }
                        | InstallStep::Extract { .. } => {}
                    }
                }
            }
        }
    }

    #[test]
    fn lua_tool_is_self_contained_and_managed() {
        assert_eq!(
            Tool::for_language(LanguageId::Lua, ToolPurpose::LanguageServer),
            Some(Tool::LuaLs)
        );
        assert!(
            Tool::LuaLs.server_args().is_empty(),
            "lua-language-server speaks stdio with no arguments"
        );
        if lua_ls_asset().is_none() || tools_dir().is_none() {
            return; // No published archive for this platform.
        }
        let attempts = Tool::LuaLs.install_attempts();
        let steps = &attempts.first().expect("a Lua language server plan").steps;
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::GithubRelease { .. }))
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::Extract { strip: 0, .. }))
        );
    }

    #[test]
    fn new_language_tools_use_trusted_managers() {
        // Go tooling (`sqls`) is provisioned by Koda's managed Go plan.
        assert_eq!(Tool::Sqls.install_command(), None);
        if go_platform().is_some() && tools_dir().is_some() {
            assert!(
                Tool::Sqls.install_attempts().iter().any(|attempt| attempt
                    .steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::GoToolchain { .. }))),
                "sqls must be able to provision Go"
            );
        }
        // `solargraph` installs into an isolated gem home.
        assert_eq!(Tool::RubyLs.install_command(), None);
        if locate("gem").is_some() && tools_dir().is_some() {
            assert!(
                Tool::RubyLs.install_attempts().iter().any(|attempt| attempt
                    .steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::Run(command)
                        if command.args.iter().any(|arg| arg == "--install-dir")))),
                "solargraph must use an isolated gem home"
            );
        }
        // `asm-lsp` uses the prebuilt release when one exists.
        assert_eq!(Tool::AsmLsp.install_command(), None);
        if asm_lsp_asset().is_some() && tools_dir().is_some() {
            assert!(
                Tool::AsmLsp
                    .install_attempts()
                    .iter()
                    .any(|attempt| attempt.steps.iter().any(
                        |step| matches!(step, InstallStep::GithubRelease { repo, .. }
                        if repo == "bergercookie/asm-lsp")
                    )),
                "asm-lsp should prefer the prebuilt release"
            );
        }
        // The multi-component toolchains are managed, so they advertise no
        // single package-manager command; whether they can be installed depends
        // on the platform and on the base tools being present.
        for tool in [
            Tool::ElixirLs,
            Tool::SwiftLs,
            Tool::PerlLs,
            Tool::DartAnalyzer,
            Tool::Clangd,
            Tool::Phpactor,
        ] {
            assert!(
                tool.install_command().is_none(),
                "{tool:?} must not advertise a package-manager command"
            );
        }
        let archives = locate("curl").is_some() && locate("tar").is_some();
        // Perl bootstraps cpanm; it needs a system perl plus the archive tools.
        assert_eq!(
            can_install(Tool::PerlLs),
            locate("perl").is_some() && archives
        );
        // Elixir is managed only where `bob` publishes builds. Its OTP runtime
        // is a tarball and the Elixir/ElixirLS archives are zips, so a zip
        // extractor is required too (any of unzip, bsdtar or Python).
        assert_eq!(
            can_install(Tool::ElixirLs),
            bob_platform().is_some() && archives && zip_extractor().is_some()
        );
        // The Dart SDK is managed as a zip, so Koda can install it where
        // published and a zip extractor is available.
        assert_eq!(
            can_install(Tool::DartAnalyzer),
            dart_sdk_asset().is_some() && locate("curl").is_some() && zip_extractor().is_some()
        );
        // Swift is managed where swift.org publishes a signed toolchain (native
        // or the portable UBI10 build with its compatibility layer), and only
        // when the signature can be verified and every library is available.
        let swift_expected = match swift_toolchain() {
            None => false,
            Some(toolchain) => {
                locate("curl").is_some()
                    && locate("gpg").is_some()
                    && locate("tar").is_some()
                    && (toolchain.native || swift_missing_libs().is_empty())
            }
        };
        assert_eq!(can_install(Tool::SwiftLs), swift_expected);
        assert_eq!(
            Tool::for_language(LanguageId::Dart, ToolPurpose::LanguageServer),
            Some(Tool::DartAnalyzer)
        );
        assert_eq!(
            Tool::for_language(LanguageId::Elixir, ToolPurpose::LanguageServer),
            Some(Tool::ElixirLs)
        );
        assert_eq!(
            Tool::for_language(LanguageId::Swift, ToolPurpose::LanguageServer),
            Some(Tool::SwiftLs)
        );
    }

    #[test]
    fn perl_runs_as_a_perl_one_liner_with_an_isolated_lib() {
        assert_eq!(Tool::PerlLs.program(), "perl");
        assert!(
            Tool::PerlLs
                .server_args()
                .contains(&"-MPerl::LanguageServer"),
            "Perl is launched through the module: {:?}",
            Tool::PerlLs.server_args()
        );
        if let Some(lib) = perl_local_lib_dir() {
            let env = launch_env(Tool::PerlLs);
            let perl5lib = env
                .iter()
                .find(|(key, _)| key == "PERL5LIB")
                .map(|(_, value)| value.clone());
            assert_eq!(
                perl5lib,
                Some(format!("{}/lib/perl5", lib.display())),
                "the managed local::lib must be on PERL5LIB"
            );
        }
    }

    #[test]
    fn elixir_has_alternate_launcher_names() {
        assert!(Tool::ElixirLs.candidates().contains(&"language_server.sh"));
        assert!(Tool::RustAnalyzer.candidates().is_empty());
        assert_eq!(Tool::ElixirLs.program(), "elixir-ls");
    }

    #[test]
    fn dart_sdk_plan_is_checksum_verified() {
        let attempts = Tool::DartAnalyzer.install_attempts();
        if dart_sdk_asset().is_none() || tools_dir().is_none() {
            assert!(attempts.is_empty());
            return;
        }
        let steps = &attempts.first().expect("a Dart plan").steps;
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::DartSdk { .. })),
            "Dart must download the SDK: {steps:?}"
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::Extract { strip: 1, .. })),
            "the SDK archive root must be stripped: {steps:?}"
        );
        // The sibling `.sha256sum` uses `<hash> *<name>`.
        let body = "ea864bc64df30a6b8bdf30b2e32550f7717d9a890de8f40293aeabb924fe232b *dartsdk-linux-x64-release.zip\n";
        assert_eq!(
            checksum_for(body, "dartsdk-linux-x64-release.zip").as_deref(),
            Some("ea864bc64df30a6b8bdf30b2e32550f7717d9a890de8f40293aeabb924fe232b")
        );
        assert_eq!(checksum_for(body, "missing.zip"), None);
    }

    #[test]
    fn go_plan_provisions_the_official_toolchain() {
        let attempts = Tool::Gopls.install_attempts();
        assert!(!attempts.is_empty(), "gopls must have an install plan");
        if go_platform().is_none() || tools_dir().is_none() {
            return;
        }
        let managed = attempts
            .iter()
            .find(|attempt| {
                attempt
                    .steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::GoToolchain { .. }))
            })
            .expect("a managed Go attempt");
        assert!(
            managed
                .steps
                .iter()
                .any(|step| matches!(step, InstallStep::Extract { strip: 1, .. })),
            "the Go archive root must be stripped: {:?}",
            managed.steps
        );
        let run = managed
            .steps
            .iter()
            .find_map(|step| match step {
                InstallStep::Run(command) if command.program == "go" => Some(command),
                _ => None,
            })
            .expect("a go install run");
        assert!(
            run.args.iter().any(|arg| arg == GOPLS_SPEC),
            "the run must install pinned gopls: {:?}",
            run.args
        );
        // Tool installations stay inside Koda's private GOPATH/GOBIN.
        if let Some(gobin) = go_bin_dir() {
            assert!(
                run.env
                    .iter()
                    .any(|(key, value)| key == "GOBIN"
                        && value == &gobin.to_string_lossy().into_owned()),
                "GOBIN must point at Koda's private bin directory: {:?}",
                run.env
            );
        }
    }

    #[test]
    fn go_install_pins_goroot_only_when_managed_go_exists() {
        // Regression: a `GOROOT` pointing at a not-yet-downloaded managed Go
        // makes a good system `go` refuse to run, forcing a needless download.
        let command = go_install_command(&["golang.org/x/tools/gopls"]);
        let managed = go_dir().is_some_and(|dir| dir.join("bin/go").is_file());
        let has_goroot = command.env.iter().any(|(key, _)| key == "GOROOT");
        assert_eq!(has_goroot, managed);
        // Tool installs always stay inside Koda's private GOBIN.
        assert!(command.env.iter().any(|(key, _)| key == "GOBIN"));
    }

    #[test]
    fn phpactor_plan_is_a_verified_phar() {
        let attempts = Tool::Phpactor.install_attempts();
        if locate("php").is_none() || tools_dir().is_none() {
            assert!(attempts.is_empty());
            return;
        }
        let steps = &attempts.first().expect("a phpactor plan").steps;
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::GithubRelease { repo, asset, .. }
                    if repo == "phpactor/phpactor" && asset == "phpactor.phar"
            )),
            "phpactor must come from the verified phar release: {steps:?}"
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::MakeExecutable { .. })),
            "the phar must be made executable: {steps:?}"
        );
    }

    #[test]
    fn swift_platform_maps_supported_distributions() {
        assert_eq!(
            swift_platform("ubuntu", "", "24.04", "x86_64"),
            Some(("ubuntu2404", "ubuntu24.04"))
        );
        assert_eq!(
            swift_platform("debian", "", "12", "aarch64"),
            Some(("debian12", "debian12"))
        );
        // Derivatives are mapped through ID_LIKE.
        assert_eq!(
            swift_platform("pop", "ubuntu", "22.04", "x86_64"),
            Some(("ubuntu2204", "ubuntu22.04"))
        );
        // Unsupported distribution or architecture is reported, never guessed.
        assert_eq!(swift_platform("endeavouros", "arch", "", "x86_64"), None);
        assert_eq!(swift_platform("ubuntu", "", "24.04", "riscv64"), None);
    }

    #[test]
    fn file_sha256_matches_a_known_digest() {
        let path = std::env::temp_dir().join(format!("koda-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let hash = file_sha256(&path).expect("a SHA-256 tool");
        assert_eq!(
            hash.to_lowercase(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sha256_hex_is_validated() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex("abc"));
        assert!(!is_sha256_hex(&"z".repeat(64)));
    }

    #[test]
    fn musl_is_detected_from_either_marker() {
        assert!(musl_from_markers(true, false));
        assert!(musl_from_markers(false, true));
        assert!(!musl_from_markers(false, false));
    }

    #[test]
    fn bob_platform_maps_supported_distributions() {
        assert_eq!(
            bob_platform_for("ubuntu", "", "24.04"),
            Some("ubuntu-24.04")
        );
        assert_eq!(
            bob_platform_for("linuxmint", "ubuntu debian", "22.04"),
            Some("ubuntu-22.04")
        );
        assert_eq!(bob_platform_for("endeavouros", "arch", ""), None);
    }

    #[test]
    fn github_asset_digest_is_parsed() {
        let hash = "a".repeat(64);
        let body = format!(
            r#"{{"assets":[{{"name":"a.zip","digest":"sha256:{hash}"}},{{"name":"b.zip","digest":null}}]}}"#
        );
        assert_eq!(
            github_asset_sha256(&body, "a.zip").as_deref(),
            Some(&hash[..])
        );
        assert_eq!(github_asset_sha256(&body, "b.zip"), None);
        assert_eq!(github_asset_sha256(&body, "missing.zip"), None);
        assert_eq!(github_asset_sha256("not json", "a.zip"), None);
    }

    #[test]
    fn builds_txt_checksum_reads_the_hex_column() {
        let hash = "b".repeat(64);
        let body = format!(
            "OTP-27.3.3 10e20b1dbe39b056fab430e50b08cb4f3696ae87 2025-04-16T14:41:39Z d45ab837970f6c40596285441432e968c2932544eeb0ae0cf792ea0b30a90923\nOTP-27.3.4 c388a2d1b3f9918652276d4798692dd4d8ef97fc 2025-05-09T18:03:06Z {hash}\nOTP-24.3 0863bd30aabd035c83158c78046c5ffda16127e1 2024-04-26T03:41:09Z\n"
        );
        assert_eq!(
            builds_txt_checksum(&body, "OTP-27.3.4").as_deref(),
            Some(&hash[..])
        );
        // An entry without a checksum column is not trusted.
        assert_eq!(builds_txt_checksum(&body, "OTP-24.3"), None);
        assert_eq!(builds_txt_checksum(&body, "OTP-99"), None);
    }

    #[test]
    fn install_errors_prefer_the_concrete_failure() {
        let stderr = b"Building Coro-6.57 ... ! Installing Coro failed. See build.log\n! Installing the dependencies failed: Module 'Coro' is not installed\n! Bailing out\nFAIL\n";
        let message = last_stderr_line(stderr);
        assert!(message.contains("Coro"), "unexpected message: {message}");
        assert_eq!(last_stderr_line(b""), "installation failed");
    }

    #[test]
    fn swift_toolchain_urls_match_swift_org() {
        let (url, signature) = swift_toolchain_urls("ubuntu2404", "ubuntu24.04");
        assert_eq!(
            url,
            "https://download.swift.org/swift-6.4.0-release/ubuntu2404/swift-6.4.0-RELEASE/swift-6.4.0-RELEASE-ubuntu24.04.tar.gz"
        );
        assert_eq!(signature, format!("{url}.sig"));
    }

    #[test]
    fn every_tool_has_an_actionable_setup_reason() {
        for tool in Tool::ALL {
            assert!(
                !tool.setup_reason().is_empty(),
                "{tool:?} must explain how to set it up"
            );
        }
    }

    #[test]
    fn human_bytes_is_readable() {
        assert_eq!(human_bytes(240_000_000), "240 MB");
        assert_eq!(human_bytes(1_150_000_000), "1.1 GB");
    }

    #[test]
    fn otp_major_is_the_first_component() {
        assert_eq!(otp_major("27.3.4"), "27");
        assert_eq!(otp_major("29.1"), "29");
    }

    #[test]
    fn elixir_pins_hex_and_rebar() {
        if bob_platform().is_none() {
            return;
        }
        let attempts = Tool::ElixirLs.install_attempts();
        let Some(attempt) = attempts.first() else {
            return;
        };
        let mut hex_pinned = false;
        let mut rebar_downloaded = false;
        let mut rebar_registered = false;
        for step in &attempt.steps {
            match step {
                InstallStep::Run(command) if command.program == "mix" => {
                    if command.args.iter().any(|arg| arg == "local.hex") {
                        assert!(
                            command.args.iter().any(|arg| arg == HEX_VERSION),
                            "Hex must be pinned: {command:?}"
                        );
                        hex_pinned = true;
                    }
                    if command.args.iter().any(|arg| arg == "local.rebar") {
                        // rebar3 is registered from the verified local file, not
                        // fetched from the network by `mix`.
                        assert!(
                            command.args.iter().any(|arg| arg == "rebar3"),
                            "rebar must be registered from a local file: {command:?}"
                        );
                        rebar_registered = true;
                    }
                }
                InstallStep::Download { url, sha256, .. }
                    if url.contains("rebar3") && sha256.as_str() == REBAR3_SHA256 =>
                {
                    rebar_downloaded = true;
                }
                _ => {}
            }
        }
        assert!(hex_pinned, "the Elixir plan must pin Hex");
        assert!(
            rebar_downloaded,
            "rebar3 must be downloaded and checksum-verified"
        );
        assert!(
            rebar_registered,
            "the verified rebar3 must be registered with Mix"
        );
    }

    #[test]
    fn perl_prefers_pls_and_installs_it_with_cpanm() {
        let position = |tool: Tool| Tool::ALL.iter().position(|t| *t == tool).unwrap();
        assert!(
            position(Tool::Pls) < position(Tool::PerlLs),
            "PLS must be the preferred Perl server"
        );
        assert_eq!(Tool::Pls.program(), "pls");
        assert!(Tool::Pls.server_args().is_empty());
        assert!(Tool::Pls.probe_as_server());

        let attempts = Tool::Pls.install_attempts();
        if locate("perl").is_none() || tools_dir().is_none() {
            assert!(attempts.is_empty());
            return;
        }
        let steps = &attempts.first().expect("a PLS plan").steps;
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::Run(command)
                if command.program == "perl" && command.args.iter().any(|arg| arg == PLS_SPEC))),
            "the PLS plan must install PLS: {steps:?}"
        );
        // Both Perl servers share the isolated local::lib.
        if let Some(lib) = perl_local_lib_dir() {
            let env = launch_env(Tool::Pls);
            assert_eq!(
                env.iter()
                    .find(|(key, _)| key == "PERL5LIB")
                    .map(|(_, value)| value.clone()),
                Some(format!("{}/lib/perl5", lib.display()))
            );
        }
    }

    #[test]
    fn a_later_server_candidate_is_used_when_the_first_is_missing() {
        // Only `Perl::LanguageServer` is installed; PLS is not.
        let registry = ToolRegistry {
            statuses: vec![
                ToolStatus {
                    tool: Tool::Pls,
                    available: false,
                    version: None,
                    path: None,
                    error: None,
                },
                ToolStatus {
                    tool: Tool::PerlLs,
                    available: true,
                    version: None,
                    path: None,
                    error: None,
                },
            ],
        };
        assert_eq!(
            Tool::available_server(LanguageId::Perl, &registry),
            Some(Tool::PerlLs),
            "Koda must fall back to another installed Perl server"
        );
    }

    #[test]
    fn elixir_formatting_uses_mix() {
        use crate::language::provider::{Capability, LanguageProvider};
        let provider = crate::language::elixir::ElixirProvider;
        assert!(
            provider.capabilities().contains(&Capability::Formatting),
            "Elixir exposes Formatting"
        );
        assert_eq!(provider.formatter(), Some("mix"));
    }

    #[test]
    fn swift_toolchain_resolves_or_is_explained() {
        match swift_toolchain() {
            Some(toolchain) => {
                assert!(!toolchain.tag.is_empty() && !toolchain.suffix.is_empty());
                assert_eq!(toolchain.native, swift_native_asset().is_some());
            }
            None => assert!(
                libc_is_musl()
                    || std::env::consts::OS != "linux"
                    || !matches!(std::env::consts::ARCH, "x86_64" | "aarch64")
            ),
        }
    }

    #[test]
    fn swift_compat_needs_system_libraries() {
        let dir = std::env::temp_dir().join(format!("koda-swift-compat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        match swift_compat(&dir) {
            Ok(()) => {
                // Every required library was present and linked.
                assert!(dir.join("libxml2.so.2").exists());
            }
            Err(message) => {
                // The first library the compatibility layer cannot resolve is
                // named; a minimal system may lack any of them.
                assert!(
                    ["libxml2", "libncurses", "libpanel", "libform"]
                        .iter()
                        .any(|library| message.contains(library)),
                    "unexpected error: {message}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swift_plan_is_gpg_verified() {
        let attempts = Tool::SwiftLs.install_attempts();
        let Some(toolchain) = swift_toolchain() else {
            assert!(attempts.is_empty());
            return;
        };
        if tools_dir().is_none() {
            assert!(attempts.is_empty());
            return;
        }
        let steps = &attempts.first().expect("a Swift plan").steps;
        let (url, signature_url, keys_url) = steps
            .iter()
            .find_map(|step| match step {
                InstallStep::DownloadGpg {
                    url,
                    signature_url,
                    keys_url,
                    ..
                } => Some((url.clone(), signature_url.clone(), keys_url.clone())),
                _ => None,
            })
            .expect("Swift must verify a GPG signature");
        assert!(url.ends_with(".tar.gz"), "unexpected URL: {url}");
        assert_eq!(signature_url, format!("{url}.sig"));
        assert!(
            keys_url.contains("swift.org"),
            "unexpected key URL: {keys_url}"
        );
        // A non-native distribution needs the compatibility layer.
        assert_eq!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::SwiftCompat { .. })),
            !toolchain.native
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::Extract { strip: 1, .. })),
            "the toolchain archive root must be stripped: {steps:?}"
        );
    }

    #[test]
    fn elixir_plan_is_a_coordinated_toolchain() {
        let attempts = Tool::ElixirLs.install_attempts();
        if bob_platform().is_none() || tools_dir().is_none() {
            assert!(attempts.is_empty());
            return;
        }
        let steps = &attempts.first().expect("an Elixir plan").steps;
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::BobBuild {
                    package: BobPackage::Erlang,
                    ..
                }
            )),
            "an Erlang/OTP build is required: {steps:?}"
        );
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::BobBuild {
                    package: BobPackage::Elixir,
                    ..
                }
            )),
            "an Elixir build is required: {steps:?}"
        );
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::GithubRelease { asset, .. } if asset.contains("elixir-ls")
            )),
            "the official ElixirLS release is required: {steps:?}"
        );
        // The server is built once with the managed Elixir and a private Mix.
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::Run(command)
                    if command.program == "elixir"
                        && command.env.iter().any(|(key, _)| key == "MIX_HOME")
            )),
            "ElixirLS must be built with a private MIX_HOME: {steps:?}"
        );
    }

    #[test]
    fn kotlin_uses_a_dedicated_jdk_21() {
        // The Kotlin plan must fetch JDK 21, while jdtls keeps JDK 25.
        let attempts = Tool::KotlinLs.install_attempts();
        let steps = &attempts.first().expect("a Kotlin plan").steps;
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::AdoptiumJdk {
                    release: ADOPTIUM_JDK21,
                    ..
                }
            )),
            "Kotlin must install a JDK 21: {steps:?}"
        );
        let jdtls = Tool::Jdtls.install_attempts();
        assert!(
            jdtls
                .first()
                .expect("a jdtls plan")
                .steps
                .iter()
                .any(|step| matches!(
                    step,
                    InstallStep::AdoptiumJdk {
                        release: ADOPTIUM_JDK25,
                        ..
                    }
                )),
            "jdtls must keep its JDK 25"
        );
        // They live in separate directories and the launcher points Kotlin at
        // its own JAVA_HOME.
        assert!(tools_dir().is_some());
        assert_ne!(kotlin_jdk_dir(), jdtls_dir());
        let env = launch_env(Tool::KotlinLs);
        let java_home = env
            .iter()
            .find(|(key, _)| key == "JAVA_HOME")
            .map(|(_, value)| value.clone())
            .expect("Kotlin launch sets JAVA_HOME");
        assert_eq!(Some(PathBuf::from(java_home)), kotlin_jdk_dir());
    }

    #[test]
    fn formatters_map_to_languages_and_install_safely() {
        for (language, tool) in [
            (LanguageId::JavaScript, Tool::Prettier),
            (LanguageId::TypeScript, Tool::Prettier),
            (LanguageId::Html, Tool::Prettier),
            (LanguageId::Css, Tool::Prettier),
            (LanguageId::Json, Tool::Prettier),
            (LanguageId::Yaml, Tool::Prettier),
            (LanguageId::Markdown, Tool::Prettier),
            (LanguageId::C, Tool::ClangFormat),
            (LanguageId::Cpp, Tool::ClangFormat),
            (LanguageId::Shell, Tool::Shfmt),
            (LanguageId::Perl, Tool::PerlTidy),
        ] {
            assert_eq!(
                Tool::for_language(language, ToolPurpose::Formatter),
                Some(tool),
                "formatter for {language:?}"
            );
        }
        assert_eq!(
            Tool::Prettier.install_command(),
            Some(("npm", &["install", "-g", PRETTIER_SPEC][..]))
        );
        // `shfmt` is installed by Koda's managed Go plan, not a package-manager
        // command.
        assert_eq!(Tool::Shfmt.install_command(), None);
    }

    #[test]
    fn make_executable_sets_the_exec_bit() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = std::env::temp_dir().join(format!("koda-exec-{}", std::process::id()));
            let _ = std::fs::remove_file(&path);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
            make_executable(&path).expect("chmod");
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_ne!(mode & 0o100, 0, "the owner exec bit must be set");
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn unknown_program_is_not_located() {
        assert!(locate("koda-definitely-not-a-real-tool").is_none());
    }

    #[test]
    fn extract_rejects_parent_paths() {
        if locate("tar").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("koda-traverse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner/file.txt"), "x").unwrap();
        let built = std::process::Command::new("tar")
            .current_dir(dir.join("inner"))
            .args([
                "-cf",
                "../evil.tar",
                "--transform",
                "s,^,../escaped/,",
                "file.txt",
            ])
            .status();
        if !built.map(|status| status.success()).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let error = extract(&dir.join("evil.tar"), &dir.join("out"), 1).unwrap_err();
        assert!(
            error.contains("parent path"),
            "expected a traversal refusal, got: {error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_accepts_a_normal_archive() {
        if locate("tar").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("koda-extract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg/bin")).unwrap();
        std::fs::write(dir.join("pkg/bin/tool"), "#!/bin/sh\n").unwrap();
        let built = std::process::Command::new("tar")
            .current_dir(&dir)
            .args(["-czf", "pkg.tar.gz", "pkg"])
            .status();
        if !built.map(|status| status.success()).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        extract(&dir.join("pkg.tar.gz"), &dir.join("out"), 1).expect("a safe archive extracts");
        assert!(dir.join("out/bin/tool").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_space_is_checked_when_measurable() {
        let dir = std::env::temp_dir();
        if free_disk_bytes(&dir).is_none() {
            return; // `df` unavailable; the check degrades to a no-op.
        }
        ensure_disk_space(&dir, 0).expect("zero bytes always fits");
        let error = ensure_disk_space(&dir, u64::MAX).expect_err("an impossible request must fail");
        assert!(error.contains("free disk space"), "{error}");
    }

    #[test]
    fn managed_paths_and_sizes_are_recognised() {
        let Some(tools) = tools_dir() else {
            return;
        };
        assert!(is_managed_path(&tools.join("clangd/bin/clangd")));
        assert!(!is_managed_path(Path::new("/usr/bin/clangd")));
        assert_eq!(
            managed_component(&tools.join("clangd/bin/clangd")),
            Some(tools.join("clangd"))
        );
    }

    #[test]
    fn only_managed_installs_are_updateable_or_removable() {
        let Some(tools) = tools_dir() else {
            return;
        };
        let managed = ToolStatus {
            tool: Tool::Clangd,
            available: true,
            version: Some("clangd 23".to_string()),
            path: Some(tools.join("clangd/bin/clangd")),
            error: None,
        };
        assert!(managed.is_managed());
        assert_eq!(managed.managed_dir(), Some(tools.join("clangd")));
        assert_eq!(
            managed.managed_size(),
            directory_size(&tools.join("clangd"))
        );

        let external = ToolStatus {
            tool: Tool::Clangd,
            available: true,
            version: None,
            path: Some(PathBuf::from("/usr/bin/clangd")),
            error: None,
        };
        assert!(
            !external.is_managed(),
            "a system install must never be removed"
        );
        assert_eq!(external.managed_dir(), None);
    }

    #[test]
    fn directory_size_sums_files_recursively() {
        let dir = std::env::temp_dir().join(format!("koda-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("x"), vec![0u8; 10]).unwrap();
        std::fs::write(dir.join("a/b/y"), vec![0u8; 32]).unwrap();
        assert_eq!(directory_size(&dir), Some(42));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn proxy_environment_is_selected_for_curl() {
        // No proxy configured: nothing is forwarded.
        assert_eq!(proxy_from_env(|_| None), None);
        // A configured proxy is selected (here from the lowercase variable).
        let env = |key: &str| (key == "https_proxy").then(|| "http://proxy:3128".to_string());
        assert_eq!(proxy_from_env(env).as_deref(), Some("http://proxy:3128"));
        // HTTPS_PROXY wins over HTTP_PROXY.
        let env = |key: &str| match key {
            "HTTPS_PROXY" => Some("http://secure:8080".to_string()),
            "HTTP_PROXY" => Some("http://plain:3128".to_string()),
            _ => None,
        };
        assert_eq!(proxy_from_env(env).as_deref(), Some("http://secure:8080"));
        // An empty value is not a proxy.
        let env = |key: &str| (key == "HTTPS_PROXY").then(String::new);
        assert_eq!(proxy_from_env(env), None);
    }

    fn cache_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koda-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cache_hit_avoids_the_network() {
        let dir = cache_test_dir("cache-hit");
        let cache = dir.join("cache");
        let payload = dir.join("payload.tar.gz");
        std::fs::write(&payload, b"payload contents").unwrap();
        let Ok(sha) = file_sha256(&payload) else {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        cache_store(&cache, &sha, &payload, HashKind::Sha256).unwrap();

        // A URL that cannot be fetched: only the verified cache can satisfy this.
        let dest = dir.join("download.tar.gz");
        download_verified(
            "file:///definitely/missing",
            &dest,
            &sha,
            HashKind::Sha256,
            Some(&cache),
        )
        .expect("a verified cache entry is used without the network");
        assert_eq!(std::fs::read(&dest).unwrap(), b"payload contents");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_miss_downloads_then_caches() {
        let dir = cache_test_dir("cache-miss");
        let cache = dir.join("cache");
        let source = dir.join("source");
        std::fs::write(&source, b"fresh contents").unwrap();
        let sha = file_sha256(&source).unwrap();
        let dest = dir.join("download");
        download_verified(
            &format!("file://{}", source.display()),
            &dest,
            &sha,
            HashKind::Sha256,
            Some(&cache),
        )
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"fresh contents");
        // The verified artifact is cached under its digest for next time.
        assert!(
            cache_lookup(&cache, &sha, HashKind::Sha256)
                .unwrap()
                .is_some()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_cache_entry_is_rejected_and_replaced() {
        let dir = cache_test_dir("cache-corrupt");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let source = dir.join("source");
        std::fs::write(&source, b"real contents").unwrap();
        let sha = file_sha256(&source).unwrap();
        // A file at the right key with the wrong contents must not be trusted.
        std::fs::write(cache.join(cache_key(&sha, HashKind::Sha256)), b"tampered").unwrap();
        assert!(
            cache_lookup(&cache, &sha, HashKind::Sha256)
                .unwrap()
                .is_none()
        );

        let dest = dir.join("download");
        download_verified(
            &format!("file://{}", source.display()),
            &dest,
            &sha,
            HashKind::Sha256,
            Some(&cache),
        )
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"real contents");
        let cached = cache_lookup(&cache, &sha, HashKind::Sha256)
            .unwrap()
            .unwrap();
        assert!(verify_hash(&cached, &sha, HashKind::Sha256).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mismatched_download_is_never_cached() {
        let dir = cache_test_dir("cache-mismatch");
        let cache = dir.join("cache");
        let source = dir.join("source");
        std::fs::write(&source, b"real contents").unwrap();
        let dest = dir.join("download");
        let wrong = "ab".repeat(32);
        let error = download_verified(
            &format!("file://{}", source.display()),
            &dest,
            &wrong,
            HashKind::Sha256,
            Some(&cache),
        )
        .unwrap_err();
        assert!(error.contains("checksum mismatch"), "{error}");
        assert!(!dest.exists());
        assert!(
            !cache.is_dir() || std::fs::read_dir(&cache).unwrap().next().is_none(),
            "an unverified file must never enter the cache"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_download_leaves_no_cache_entry() {
        let dir = cache_test_dir("cache-failed");
        let cache = dir.join("cache");
        let sha = "cd".repeat(32);
        let dest = dir.join("download");
        let error = download_verified(
            "file:///definitely/missing",
            &dest,
            &sha,
            HashKind::Sha256,
            Some(&cache),
        )
        .unwrap_err();
        assert!(
            error.contains("could not copy") || error.contains("download failed"),
            "{error}"
        );
        assert!(!dest.exists());
        assert!(
            !cache.is_dir() || std::fs::read_dir(&cache).unwrap().next().is_none(),
            "a failed download must not create a trusted cache entry"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tools_dir_writability_is_checked() {
        let dir = cache_test_dir("writable");
        ensure_tools_dir_writable(&dir).expect("a temp directory is writable");
        // A regular file where a directory is expected cannot be written into.
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        assert!(ensure_tools_dir_writable(&file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runnable_strategies_are_available_and_gate_installation() {
        for &tool in Tool::ALL {
            let runnable = filter_runnable(tool.install_attempts());
            for attempt in &runnable {
                assert!(
                    attempt.steps.iter().all(step_available),
                    "{tool:?} kept a strategy with an unavailable step"
                );
            }
            // If no strategy can run, Koda must not claim the tool is installable.
            if runnable.is_empty() {
                assert!(!can_install(tool), "{tool:?} has no runnable strategy");
            }
        }
    }

    #[test]
    fn download_failure_hints_are_actionable() {
        assert!(download_failure_hint("could not resolve host: example.com").contains("DNS"));
        assert!(download_failure_hint("Failed to connect to proxy").contains("HTTPS_PROXY"));
        assert!(download_failure_hint("Operation timed out after 30s").contains("timed out"));
        assert!(download_failure_hint("SSL certificate problem").contains("TLS"));
        assert!(download_failure_hint("the requested URL returned error: 404").contains("retry"));
    }

    #[test]
    fn a_partial_plan_keeps_earlier_work_and_reports_the_failing_step() {
        let dir = cache_test_dir("partial-plan");
        let source = dir.join("source");
        std::fs::write(&source, b"good payload for the partial plan test").unwrap();
        let sha = file_sha256(&source).unwrap();

        // Step A of a plan succeeds.
        let a = dir.join("a");
        run_step(&InstallStep::Download {
            url: format!("file://{}", source.display()),
            dest: a.clone(),
            sha256: sha.clone(),
        })
        .expect("the first step succeeds");
        assert!(a.is_file());

        // Step B fails; the failure is explicit and earlier work remains. Koda
        // reports the incomplete plan rather than a false success.
        let b = dir.join("b");
        let error = run_step(&InstallStep::Download {
            url: "file:///definitely/missing".to_string(),
            dest: b.clone(),
            sha256: "cd".repeat(32),
        })
        .expect_err("the second step fails");
        assert!(!error.is_empty());
        assert!(!b.exists());
        assert!(a.is_file(), "earlier work in an incomplete plan remains");

        // Do not leave this test's payload in the real digest cache.
        if let Some(cache) = cache_dir() {
            let _ = std::fs::remove_file(cache.join(cache_key(&sha, HashKind::Sha256)));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_cache_round_trips_and_is_url_keyed() {
        let url = "https://example.invalid/koda-metadata-test";
        remember_metadata(url, "{\"body\":1}").unwrap();
        assert_eq!(cached_metadata(url).as_deref(), Some("{\"body\":1}"));
        assert_eq!(cached_metadata("https://example.invalid/other"), None);
        if let Some(path) = metadata_cache_path(url) {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn a_checksum_mismatch_is_rejected_before_install() {
        let dir = std::env::temp_dir().join(format!("koda-badsum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("payload.tar.gz");
        std::fs::write(&source, b"the real payload").unwrap();
        let dest = dir.join("download.tar.gz");
        // A valid-looking but wrong digest must abort the real step.
        let step = InstallStep::Download {
            url: format!("file://{}", source.display()),
            dest: dest.clone(),
            sha256: "00".repeat(32),
        };
        let error = run_step(&step).expect_err("a wrong checksum must fail");
        assert!(error.contains("checksum mismatch"), "{error}");
        assert!(
            !dest.exists(),
            "a mismatched download must not be left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_download_is_rejected_before_install() {
        let dir = std::env::temp_dir().join(format!("koda-truncated-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let full = b"a complete archive payload with enough bytes to truncate cleanly";
        let full_path = dir.join("full");
        std::fs::write(&full_path, full).unwrap();
        // The expected digest is the full file's; only half the bytes arrive.
        let Ok(expected) = file_sha256(&full_path) else {
            let _ = std::fs::remove_dir_all(&dir);
            return; // No hashing tool on this host.
        };
        let source = dir.join("truncated");
        std::fs::write(&source, &full[..full.len() / 2]).unwrap();
        let dest = dir.join("download");
        let step = InstallStep::Download {
            url: format!("file://{}", source.display()),
            dest: dest.clone(),
            sha256: expected,
        };
        let error = run_step(&step).expect_err("a truncated download must fail");
        assert!(error.contains("checksum mismatch"), "{error}");
        assert!(
            !dest.exists(),
            "a truncated download must not be left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_archive_fails_extraction() {
        if locate("tar").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("koda-shorttar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg/bin")).unwrap();
        std::fs::write(dir.join("pkg/bin/tool"), "#!/bin/sh\n").unwrap();
        let built = std::process::Command::new("tar")
            .current_dir(&dir)
            .args(["-czf", "pkg.tar.gz", "pkg"])
            .status();
        if !built.map(|status| status.success()).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let archive = std::fs::read(dir.join("pkg.tar.gz")).unwrap();
        std::fs::write(dir.join("short.tar.gz"), &archive[..archive.len() / 2]).unwrap();
        let result = extract(&dir.join("short.tar.gz"), &dir.join("out"), 1);
        assert!(
            result.is_err(),
            "a truncated archive must not extract successfully"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_extraction_preserves_the_existing_installation() {
        if locate("tar").is_none() {
            return;
        }
        let dir = cache_test_dir("extract-preserve");
        std::fs::create_dir_all(dir.join("dest")).unwrap();
        std::fs::write(dir.join("dest/keep"), "known-good").unwrap();
        let archive = dir.join("bad.tar.gz");
        std::fs::write(&archive, b"not a real gzip archive").unwrap();

        assert!(extract(&archive, &dir.join("dest"), 0).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("dest/keep")).unwrap(),
            "known-good",
            "a failed extraction must not touch the existing installation"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".koda-"))
            .map(|entry| entry.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging must be cleaned up: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_successful_extraction_replaces_the_installation() {
        if locate("tar").is_none() {
            return;
        }
        let dir = cache_test_dir("extract-replace");
        std::fs::create_dir_all(dir.join("pkg/bin")).unwrap();
        std::fs::write(dir.join("pkg/bin/tool"), "#!/bin/sh\n").unwrap();
        let built = std::process::Command::new("tar")
            .current_dir(&dir)
            .args(["-czf", "pkg.tar.gz", "pkg"])
            .status();
        if !built.map(|status| status.success()).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let dest = dir.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("old"), "v1").unwrap();

        extract(&dir.join("pkg.tar.gz"), &dest, 1).expect("a safe archive extracts");
        assert!(dest.join("bin/tool").is_file());
        assert!(
            !dest.join("old").exists(),
            "the promoted tree replaces the previous installation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_promotion_restores_the_previous_installation() {
        let dir = cache_test_dir("promote-restore");
        let dest = dir.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("keep"), "known-good").unwrap();
        // A staging directory that does not exist makes the promote rename fail.
        let missing = dir.join(".missing-staging");
        let error = promote_directory(&missing, &dest).unwrap_err();
        assert!(error.contains("could not install into"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dest.join("keep")).unwrap(),
            "known-good",
            "a failed promotion must restore the previous installation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_zip_works_with_every_available_extractor() {
        if locate("python3").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("koda-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg/bin")).unwrap();
        std::fs::write(dir.join("pkg/bin/tool"), "x").unwrap();
        let archive = dir.join("pkg.zip");
        let built = std::process::Command::new("python3")
            .current_dir(&dir)
            .args([
                "-c",
                "import zipfile;z=zipfile.ZipFile('pkg.zip','w');z.write('pkg/bin/tool','pkg/bin/tool')",
            ])
            .status();
        if !built.map(|status| status.success()).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        for (extractor, tool) in [
            (ZipExtractor::Unzip, "unzip"),
            (ZipExtractor::BsdTar, "bsdtar"),
            (ZipExtractor::Python, "python3"),
        ] {
            if locate(tool).is_none() {
                continue;
            }
            // The traversal safety check must list the archive with the same
            // extractor that unpacks it (regression: it once hardcoded `unzip`,
            // so a system without it refused a perfectly safe archive).
            let listed = zip_listing(extractor, &archive)
                .unwrap_or_else(|err| panic!("{tool} listing: {err}"));
            assert!(
                listed.contains("pkg/bin/tool"),
                "{tool} did not list the archive: {listed:?}"
            );
            let out = dir.join(format!("out-{tool}"));
            extract_zip_with(extractor, &archive, &out, 1)
                .unwrap_or_else(|err| panic!("{tool}: {err}"));
            assert!(out.join("bin/tool").is_file(), "{tool} did not extract");
        }
        // The public entry point picks a working extractor automatically.
        let out = dir.join("out-auto");
        extract(&archive, &out, 1).expect("zip extracts");
        assert!(out.join("bin/tool").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strip_move_relocates_a_single_root_directory() {
        // `unzip` has no `--strip-components`, so a stripped zip is unpacked to
        // a scratch directory and its single root is moved up. Regression: the
        // Kotlin server once landed under `server/` because the strip was
        // silently ignored.
        let dir = std::env::temp_dir().join(format!("koda-strip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let scratch = dir.join("scratch");
        std::fs::create_dir_all(scratch.join("pkg/bin")).unwrap();
        std::fs::write(scratch.join("pkg/bin/tool"), "#!/bin/sh\n").unwrap();
        std::fs::write(scratch.join("pkg/data.txt"), "data").unwrap();

        let root = single_child_directory(&scratch).expect("a single root");
        assert!(root.ends_with("pkg"));
        let dest = dir.join("dest");
        move_directory_contents(&root, &dest).unwrap();
        assert!(dest.join("bin/tool").is_file());
        assert!(dest.join("data.txt").is_file());

        // Two top-level entries cannot be stripped by descending.
        std::fs::create_dir_all(scratch.join("other")).unwrap();
        assert!(single_child_directory(&scratch).is_none());

        let _ = std::fs::remove_dir_all(&dir);
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
    fn clangd_serves_c_and_cpp_and_provisions_the_official_bundle() {
        assert!(Tool::Clangd.serves(LanguageId::C));
        assert!(Tool::Clangd.serves(LanguageId::Cpp));
        assert_eq!(
            Tool::for_language(LanguageId::Cpp, ToolPurpose::LanguageServer),
            Some(Tool::Clangd)
        );
        let attempts = Tool::Clangd.install_attempts();
        if attempts.is_empty() {
            // No fetchable bundle: musl (the release is glibc-only), a platform
            // clangd publishes no asset for, or no tools directory to install
            // into. Any other reason would be a bug.
            assert!(
                libc_is_musl() || clangd_asset().is_none() || tools_dir().is_none(),
                "clangd must have an install plan when its bundle is offered"
            );
            return;
        }
        let steps = &attempts.first().expect("a clangd plan").steps;
        assert!(
            steps.iter().any(|step| matches!(
                step,
                InstallStep::GithubRelease { repo, .. } if repo == "clangd/clangd"
            )),
            "clangd must come from the official clangd release: {steps:?}"
        );
        assert!(
            steps
                .iter()
                .any(|step| matches!(step, InstallStep::Extract { strip: 1, .. })),
            "the clangd bundle root must be stripped: {steps:?}"
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
                        && command.args.iter().any(|arg| arg == TYPESCRIPT_LS_SPEC)
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
            Tool::Pylsp.install_command(),
            Some(("pipx", &["install", PYTHON_LSP_SPEC][..]))
        );
        // Go tooling is provisioned by Koda's managed Go plan.
        assert_eq!(Tool::Gopls.install_command(), None);
        assert_eq!(Tool::Sqls.install_command(), None);
        assert_eq!(
            Tool::BashLs.install_command(),
            Some(("npm", &["install", "-g", BASH_LS_SPEC][..]))
        );
        // Python tooling has several fallbacks, so installation is attempted
        // even without pipx or a working `pip`.
        assert!(Tool::Pylsp.install_attempts().len() >= 2);
        assert_eq!(Tool::Gofmt.install_command(), None);
        // `gofmt` ships with Go, so its plan provisions the Go toolchain.
        if go_platform().is_some() && tools_dir().is_some() {
            assert!(
                Tool::Gofmt.install_attempts().iter().any(|attempt| attempt
                    .steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::GoToolchain { .. }))),
                "gofmt must be installable through the Go toolchain"
            );
        } else {
            assert!(Tool::Gofmt.install_attempts().is_empty());
        }
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
            error: None,
        };
        assert_eq!(available.summary(), "rustfmt 1.8.0");

        let missing = ToolStatus {
            tool: Tool::Rustfmt,
            available: false,
            version: None,
            path: None,
            error: None,
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

    #[test]
    fn known_bin_dirs_cover_common_toolchains() {
        let dirs = known_bin_dirs();
        for suffix in [
            ".mix/escripts",
            ".asdf/shims",
            "perl5/bin",
            "development/flutter/bin/cache/dart-sdk/bin",
        ] {
            assert!(
                dirs.iter().any(|dir| dir.ends_with(suffix)),
                "discovery should search a directory ending in {suffix}"
            );
        }
        for absolute in [
            "/usr/local/swift/usr/bin",
            "/usr/lib/swift/bin",
            "/snap/bin",
        ] {
            assert!(
                dirs.iter().any(|dir| dir == Path::new(absolute)),
                "discovery should search {absolute}"
            );
        }
    }

    #[test]
    fn html_and_css_are_probed_by_starting_their_server() {
        // Regression: the extracted VS Code servers reject `--version`, so a
        // version probe reported a successful install as missing.
        for tool in [Tool::HtmlLs, Tool::CssLs, Tool::KotlinLs, Tool::AsmLsp] {
            assert!(
                tool.probe_as_server(),
                "{tool:?} must be probed by launching its stdio server"
            );
        }
        for tool in [Tool::RustAnalyzer, Tool::BashLs, Tool::TypeScriptLs] {
            assert!(
                !tool.probe_as_server(),
                "{tool:?} answers a version query and must keep the cheap probe"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn server_probe_accepts_a_live_server_and_rejects_a_broken_launcher() {
        use std::process::{Command, Stdio};

        // A server still running at the deadline is usable.
        let mut live = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        assert!(alive_or_clean(&mut live).0);

        // A launcher that fails immediately (the broken-Node symptom) is not.
        let mut broken = Command::new("sh")
            .args(["-c", "exit 1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        assert!(!alive_or_clean(&mut broken).0);

        // A clean one-shot exit is still a usable binary.
        let mut clean = Command::new("sh")
            .args(["-c", "exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        assert!(alive_or_clean(&mut clean).0);
    }

    #[test]
    fn npm_tools_can_provision_a_managed_node_runtime() {
        if !node_provisionable() {
            // No `curl`/archive tool for this platform: nothing to assert.
            return;
        }
        for tool in [Tool::HtmlLs, Tool::CssLs, Tool::BashLs, Tool::TypeScriptLs] {
            let attempts = tool.install_attempts();
            assert!(
                attempts.iter().any(|attempt| attempt
                    .steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::NodeRuntime { .. }))),
                "{tool:?} must be installable without a system Node.js: {attempts:?}"
            );
            // The managed runtime is downloaded, unpacked and then used to run npm.
            let managed = attempts
                .iter()
                .find(|attempt| {
                    attempt
                        .steps
                        .iter()
                        .any(|step| matches!(step, InstallStep::NodeRuntime { .. }))
                })
                .expect("a managed Node plan");
            assert!(
                managed
                    .steps
                    .iter()
                    .any(|step| matches!(step, InstallStep::Extract { .. })),
                "the Node archive must be extracted"
            );
            assert!(
                run_commands(managed)
                    .into_iter()
                    .any(|command| is_user_bin_program(&command.program)),
                "the managed npm must be the runtime Koda just installed"
            );
        }
    }

    #[test]
    fn managed_bin_dirs_are_searched_when_node_is_managed() {
        if let Some(node) = node_bin_dir() {
            assert!(
                known_bin_dirs().iter().any(|dir| dir == &node),
                "Koda's managed Node.js bin directory should be searched"
            );
        }
    }

    #[test]
    fn node_checksum_lookup_rejects_malformed_entries() {
        let asset = "node-v24.21.0-linux-x64.tar.xz";
        let sum = "fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6";
        let body = format!(
            "aec7b2464afb99f078c19cb06d201d543bd3b311cba071282bce1b17c97e58bb  node-v24.21.0-aix-ppc64.tar.gz\n\
             {sum}  {asset}\n\
             22ca85110f26015696a3fa9216bc372ae65203d170622eaf7d211e2dd5bb49e3  node-v24.21.0-arm64.msi\n"
        );
        assert_eq!(checksum_for(&body, asset).as_deref(), Some(sum));

        // An absent asset, a short hash and a lone line never yield a checksum.
        assert_eq!(
            checksum_for(&body, "node-v24.21.0-linux-arm64.tar.xz"),
            None
        );
        assert_eq!(checksum_for("not-a-hash  some-file", "some-file"), None);
        assert_eq!(checksum_for("deadbeef", "deadbeef"), None);
        assert_eq!(checksum_for("", asset), None);
        // A binary-mode `*` prefix is tolerated.
        assert_eq!(
            checksum_for(&format!("{sum}  *{asset}"), asset).as_deref(),
            Some(sum)
        );
    }

    #[test]
    fn managed_node_asset_matches_the_pinned_version() {
        if let Some(asset) = node_asset() {
            assert!(
                asset.starts_with(&format!("node-v{NODE_VERSION}-")),
                "asset should carry the pinned version: {asset}"
            );
            assert!(
                asset.ends_with(".tar.xz") || asset.ends_with(".tar.gz") || asset.ends_with(".zip"),
                "asset should be an extractable archive: {asset}"
            );
        }
    }
}
