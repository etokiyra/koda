//! Live smoke test for the managed Kotlin language server and its dedicated
//! JDK 21.
//!
//! `kotlin-language-server` cannot run on Koda's managed JDK 25 (its bundled
//! compiler rejects the four-part version string), so Koda installs and launches
//! it with a separate JDK 21. This test proves the pair actually starts:
//!
//! ```sh
//! XDG_DATA_HOME=/tmp/koda-tools cargo run --example install_check -- kotlin
//! XDG_DATA_HOME=/tmp/koda-tools cargo test --test kotlin_tooling -- --ignored
//! ```

use std::time::{Duration, Instant};

use koda::language::id::LanguageId;
use koda::language::lsp::Server;
use koda::language::tools::{Tool, ToolRegistry, launch_env};

#[test]
#[ignore = "requires the Kotlin language server and its managed JDK 21"]
fn kotlin_server_completes_the_handshake() {
    let registry = ToolRegistry::discover();
    let Some(path) = registry
        .program_path(Tool::KotlinLs)
        .map(|p| p.to_path_buf())
    else {
        panic!("kotlin-language-server is not installed; run the install_check example first");
    };
    let root = std::env::temp_dir().join("koda-live-kotlin");
    std::fs::create_dir_all(&root).expect("temp root");

    let env = launch_env(Tool::KotlinLs);
    assert!(
        env.iter().any(|(key, _)| key == "JAVA_HOME"),
        "the Kotlin server must be launched with its dedicated JDK"
    );

    let mut server = Server::start_with_env(
        LanguageId::Kotlin,
        path.to_str().unwrap(),
        Tool::KotlinLs.server_args(),
        &root,
        &env,
    )
    .expect("start server");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !server.is_ready() {
        for _ in server.poll() {}
        assert!(
            Instant::now() < deadline,
            "kotlin-language-server never completed its LSP handshake"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
