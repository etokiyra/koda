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
        Some("html") => koda::language::tools::Tool::HtmlLs,
        Some("css") => koda::language::tools::Tool::CssLs,
        Some("lua") => koda::language::tools::Tool::LuaLs,
        Some("kotlin") => koda::language::tools::Tool::KotlinLs,
        Some("sql") => koda::language::tools::Tool::Sqls,
        Some("asm") => koda::language::tools::Tool::AsmLsp,
        Some("dart") => koda::language::tools::Tool::DartAnalyzer,
        Some("elixir") => koda::language::tools::Tool::ElixirLs,
        Some("swift") => koda::language::tools::Tool::SwiftLs,
        Some("perl") => koda::language::tools::Tool::PerlLs,
        Some("pls") => koda::language::tools::Tool::Pls,
        other => {
            eprintln!(
                "usage: install_check <omnisharp|jdtls|html|css|lua|kotlin|sql|asm|dart|elixir|swift|perl|pls> (got {other:?})"
            );
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
