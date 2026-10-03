//! Convert LSP payloads into Koda's own types.
//!
//! Keeping conversions here means the UI never sees a `serde_json::Value`, and
//! the same popups and pickers serve both language-server results and the
//! built-in heuristics.

use std::path::PathBuf;

use serde_json::Value;

use crate::language::completion::{Completion, CompletionKind};
use crate::language::hover::Hover;

use super::uri_to_path;

/// A jump target from a definition or references response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

/// Completion items from a `textDocument/completion` result.
///
/// The result may be a bare array or a `CompletionList` object.
pub fn completions(value: &Value) -> Vec<Completion> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let label = item.get("label")?.as_str()?;
            if label.is_empty() {
                return None;
            }
            let kind = completion_kind(item.get("kind").and_then(Value::as_u64));
            Some(Completion::new(label, kind))
        })
        .collect()
}

fn completion_kind(kind: Option<u64>) -> CompletionKind {
    match kind {
        Some(2..=4) => CompletionKind::Function,
        Some(5 | 6 | 10 | 12 | 20) => CompletionKind::Variable,
        Some(7 | 8 | 13 | 22 | 25) => CompletionKind::Type,
        Some(9 | 19) => CompletionKind::Text,
        Some(14) => CompletionKind::Keyword,
        Some(21) => CompletionKind::Constant,
        _ => CompletionKind::Text,
    }
}

/// Hover text from a `textDocument/hover` result.
pub fn hover(value: &Value) -> Option<Hover> {
    let text = markup_text(value.get("contents")?);
    // Drop markdown code fences, which read poorly in a plain popup.
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.trim_start().starts_with("```"))
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let title = lines.remove(0);
    Some(Hover::new(title, None, lines))
}

/// Extract text from any of LSP's hover `contents` shapes.
fn markup_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .map(markup_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
    }
    value
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Jump targets from a definition or references result.
///
/// Handles `Location`, `Location[]` and `LocationLink[]`.
pub fn locations(value: &Value) -> Vec<Location> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    };
    items.iter().filter_map(|item| location(item)).collect()
}

fn location(item: &Value) -> Option<Location> {
    let uri = item
        .get("uri")
        .or_else(|| item.get("targetUri"))?
        .as_str()?;
    let path = uri_to_path(uri)?;
    let range = item
        .get("range")
        .or_else(|| item.get("targetSelectionRange"))
        .or_else(|| item.get("targetRange"))?;
    let start = range.get("start")?;
    Some(Location {
        path,
        line: start.get("line")?.as_u64()? as usize,
        col: start.get("character")?.as_u64()? as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completes_from_a_list_or_array() {
        let list = json!({ "items": [
            { "label": "println!", "kind": 3 },
            { "label": "String", "kind": 7 },
            { "label": "let", "kind": 14 }
        ]});
        let items = completions(&list);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "println!");
        assert_eq!(items[0].kind, CompletionKind::Function);
        assert_eq!(items[1].kind, CompletionKind::Type);
        assert_eq!(items[2].kind, CompletionKind::Keyword);

        let array = json!([{ "label": "x", "kind": 6 }]);
        assert_eq!(completions(&array)[0].kind, CompletionKind::Variable);
    }

    #[test]
    fn hovers_markdown_content() {
        let value = json!({
            "contents": { "kind": "markdown", "value": "```rust\nfn main()\n```\nprints hello" }
        });
        let hover = hover(&value).expect("hover");
        assert_eq!(hover.title, "fn main()");
        assert_eq!(hover.body, vec!["prints hello".to_string()]);
    }

    #[test]
    fn hovers_string_and_array_contents() {
        assert_eq!(
            hover(&json!({ "contents": "plain" })).unwrap().title,
            "plain"
        );
        let array = json!({ "contents": ["first", { "language": "rust", "value": "second" }] });
        let hover = hover(&array).unwrap();
        assert_eq!(hover.title, "first");
        assert_eq!(hover.body, vec!["second".to_string()]);
    }

    #[test]
    fn converts_locations_and_links() {
        let array = json!([
            { "uri": "file:///tmp/a.rs", "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 7 } } },
            { "targetUri": "file:///tmp/b.rs", "targetSelectionRange": { "start": { "line": 9, "character": 0 }, "end": { "line": 9, "character": 3 } } }
        ]);
        let found = locations(&array);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, PathBuf::from("/tmp/a.rs"));
        assert_eq!((found[0].line, found[0].col), (2, 4));
        assert_eq!(found[1].path, PathBuf::from("/tmp/b.rs"));
        assert_eq!((found[1].line, found[1].col), (9, 0));

        assert!(locations(&json!(null)).is_empty());
    }
}
