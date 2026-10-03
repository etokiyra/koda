//! Live smoke test for the managed Lua language server.
//!
//! `lua-language-server` is self-contained and provisioned by Koda, so this is
//! the closest automated check to the real workflow. It is `#[ignore]`d because
//! it needs the tool installed:
//!
//! ```sh
//! XDG_DATA_HOME=/tmp/koda-tools cargo run --example install_check -- lua
//! XDG_DATA_HOME=/tmp/koda-tools cargo test --test lua_tooling -- --ignored
//! ```

use std::time::{Duration, Instant};

use koda::language::id::LanguageId;
use koda::language::lsp::Server;
use koda::language::tools::{Tool, ToolRegistry};

#[test]
#[ignore = "requires the Lua language server to be installed"]
fn lua_server_completes_the_handshake() {
    let registry = ToolRegistry::discover();
    let Some(path) = registry.program_path(Tool::LuaLs).map(|p| p.to_path_buf()) else {
        panic!("lua-language-server is not installed; run the install_check example first");
    };
    let root = std::env::temp_dir().join("koda-live-lua");
    std::fs::create_dir_all(&root).expect("temp root");

    let mut server = Server::start(
        LanguageId::Lua,
        path.to_str().unwrap(),
        Tool::LuaLs.server_args(),
        &root,
    )
    .expect("start server");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !server.is_ready() {
        for _ in server.poll() {}
        assert!(
            Instant::now() < deadline,
            "lua-language-server never completed its LSP handshake"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
