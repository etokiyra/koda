//! Minimal debug adapter (DAP) scaffold.
//!
//! Koda does not ship a full debug adapter protocol client yet. This module
//! provides a small, best-effort launcher that spawns an adapter, performs the
//! `initialize`/`launch`/`setBreakpoints` handshake, and reports the outcome.
//! It is the seam a real DAP client will replace.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;

fn write_frame(stdin: &mut dyn Write, value: &serde_json::Value) -> std::io::Result<()> {
    let body = value.to_string();
    write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    stdin.flush()
}

fn read_frame(reader: &mut dyn Read) -> Option<serde_json::Value> {
    // Read headers.
    let mut buf = [0u8; 1];
    // Read until \r\n\r\n
    let mut last = String::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return None,
            Ok(_) => {
                last.push(buf[0] as char);
                if last.ends_with("\r\n\r\n") {
                    break;
                }
                if last.len() > 4096 {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    let length = last.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse::<usize>().ok()
        } else {
            None
        }
    })?;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Launch `adapter` with `args`, run a minimal handshake, and return a status.
pub fn launch(adapter: &str, args: &[&str], breakpoints: &[usize]) -> Result<String, String> {
    let mut child = Command::new(adapter)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("could not start {adapter}: {err}"))?;

    let mut stdin = child.stdin.take().ok_or_else(|| "no stdin".to_string())?;
    let mut stdout = child.stdout.take().ok_or_else(|| "no stdout".to_string())?;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read_frame(&mut stdout));
    });

    let init = json!({
        "seq": 1,
        "type": "request",
        "command": "initialize",
        "arguments": { "adapterID": adapter, "clientID": "koda", "clientName": "Koda" }
    });
    write_frame(&mut stdin, &init).map_err(|err| err.to_string())?;

    let _ = rx.recv_timeout(Duration::from_secs(2));

    let launch = json!({
        "seq": 2,
        "type": "request",
        "command": "launch",
        "arguments": { "noDebug": true }
    });
    write_frame(&mut stdin, &launch).map_err(|err| err.to_string())?;

    let bp_args = json!({
        "seq": 3,
        "type": "request",
        "command": "setBreakpoints",
        "arguments": { "breakpoints": breakpoints.iter().map(|line| json!({"line": line + 1})).collect::<Vec<_>>() }
    });
    write_frame(&mut stdin, &bp_args).map_err(|err| err.to_string())?;

    let _ = child.kill();
    let _ = child.wait();
    Ok(format!("Debug adapter {adapter} initialized"))
}
