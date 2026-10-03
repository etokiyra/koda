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

/// A symbol from a `workspace/symbol` response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSymbolItem {
    pub name: String,
    pub kind: crate::language::symbols::SymbolKind,
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

/// A single text replacement within a file, in `(line, character)` coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub new_text: String,
}

/// Every edit a workspace edit makes to one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    pub path: PathBuf,
    pub edits: Vec<TextEdit>,
}

/// A command to run on the server.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRef {
    pub command: String,
    pub arguments: Value,
}

/// A quick fix or refactor offered by the server.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeAction {
    pub title: String,
    /// A workspace edit to apply, if the action carries one.
    pub edit: Option<Value>,
    /// A command to run, if the action carries one.
    pub command: Option<CommandRef>,
}

/// Parse a `textDocument/codeAction` result, which mixes `CodeAction` and
/// `Command` shapes.
pub fn code_actions(value: &Value) -> Vec<CodeAction> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?.to_string();
            let edit = item.get("edit").cloned();
            let command = match item.get("command") {
                // A bare `Command`: the command is a string with sibling args.
                Some(command) if command.is_string() => Some(CommandRef {
                    command: command.as_str()?.to_string(),
                    arguments: item.get("arguments").cloned().unwrap_or(Value::Null),
                }),
                // A `CodeAction`'s embedded `Command` object.
                Some(command) => parse_command(command),
                None => None,
            };
            Some(CodeAction {
                title,
                edit,
                command,
            })
        })
        .collect()
}

fn parse_command(value: &Value) -> Option<CommandRef> {
    Some(CommandRef {
        command: value.get("command")?.as_str()?.to_string(),
        arguments: value.get("arguments").cloned().unwrap_or(Value::Null),
    })
}

/// File edits from a `WorkspaceEdit` (`changes` or `documentChanges`).
pub fn workspace_edit(value: &Value) -> Vec<FileEdit> {
    let mut files = Vec::new();

    // `documentChanges` supersedes `changes` when both are present (LSP 3.16).
    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            // Skip create/rename/delete operations; we only apply edits.
            let Some(uri) = change.pointer("/textDocument/uri").and_then(Value::as_str) else {
                continue;
            };
            let edits = change.get("edits").cloned().unwrap_or(Value::Null);
            if let Some(file) = file_edit(uri, &edits) {
                files.push(file);
            }
        }
    } else if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            if let Some(file) = file_edit(uri, edits) {
                files.push(file);
            }
        }
    }

    files
}

fn file_edit(uri: &str, edits: &Value) -> Option<FileEdit> {
    let path = uri_to_path(uri)?;
    let edits = parse_edits(edits)?;
    if edits.is_empty() {
        return None;
    }
    Some(FileEdit { path, edits })
}

fn parse_edits(value: &Value) -> Option<Vec<TextEdit>> {
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| {
                let range = item.get("range")?;
                let start = range.get("start")?;
                let end = range.get("end")?;
                Some(TextEdit {
                    start: (
                        start.get("line")?.as_u64()? as usize,
                        start.get("character")?.as_u64()? as usize,
                    ),
                    end: (
                        end.get("line")?.as_u64()? as usize,
                        end.get("character")?.as_u64()? as usize,
                    ),
                    new_text: item.get("newText")?.as_str()?.to_string(),
                })
            })
            .collect(),
    )
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

