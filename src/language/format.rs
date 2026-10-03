//! Formatting through mature external tools.
//!
//! Koda does not reimplement formatting. When a language has a trusted
//! formatter it runs it on a buffer snapshot (the tools read from stdin, so
//! unsaved edits are included), entirely on the background worker. If the tool
//! is missing, Koda says exactly what is missing instead of failing silently.
//!
//! This is the "reuse system tools" half of the zero-configuration promise;
//! missing tools are installed through their official package managers from
//! **Language Setup…**.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The longest a formatter may run before it is killed.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most formatter output Koda keeps; the excess is drained and discarded.
const MAX_FORMAT_OUTPUT: usize = 8 * 1024 * 1024;

/// The result of asking a provider to format a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatOutcome {
    /// The formatted document.
    Formatted(String),
    /// The provider has no formatter for this language.
    Unsupported,
    /// The formatter executable is not installed.
    ToolMissing { tool: String, hint: String },
    /// The formatter ran but reported a problem.
    Failed(String),
}

/// Whether an executable named `tool` can be found on `PATH`.
///
/// Used to tell the user up front that a formatter is missing instead of
/// failing only when they ask for it.
pub fn is_available(tool: &str) -> bool {
    crate::language::tools::locate(tool).is_some()
}

/// Format Rust source with `rustfmt`, matching the project's edition when a
/// `Cargo.toml` can be found above `path`.
pub fn rustfmt(path: &Path, text: &str) -> FormatOutcome {
    let edition = rust_edition_for(path);
    run(
        "rustfmt",
        &["--edition", &edition, "--emit", "stdout"],
        text,
        "rustfmt",
        "install it with `rustup component add rustfmt`",
    )
}

/// Format Go source with `gofmt`.
pub fn gofmt(text: &str) -> FormatOutcome {
    run(
        "gofmt",
        &[],
        text,
        "gofmt",
        "it ships with the Go toolchain",
    )
}

fn run(program: &str, args: &[&str], text: &str, tool: &str, hint: &str) -> FormatOutcome {
    // Prefer a located executable: the process PATH may be minimal when Koda is
    // launched from a GUI or a non-login shell.
    let program_path = crate::language::tools::locate(program)
        .unwrap_or_else(|| std::path::PathBuf::from(program));
    let mut child = match Command::new(&program_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return FormatOutcome::ToolMissing {
                tool: tool.to_string(),
                hint: hint.to_string(),
            };
        }
        Err(err) => return FormatOutcome::Failed(format!("could not run {tool}: {err}")),
    };

    // Feed the snapshot and close stdin so the formatter knows the input ended.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }

    let captured =
        match crate::process::wait_captured(&mut child, FORMAT_TIMEOUT, MAX_FORMAT_OUTPUT) {
            Ok(captured) => captured,
            Err(err) => return FormatOutcome::Failed(format!("{tool} failed: {err}")),
        };
    let Some(status) = captured.status else {
        return FormatOutcome::Failed(format!(
            "{tool} timed out after {}s",
            FORMAT_TIMEOUT.as_secs()
        ));
    };

    if status.success() {
        let formatted = String::from_utf8_lossy(&captured.stdout).into_owned();
        if formatted.trim().is_empty() {
            FormatOutcome::Failed(format!("{tool} produced no output"))
        } else {
            FormatOutcome::Formatted(formatted)
        }
    } else {
        let stderr = String::from_utf8_lossy(&captured.stderr);
        let message = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("unknown error")
            .trim()
            .to_string();
        FormatOutcome::Failed(format!("{tool}: {message}"))
    }
}

/// The Rust edition of the project containing `path`, defaulting to 2021.
fn rust_edition_for(path: &Path) -> String {
    for dir in path.ancestors() {
        if let Ok(contents) = std::fs::read_to_string(dir.join("Cargo.toml"))
            && let Some(edition) = parse_edition(&contents)
        {
            return edition;
        }
    }
    "2021".to_string()
}

/// Extract `edition = "…"` from a `Cargo.toml`.
fn parse_edition(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("edition") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches(['"', '\'']).trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cargo_edition() {
        let cargo = "[package]\nname = \"x\"\nedition = \"2024\"\n";
        assert_eq!(parse_edition(cargo).as_deref(), Some("2024"));
        assert_eq!(parse_edition("[package]\nname = \"x\"\n"), None);
    }

    #[test]
    fn missing_program_is_not_available() {
        assert!(!is_available("koda-definitely-not-a-real-tool"));
    }

    #[test]
    fn rustfmt_formats_when_installed() {
        // Skip silently when the tool is unavailable (offline/CI sandbox).
        if Command::new("rustfmt").arg("--version").output().is_err() {
            return;
        }
        let path = std::env::temp_dir().join("koda-format-test.rs");
        let outcome = rustfmt(&path, "fn main(){let x=1;}\n");
        match outcome {
            FormatOutcome::Formatted(formatted) => {
                assert!(formatted.contains("fn main() {"));
                assert!(formatted.contains("    let x = 1;"));
            }
            other => panic!("expected rustfmt to format, got {other:?}"),
        }
    }

    #[test]
    fn missing_tool_is_reported() {
        let outcome = run(
            "koda-definitely-not-a-real-formatter",
            &[],
            "x",
            "fakefmt",
            "do nothing",
        );
        assert_eq!(
            outcome,
            FormatOutcome::ToolMissing {
                tool: "fakefmt".to_string(),
                hint: "do nothing".to_string(),
            }
        );
    }
}
