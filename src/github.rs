//! GitHub CLI (`gh`) integration.
//!
//! Koda shells out to the user's `gh` binary when present. It never stores
//! credentials itself; authentication is delegated to `gh auth`.

use std::path::Path;
use std::process::Command;

/// Whether `gh` is available on PATH.
pub fn available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The logged-in account, if any.
pub fn auth_status() -> Option<String> {
    let output = Command::new("gh").args(["auth", "status"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .find(|line| line.contains("Logged in"))
        .map(|line| line.trim().to_string())
        .or_else(|| Some("logged in".to_string()))
}

/// Create a GitHub repository for the project and push it.
pub fn create_repo(root: &Path, name: &str, private: bool) -> Result<String, String> {
    let visibility = if private { "--private" } else { "--public" };
    let output = Command::new("gh")
        .arg("repo")
        .arg("create")
        .arg(name)
        .arg(visibility)
        .arg("--source")
        .arg(root)
        .arg("--remote")
        .arg("origin")
        .arg("--push")
        .current_dir(root)
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(if stdout.trim().is_empty() {
            "Repository created and pushed".to_string()
        } else {
            stdout.trim().to_string()
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.trim().to_string())
    }
}