/// Workspace symbols from a `workspace/symbol` result.
///
/// Handles both `SymbolInformation` (a `location` with a range) and the newer
/// `WorkspaceSymbol` shape. Symbols without a resolvable location are dropped.
pub fn workspace_symbols(value: &Value) -> Vec<WorkspaceSymbolItem> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            if name.is_empty() {
                return None;
            }
            let location = item.get("location")?;
            let uri = location
                .get("uri")
                .or_else(|| location.get("targetUri"))?
                .as_str()?;
            let path = uri_to_path(uri)?;
            let start = location
                .get("range")
                .or_else(|| location.get("targetSelectionRange"))
                .or_else(|| location.get("targetRange"))
                .and_then(|range| range.get("start"));
            Some(WorkspaceSymbolItem {
                name: name.to_string(),
                kind: symbol_kind(item.get("kind").and_then(Value::as_u64)),
                path,
                line: start
                    .and_then(|start| start.get("line"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize,
                col: start
                    .and_then(|start| start.get("character"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize,
            })
        })
        .collect()
}

/// Map an LSP `SymbolKind` number onto Koda's own symbol kinds.
fn symbol_kind(kind: Option<u64>) -> crate::language::symbols::SymbolKind {
    use crate::language::symbols::SymbolKind;
    match kind {
        Some(2..=4) => SymbolKind::Module,
        Some(5 | 26) => SymbolKind::Type,
        Some(6) => SymbolKind::Method,
        Some(7 | 8 | 13) => SymbolKind::Variable,
        Some(10) => SymbolKind::Enum,
        Some(11) => SymbolKind::Interface,
        Some(12) => SymbolKind::Function,
        Some(14 | 22) => SymbolKind::Constant,
        Some(20) => SymbolKind::Key,
        Some(23) => SymbolKind::Struct,
        _ => SymbolKind::Variable,
    }
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

/// Parse the edits from a `textDocument/formatting` result.
pub fn formatting_edits(value: &Value) -> Vec<TextEdit> {
    parse_edits(value).unwrap_or_default()
}

/// One overload from a `textDocument/signatureHelp` response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// The full callable label, e.g. `fn add(a: i32, b: i32) -> i32`.
    pub label: String,
    /// The parameter labels, sliced out of `label` when the server sends
    /// offsets instead of strings.
    pub parameters: Vec<String>,
    /// The signature's documentation, flattened to plain lines.
    pub documentation: Option<String>,
}

/// The active parameter hints for a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureHelp {
    pub signatures: Vec<Signature>,
    /// Index of the signature to show.
    pub active: usize,
    /// Index of the parameter the cursor is on.
    pub parameter: usize,
}

