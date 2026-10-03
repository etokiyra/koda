//! Dev harness: install a managed tool through Koda's real provisioning path.
//!
//! ```text
//! XDG_DATA_HOME=/tmp/koda-tools cargo run --example install_check -- omnisharp
//! ```
//!
//! Not part of the app; it exists so the managed download/verify/extract flow
//! can be exercised against the real network and probed, end to end.

fn main() {
    let tool = match std::env::args().nth(1).as_deref() {
        Some("omnisharp") => koda::language::tools::Tool::OmniSharp,
        Some("jdtls") => koda::language::tools::Tool::Jdtls,
        other => {
            eprintln!("usage: install_check <omnisharp|jdtls> (got {other:?})");
            std::process::exit(2);
        }
    };

    println!("installing {:?}…", tool);
    match koda::language::tools::install(tool) {
        Ok(message) => {
            println!("OK: {message}");
            let registry = koda::language::tools::ToolRegistry::discover();
            if let Some(status) = registry.status(tool) {
                println!(
                    "probe: available={} version={:?}",
                    status.available, status.version
                );
            }
        }
        Err(error) => {
            eprintln!("ERR: {error}");
            std::process::exit(1);
        }
    }
}
