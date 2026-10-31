//! Pluggable command execution.
//!
//! Koda runs a handful of external programs (`git`, formatters, language
//! servers' helper commands). Today they all run on the local machine. This
//! module abstracts *how* a command is run behind a trait so a future
//! remote (SSH) backend can reuse the same call sites without rewriting them.
//!
//! Only a local implementation exists today; the SSH backend is not wired.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::process::{wait_captured, Captured};

/// A runner that can execute a program with arguments in a working directory.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Captured, String>;
}

/// The default, local implementation: spawns the program on this machine.
#[derive(Clone, Copy, Default, Debug)]
pub struct LocalRunner;

impl CommandRunner for LocalRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Captured, String> {
        let mut command = Command::new(program);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|err| err.to_string())?;
        wait_captured(&mut child, std::time::Duration::from_secs(30), 1024 * 1024)
            .map_err(|err| err.to_string())
    }
}

/// An SSH-backed runner: shells out to `ssh` for a configured host.
#[derive(Clone, Debug)]
pub struct SshRunner {
    pub host: String,
}

impl CommandRunner for SshRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Captured, String> {
        // Best-effort: run the command on the remote host. The working
        // directory is forwarded as a `cd` so project-relative commands keep
        // working; quoting is minimal and assumes POSIX paths.
        let mut remote_cmd = String::new();
        if let Some(dir) = cwd {
            remote_cmd.push_str("cd ");
            remote_cmd.push_str(&shell_quote(dir));
            remote_cmd.push_str(" && ");
        }
        remote_cmd.push_str(program);
        for arg in args {
            remote_cmd.push(' ');
            remote_cmd.push_str(&shell_quote_str(arg));
        }
        let mut command = Command::new("ssh");
        command
            .arg(&self.host)
            .arg(remote_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|err| err.to_string())?;
        wait_captured(&mut child, std::time::Duration::from_secs(30), 1024 * 1024)
            .map_err(|err| err.to_string())
    }
}

fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    // Single-quote, escaping embedded single quotes.
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn shell_quote_str(s: &str) -> String {
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Build a runner from a remote spec. `None` (or empty) means local.
pub fn runner_for(remote: Option<&str>) -> Box<dyn CommandRunner> {
    match remote {
        Some(host) if !host.trim().is_empty() => Box::new(SshRunner {
            host: host.to_string(),
        }),
        _ => Box::new(LocalRunner),
    }
}

// Keep PathBuf in scope for future backends.
#[allow(dead_code)]
fn _pathbuf(_p: PathBuf) {}
