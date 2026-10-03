//! Live smoke tests for language servers Koda discovers or installs.
//!
//! Every test is `#[ignore]`d and **skips silently when its tool is not
//! available**, so it only asserts on a machine that actually has the server.
//! This lets Koda's CI stay offline while still documenting and exercising the
//! real handshake where a toolchain exists:
//!
//! ```sh
//! XDG_DATA_HOME=/tmp/koda-tools cargo run --example install_check -- sql
//! cargo test --test optional_servers -- --ignored --test-threads=1 --nocapture
//! ```

use std::time::{Duration, Instant};

use koda::language::id::LanguageId;
use koda::language::lsp::Server;
use koda::language::tools::{Tool, ToolRegistry, launch_env};

fn handshake(tool: Tool, language: LanguageId) {
    handshake_with(tool, language, |_| {});
}

fn handshake_with(tool: Tool, language: LanguageId, prepare: impl FnOnce(&std::path::Path)) {
    let registry = ToolRegistry::discover();
    // A tool is usable only when its probe succeeds: `perl` may exist while
    // `Perl::LanguageServer` is not installed, or `dart` may be absent.
    if !registry.available(tool) {
        eprintln!("skipping {tool:?}: not available");
        return;
    }
    let Some(path) = registry.program_path(tool).map(|p| p.to_path_buf()) else {
        eprintln!("skipping {tool:?}: no executable path");
        return;
    };
    let root = std::env::temp_dir().join("koda-live-optional");
    std::fs::create_dir_all(&root).expect("temp root");
    prepare(&root);
    let env = launch_env(tool);
    let mut server = Server::start_with_env(
        language,
        path.to_str().unwrap(),
        tool.server_args(),
        &root,
        &env,
    )
    .unwrap_or_else(|err| panic!("could not start {tool:?}: {err}"));

    let deadline = Instant::now() + Duration::from_secs(60);
    while !server.is_ready() {
        for _ in server.poll() {}
        assert!(
            Instant::now() < deadline,
            "{tool:?} never completed its LSP handshake"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("{tool:?} handshake ok");
}

#[test]
#[ignore = "requires sqls"]
fn sql_handshake() {
    handshake(Tool::Sqls, LanguageId::Sql);
}

#[test]
#[ignore = "requires asm-lsp"]
fn assembly_handshake() {
    handshake(Tool::AsmLsp, LanguageId::Assembly);
}

#[test]
#[ignore = "requires solargraph"]
fn ruby_handshake() {
    handshake(Tool::RubyLs, LanguageId::Ruby);
}

#[test]
#[ignore = "requires the Dart SDK"]
fn dart_handshake() {
    handshake(Tool::DartAnalyzer, LanguageId::Dart);
}

#[test]
#[ignore = "requires ElixirLS + Erlang/Elixir"]
fn elixir_handshake() {
    handshake(Tool::ElixirLs, LanguageId::Elixir);
}

#[test]
#[ignore = "requires the Swift toolchain"]
fn swift_handshake() {
    handshake(Tool::SwiftLs, LanguageId::Swift);
}

#[test]
#[ignore = "requires the Swift toolchain"]
fn swift_package_handshake() {
    // Exercise SwiftPM project-root handling: a `Package.swift` and a source
    // file make sourcekit-lsp treat the root as a package.
    handshake_with(Tool::SwiftLs, LanguageId::Swift, |root| {
        let _ = std::fs::write(
            root.join("Package.swift"),
            "// swift-tools-version:5.9\nimport PackageDescription\nlet package = Package(name: \"Probe\")\n",
        );
        let sources = root.join("Sources/Probe");
        let _ = std::fs::create_dir_all(&sources);
        let _ = std::fs::write(sources.join("main.swift"), "print(\"hi\")\n");
    });
}

#[test]
#[ignore = "requires Perl::LanguageServer"]
fn perl_handshake() {
    handshake(Tool::PerlLs, LanguageId::Perl);
}

#[test]
#[ignore = "requires PLS"]
fn pls_handshake() {
    handshake(Tool::Pls, LanguageId::Perl);
}