/// Parse a `textDocument/signatureHelp` result.
pub fn signature_help(value: &Value) -> Option<SignatureHelp> {
    let signatures = value.get("signatures")?.as_array()?;
    let active = value
        .get("activeSignature")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut items = Vec::new();
    for signature in signatures {
        let label = signature.get("label")?.as_str()?.to_string();
        let parameters = signature
            .get("parameters")
            .and_then(Value::as_array)
            .map(|parameters| {
                parameters
                    .iter()
                    .filter_map(|parameter| parameter_label(parameter, &label))
                    .collect()
            })
            .unwrap_or_default();
        let documentation = signature
            .get("documentation")
            .map(markup_text)
            .filter(|text| !text.trim().is_empty());
        items.push(Signature {
            label,
            parameters,
            documentation,
        });
    }
    if items.is_empty() {
        return None;
    }
    let active = active.min(items.len() - 1);
    // `activeParameter` may sit on the help object or on the active signature.
    let parameter = value
        .get("activeParameter")
        .or_else(|| signatures.get(active)?.get("activeParameter"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    Some(SignatureHelp {
        signatures: items,
        active,
        parameter,
    })
}

/// A parameter label, either a string or `[start, end]` offsets into `label`.
fn parameter_label(parameter: &Value, label: &str) -> Option<String> {
    match parameter.get("label")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(offsets) => {
            let start = offsets.first()?.as_u64()? as usize;
            let end = offsets.get(1)?.as_u64()? as usize;
            let chars: Vec<char> = label.chars().collect();
            (start < end && end <= chars.len())
                .then(|| chars[start..end].iter().collect::<String>())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_formatting_edits() {
        let value = json!([
            { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }, "newText": "let" }
        ]);
        let edits = formatting_edits(&value);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].start, (0, 0));
        assert_eq!(edits[0].new_text, "let");
        assert!(formatting_edits(&json!(null)).is_empty());
    }

    #[test]
    fn parses_signature_help_with_offset_parameters() {
        let value = json!({
            "signatures": [
                {
                    "label": "fn add(a: i32, b: i32)",
                    "parameters": [ { "label": [7, 13] }, { "label": [15, 21] } ],
                    "documentation": "Adds two numbers"
                },
                { "label": "fn add(a: f64, b: f64)" }
            ],
            "activeSignature": 1,
            "activeParameter": 0
        });
        let help = signature_help(&value).expect("help");
        assert_eq!(help.active, 1);
        assert_eq!(help.signatures.len(), 2);
        assert_eq!(help.signatures[0].parameters, vec!["a: i32", "b: i32"]);
        assert_eq!(
            help.signatures[0].documentation.as_deref(),
            Some("Adds two numbers")
        );
        assert_eq!(help.parameter, 0);

        assert!(signature_help(&json!({})).is_none());
        assert!(signature_help(&json!({ "signatures": [] })).is_none());
    }

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

    #[test]
    fn parses_workspace_symbols() {
        use crate::language::symbols::SymbolKind;
        let value = json!([
            { "name": "Server", "kind": 23, "location": {
                "uri": "file:///tmp/a.rs",
                "range": { "start": { "line": 3, "character": 4 }, "end": { "line": 3, "character": 10 } }
            }},
            { "name": "run", "kind": 12, "location": {
                "uri": "file:///tmp/b.rs",
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 3 } }
            }},
            { "name": "NoLocation" }
        ]);
        let items = workspace_symbols(&value);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "Server");
        assert_eq!(items[0].kind, SymbolKind::Struct);
        assert_eq!(items[0].path, PathBuf::from("/tmp/a.rs"));
        assert_eq!((items[0].line, items[0].col), (3, 4));
        assert_eq!(items[1].kind, SymbolKind::Function);

        assert!(workspace_symbols(&json!(null)).is_empty());
    }

    #[test]
    fn converts_a_workspace_edit() {
        let value = json!({
            "changes": {
                "file:///tmp/a.rs": [
                    { "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 9 } }, "newText": "renamed" }
                ]
            }
        });
        let files = workspace_edit(&value);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("/tmp/a.rs"));
        assert_eq!(files[0].edits[0].start, (1, 4));
        assert_eq!(files[0].edits[0].new_text, "renamed");

        let document_changes = json!({
            "documentChanges": [
                { "textDocument": { "uri": "file:///tmp/b.rs" },
                  "edits": [ { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "newText": "x" } ] }
            ]
        });
        let files = workspace_edit(&document_changes);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("/tmp/b.rs"));
    }

    #[test]
    fn prefers_document_changes_when_both_are_present() {
        let value = json!({
            "changes": {
                "file:///tmp/stale.rs": [
                    { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "newText": "stale" }
                ]
            },
            "documentChanges": [
                { "textDocument": { "uri": "file:///tmp/fresh.rs" },
                  "edits": [ { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "newText": "fresh" } ] }
            ]
        });
        let files = workspace_edit(&value);
        assert_eq!(files.len(), 1, "documentChanges supersedes changes");
        assert_eq!(files[0].path, PathBuf::from("/tmp/fresh.rs"));
        assert_eq!(files[0].edits[0].new_text, "fresh");
    }

    #[test]
    fn parses_code_actions_and_commands() {
        let value = json!([
            { "title": "Add `mut`", "kind": "quickfix",
              "edit": { "changes": { "file:///tmp/a.rs": [
                  { "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }, "newText": "mut " }
              ] } } },
            { "title": "Import trait", "command": { "command": "rust-analyzer.applySourceChange", "arguments": [] } },
            { "title": "Bare command", "command": "do.thing", "arguments": [1] }
        ]);
        let actions = code_actions(&value);
        assert_eq!(actions.len(), 3);
        assert!(actions[0].edit.is_some());
        assert_eq!(
            actions[1].command.as_ref().unwrap().command,
            "rust-analyzer.applySourceChange"
        );
        assert_eq!(actions[2].command.as_ref().unwrap().command, "do.thing");
        assert_eq!(actions[2].command.as_ref().unwrap().arguments, json!([1]));
    }
}
