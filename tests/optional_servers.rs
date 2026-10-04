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
use koda::language::lsp::{RequestKind, Server, ServerEvent};
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

#[test]
#[ignore = "requires clangd"]
fn clangd_handshake() {
    handshake(Tool::Clangd, LanguageId::C);
}

#[test]
#[ignore = "requires gopls + Go"]
fn gopls_handshake() {
    handshake(Tool::Gopls, LanguageId::Go);
}

#[test]
#[ignore = "requires phpactor + PHP"]
fn phpactor_handshake() {
    handshake(Tool::Phpactor, LanguageId::Php);
}

/// Open `text` in a live server and wait until it is initialized.
fn serve_text(
    tool: Tool,
    language: LanguageId,
    file: &str,
    text: &str,
) -> Option<(Server, std::path::PathBuf)> {
    let registry = ToolRegistry::discover();
    if !registry.available(tool) {
        eprintln!("skipping {tool:?}: not available");
        return None;
    }
    let path = registry.program_path(tool)?.to_path_buf();
    // A unique root per call: tests may run in parallel and must not delete
    // each other's directory.
    let unique = format!(
        "koda-live-roundtrip-{}-{}-{}",
        file.replace(['/', '.'], "_"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0)
    );
    let root = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&root).expect("temp root");
    let document = root.join(file);
    std::fs::write(&document, text).expect("write document");
    let mut server = Server::start_with_env(
        language,
        path.to_str().unwrap(),
        tool.server_args(),
        &root,
        &launch_env(tool),
    )
    .unwrap_or_else(|err| panic!("could not start {tool:?}: {err}"));
    let deadline = Instant::now() + Duration::from_secs(60);
    while !server.is_ready() {
        for _ in server.poll() {}
        assert!(Instant::now() < deadline, "{tool:?} never became ready");
        std::thread::sleep(Duration::from_millis(50));
    }
    server.did_open(&document, text);
    Some((server, document))
}

fn completion_round_trip(
    tool: Tool,
    language: LanguageId,
    file: &str,
    text: &str,
    line: u32,
    col: u32,
    require_items: bool,
) {
    let Some((mut server, document)) = serve_text(tool, language, file, text) else {
        return;
    };
    let id = server
        .completion(&document, line as usize, col as usize)
        .unwrap_or_else(|| panic!("{tool:?} did not send a completion request"));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for event in server.poll() {
            if let ServerEvent::Response {
                kind: RequestKind::Completion,
                id: got,
                result,
            } = event
                && got == id
            {
                let value =
                    result.unwrap_or_else(|err| panic!("{tool:?} completion failed: {err}"));
                let count = value
                    .get("items")
                    .and_then(|items| items.as_array())
                    .or_else(|| value.as_array())
                    .map(|items| items.len())
                    .unwrap_or(0);
                assert!(
                    !require_items || count > 0,
                    "{tool:?} returned no completions: {value}"
                );
                eprintln!("{tool:?} completion ok ({count} items)");
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "{tool:?} never answered the completion request"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires clangd"]
fn clangd_completion_round_trip() {
    // Member completion needs no system headers and is a reliable trigger.
    let text = "struct Point { int x; int y; };\nvoid f() {\n  struct Point p;\n  p.\n}\n";
    completion_round_trip(Tool::Clangd, LanguageId::C, "probe.c", text, 3, 4, true);
}

#[test]
#[ignore = "requires gopls + Go"]
fn gopls_completion_round_trip() {
    let text = "package main\n\nfunc main() {\n\tvar x int\n\tx = \n}\n";
    completion_round_trip(Tool::Gopls, LanguageId::Go, "main.go", text, 4, 5, true);
}

fn diagnostics_round_trip(tool: Tool, language: LanguageId, file: &str, text: &str) {
    let Some((mut server, _document)) = serve_text(tool, language, file, text) else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for event in server.poll() {
            if let ServerEvent::Diagnostics { diagnostics, .. } = event
                && !diagnostics.is_empty()
            {
                eprintln!("{tool:?} diagnostics ok ({} items)", diagnostics.len());
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "{tool:?} published no diagnostics"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires clangd"]
fn clangd_diagnostics_round_trip() {
    let text = "int main() {\n  return x;\n}\n";
    diagnostics_round_trip(Tool::Clangd, LanguageId::C, "broken.c", text);
}

#[test]
#[ignore = "requires PLS"]
fn pls_completion_round_trip() {
    // A partial package name after `use` is a stable completion trigger.
    let text = "use str\n";
    completion_round_trip(Tool::Pls, LanguageId::Perl, "probe.pl", text, 0, 8, false);
}

#[test]
#[ignore = "requires sqls"]
fn sqls_completion_round_trip() {
    let text = "SELECT * FROM users WHERE ";
    completion_round_trip(Tool::Sqls, LanguageId::Sql, "probe.sql", text, 0, 26, false);
}
