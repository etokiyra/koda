//! Live smoke tests for the HTML/CSS language servers.
//!
//! These require Koda's managed `vscode-langservers-extracted` package (or the
//! servers on `PATH`) and are therefore `#[ignore]`d by default. They are the
//! closest automated check to the real user workflow:
//!
//! ```sh
//! cargo test --test html_css_tooling -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! The deterministic regression tests for the provisioning and crash fixes live
//! next to the code (`language::tools` and `language::html`).

use std::path::Path;
use std::time::{Duration, Instant};

use koda::language::id::LanguageId;
use koda::language::lsp::Server;
use koda::language::tools::{Tool, ToolRegistry};

fn handshake(tool: Tool, language: LanguageId) {
    let registry = ToolRegistry::discover();
    let Some(path) = registry.program_path(tool).map(Path::to_path_buf) else {
        panic!("{tool:?} is not installed; run Language Setup first");
    };
    let root = std::env::temp_dir().join("koda-live-html");
    std::fs::create_dir_all(&root).expect("temp root");

    let mut server = Server::start(language, path.to_str().unwrap(), tool.server_args(), &root)
        .expect("start server");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !server.is_ready() {
        for _ in server.poll() {}
        assert!(
            Instant::now() < deadline,
            "{tool:?} never completed its LSP handshake"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires the HTML language server to be installed"]
fn html_server_completes_the_handshake() {
    handshake(Tool::HtmlLs, LanguageId::Html);
}

#[test]
#[ignore = "requires the CSS language server to be installed"]
fn css_server_completes_the_handshake() {
    handshake(Tool::CssLs, LanguageId::Css);
}

#[test]
#[ignore = "requires the HTML/CSS language servers to be installed"]
fn html_and_css_are_discovered_after_install() {
    let registry = ToolRegistry::discover();
    for tool in [Tool::HtmlLs, Tool::CssLs] {
        let status = registry.status(tool).expect("status");
        assert!(
            status.available,
            "{tool:?} must be available once installed (probe regression)"
        );
        assert!(status.path.is_some(), "{tool:?} must resolve a path");
    }
}
