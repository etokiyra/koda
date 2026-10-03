//! Bounded, timed subprocess capture.
//!
//! External tools (formatters, package managers, `git`) are run as children and
//! must never be able to hang Koda forever or exhaust memory with unbounded
//! output. This module captures both pipes on their own threads — so a child
//! cannot deadlock on a full pipe — caps the bytes kept, and kills the child if
//! it runs past a deadline.

use std::io::{self, Read};
use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

/// How often the child is polled while waiting for it to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The captured result of a child process.
pub struct Captured {
    /// The exit status, or `None` when the child had to be killed because it
    /// ran past the timeout.
    pub status: Option<ExitStatus>,
    /// Up to the caller's cap of stdout, with the rest drained and discarded.
    pub stdout: Vec<u8>,
    /// Up to the caller's cap of stderr, with the rest drained and discarded.
    pub stderr: Vec<u8>,
}

/// Wait for `child`, capturing bounded stdout/stderr and killing it if it runs
/// past `timeout`.
///
/// The child's stdout and stderr must be piped before calling. Returns an error
/// only if waiting itself fails; a timeout is reported as `status == None`.
pub fn wait_captured(
    child: &mut Child,
    timeout: Duration,
    max_output: usize,
) -> io::Result<Captured> {
    let stdout = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(move || read_capped(pipe, max_output)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || read_capped(pipe, max_output)));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    };

    let stdout = stdout
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    Ok(Captured {
        status,
        stdout,
        stderr,
    })
}

/// Read at most `max` bytes, draining the rest so the writer never blocks.
pub fn read_capped<R: Read>(mut reader: R, max: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if kept.len() < max {
                    let take = (max - kept.len()).min(n);
                    kept.extend_from_slice(&chunk[..take]);
                }
            }
            Err(_) => break,
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn read_capped_bounds_and_drains_output() {
        let data = vec![b'x'; 100_000];
        let kept = read_capped(std::io::Cursor::new(data), 4_096);
        assert_eq!(kept.len(), 4_096);
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_child_is_killed_at_the_deadline() {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        let started = Instant::now();
        let captured = wait_captured(&mut child, Duration::from_millis(100), 4096).unwrap();
        assert!(captured.status.is_none(), "the child must have been killed");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waiting must not outlive the deadline"
        );
    }

    #[cfg(unix)]
    #[test]
    fn normal_output_is_captured() {
        let mut child = Command::new("sh")
            .args(["-c", "printf hello; printf oops >&2"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh");
        let captured = wait_captured(&mut child, Duration::from_secs(5), 4096).unwrap();
        assert!(captured.status.is_some_and(|status| status.success()));
        assert_eq!(String::from_utf8_lossy(&captured.stdout), "hello");
        assert_eq!(String::from_utf8_lossy(&captured.stderr), "oops");
    }
}
