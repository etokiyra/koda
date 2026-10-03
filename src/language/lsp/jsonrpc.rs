//! JSON-RPC framing for the Language Server Protocol.
//!
//! LSP speaks JSON-RPC 2.0 over a byte stream, with each message prefixed by a
//! `Content-Length` header. This module is pure I/O with no process or thread
//! concerns, so the tricky part — framing and decoding — is easy to test.

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// A JSON-RPC error object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// A decoded JSON-RPC message.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A reply to a request. `result` and `error` are mutually exclusive.
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<RpcError>,
    },
    /// A one-way notification (no `id`), such as `publishDiagnostics`.
    Notification { method: String, params: Value },
    /// A request from the server that the client must answer.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// Write one `Content-Length` framed message.
pub fn write_message<W: Write>(writer: &mut W, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

/// Read one framed message, or `None` at end of stream.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }

    let Some(length) = length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };
    // Guard against a corrupt header allocating wildly.
    if length > 64 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }

    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let value: Value = serde_json::from_slice(&body).map_err(io::Error::other)?;
    Ok(Some(value))
}

/// Classify a raw JSON value into a [`Message`].
pub fn decode(value: Value) -> Option<Message> {
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        let method = method.to_string();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        return Some(match value.get("id") {
            Some(id) => Message::Request {
                id: id.clone(),
                method,
                params,
            },
            None => Message::Notification { method, params },
        });
    }

    let id = value.get("id")?.clone();
    let error = value.get("error").and_then(|error| {
        Some(RpcError {
            code: error.get("code")?.as_i64()?,
            message: error.get("message")?.as_str()?.to_string(),
        })
    });
    Some(Message::Response {
        id,
        result: value.get("result").cloned(),
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trips_a_message() {
        let message = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let mut buffer = Vec::new();
        write_message(&mut buffer, &message).unwrap();

        let mut reader = Cursor::new(buffer);
        let decoded = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn reads_consecutive_messages() {
        let first = serde_json::json!({"jsonrpc": "2.0", "method": "a"});
        let second = serde_json::json!({"jsonrpc": "2.0", "method": "b", "params": [1, 2]});
        let mut buffer = Vec::new();
        write_message(&mut buffer, &first).unwrap();
        write_message(&mut buffer, &second).unwrap();

        let mut reader = Cursor::new(buffer);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), first);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), second);
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn decodes_response_notification_and_request() {
        let response = decode(serde_json::json!({"id": 3, "result": {"ok": true}})).unwrap();
        assert!(matches!(response, Message::Response { id, .. } if id == 3));

        let notification =
            decode(serde_json::json!({"method": "textDocument/publishDiagnostics", "params": {}}))
                .unwrap();
        assert!(
            matches!(notification, Message::Notification { method, .. } if method == "textDocument/publishDiagnostics")
        );

        let request =
            decode(serde_json::json!({"id": 7, "method": "workspace/configuration"})).unwrap();
        assert!(matches!(request, Message::Request { id, .. } if id == 7));
    }

    #[test]
    fn decodes_error_responses() {
        let value = serde_json::json!({
            "id": 2,
            "error": {"code": -32601, "message": "method not found"}
        });
        match decode(value).unwrap() {
            Message::Response { error, .. } => {
                let error = error.expect("error");
                assert_eq!(error.code, -32601);
                assert_eq!(error.message, "method not found");
            }
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_bad_length_header() {
        // A present-but-unparseable Content-Length is malformed.
        let mut reader = Cursor::new(b"Content-Length: nope\r\n\r\n".to_vec());
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn headerless_line_is_end_of_stream() {
        // Not a framed message, and no header follows; treat as EOF.
        let mut reader = Cursor::new(b"{\"id\":1}\n".to_vec());
        assert!(read_message(&mut reader).unwrap().is_none());
    }
}
