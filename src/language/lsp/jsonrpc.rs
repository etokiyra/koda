//! JSON-RPC framing for the Language Server Protocol.
//!
//! LSP speaks JSON-RPC 2.0 over a byte stream, with each message prefixed by a
//! `Content-Length` header. This module is pure I/O with no process or thread
//! concerns, so the tricky part — framing and decoding — is easy to test.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// The most header bytes Koda reads before a message body. A corrupt or
/// non-LSP stream must not be able to grow a header line without bound.
const MAX_HEADER_BYTES: usize = 16 * 1024;

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
    let mut header_bytes = 0usize;
    loop {
        let mut line = Vec::new();
        // Bound each header line so a stream of garbage cannot allocate
        // without bound before the body cap applies.
        let read = reader
            .by_ref()
            .take(MAX_HEADER_BYTES as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(None);
        }
        header_bytes += read;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "message header too large",
            ));
        }
        let text = String::from_utf8_lossy(&line);
        let header = text.trim_end_matches(['\r', '\n']);
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

/// A JSON-RPC id as an integer, accepting the numeric strings a non-conformant
/// server might echo back.
pub fn id_as_i64(id: &Value) -> Option<i64> {
    id.as_i64()
        .or_else(|| id.as_str().and_then(|text| text.parse::<i64>().ok()))
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
    fn a_bad_json_body_can_be_skipped() {
        // A body that is not JSON consumes exactly its framed length, so the
        // stream stays in sync and the next message is still readable.
        let mut stream = Vec::new();
        let bad = b"definitely not json";
        stream.extend_from_slice(format!("Content-Length: {}\r\n\r\n", bad.len()).as_bytes());
        stream.extend_from_slice(bad);
        let good = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": true});
        let good_body = serde_json::to_vec(&good).unwrap();
        stream.extend_from_slice(format!("Content-Length: {}\r\n\r\n", good_body.len()).as_bytes());
        stream.extend_from_slice(&good_body);

        let mut reader = Cursor::new(stream);
        let err = read_message(&mut reader).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), good);
    }

    #[test]
    fn headerless_line_is_end_of_stream() {
        // Not a framed message, and no header follows; treat as EOF.
        let mut reader = Cursor::new(b"{\"id\":1}\n".to_vec());
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn rejects_an_oversized_header() {
        // A stream of non-protocol bytes must not grow a header without bound.
        let mut data = vec![b'x'; MAX_HEADER_BYTES + 10];
        data.push(b'\n');
        let mut reader = Cursor::new(data);
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn reads_integer_and_numeric_string_ids() {
        assert_eq!(id_as_i64(&serde_json::json!(3)), Some(3));
        assert_eq!(id_as_i64(&serde_json::json!("3")), Some(3));
        assert_eq!(id_as_i64(&serde_json::json!("x")), None);
        assert_eq!(id_as_i64(&serde_json::json!(null)), None);
    }
}
