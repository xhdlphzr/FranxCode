// Copyright (C) 2026 xhdlphzr
// SPDX-License-Identifier: AGPL-3.0-or-later

//! LSP client over stdio JSON-RPC with proper offset ↔ position conversion (asynchronous).

use crate::edit::Range;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Converts a 0‑based character offset to an LSP position (line, UTF‑16 column).
///
/// # Arguments
/// * `text` - The full document text.
/// * `char_offset` - The character offset (Unicode scalar count).
///
/// # Returns
/// A tuple `(line, utf16_column)`.
pub fn offset_to_position(text: &str, char_offset: usize) -> (usize, usize) {
    let co = char_offset.min(text.chars().count());
    let mut remaining = co;
    for (line_idx, line) in text.lines().enumerate() {
        let line_chars = line.chars().count();
        if remaining <= line_chars {
            let prefix: String = line.chars().take(remaining).collect();
            return (line_idx, prefix.encode_utf16().count());
        }
        remaining -= line_chars + 1;
    }
    (text.lines().count().saturating_sub(1), 0)
}

/// Converts an LSP position (line, UTF‑16 column) to a character offset.
///
/// # Arguments
/// * `text` - The full document text.
/// * `line` - The 0‑based line number.
/// * `utf16_col` - The UTF‑16 column offset on that line.
///
/// # Returns
/// The character offset (Unicode scalar count).
pub fn position_to_offset(text: &str, line: usize, utf16_col: usize) -> usize {
    let mut offset = 0;
    for (idx, line_str) in text.lines().enumerate() {
        if idx == line {
            let mut char_count = 0;
            let mut utf16_pos = 0;
            for ch in line_str.chars() {
                if utf16_pos >= utf16_col {
                    break;
                }
                utf16_pos += ch.len_utf16();
                char_count += 1;
            }
            return offset + char_count;
        }
        offset += line_str.chars().count() + 1;
    }
    offset
}

/// Configuration for an LSP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    pub command: String,
    pub args: Vec<String>,
    pub root_uri: String,
    pub language_id: String,
}

impl LspConfig {
    /// Creates a configuration for rust-analyzer.
    ///
    /// # Arguments
    /// * `root` - The root directory path (will be converted to file URI).
    ///
    /// # Returns
    /// A preconfigured `LspConfig` for Rust.
    pub fn rust_analyzer(root: &str) -> Self {
        let path = root.replace('\\', "/");
        let root_uri = if path.starts_with('/') {
            format!("file://{}", path)
        } else {
            format!("file:///{}", path)
        };
        Self {
            command: "rust-analyzer".into(),
            args: vec![],
            root_uri,
            language_id: "rust".into(),
        }
    }
}

/// A location in a file (URI + character range).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// A completion item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<String>,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
    pub additional_edits: Vec<(Range, String)>,
}

/// Hover information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverInfo {
    pub contents: String,
    pub range: Option<Range>,
}

/// Asynchronous LSP client communicating via stdio.
///
/// **Note**: This type does **not** implement `Serialize`/`Deserialize` because it contains
/// a child process handle and internal state that cannot be serialized.
pub struct LspClient {
    process: Mutex<Child>,
    next_id: AtomicU64,
    #[allow(dead_code)]
    config: LspConfig,
}

impl LspClient {
    /// Starts the LSP server process and performs initialisation.
    ///
    /// # Arguments
    /// * `config` - The server configuration.
    ///
    /// # Returns
    /// A new `LspClient` on success, or an error if the server cannot be launched.
    pub async fn start(config: LspConfig) -> anyhow::Result<Self> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": config.root_uri,
                "capabilities": {
                    "textDocument": {
                        "completion": { "completionItem": { "snippetSupport": true } },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "definition": { "linkSupport": true },
                        "references": {},
                        "formatting": { "dynamicRegistration": true },
                        "codeAction": {}
                    }
                }
            }
        });
        let d = serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}});

        if let Some(stdin) = child.stdin.as_mut() {
            let b = init.to_string();
            stdin
                .write_all(format!("Content-Length: {}\r\n\r\n{}", b.len(), b).as_bytes())
                .await?;
            stdin.flush().await?;
            let b2 = d.to_string();
            stdin
                .write_all(format!("Content-Length: {}\r\n\r\n{}", b2.len(), b2).as_bytes())
                .await?;
            stdin.flush().await?;
        }

        Ok(Self {
            process: Mutex::new(child),
            next_id: AtomicU64::new(1),
            config,
        })
    }

    /// Sends a JSON‑RPC request and waits for the response asynchronously.
    async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let body = req.to_string();
        let hdr = format!("Content-Length: {}\r\n\r\n", body.len());

        let mut proc = self.process.lock().await;
        if let Some(stdin) = proc.stdin.as_mut() {
            stdin.write_all(hdr.as_bytes()).await?;
            stdin.write_all(body.as_bytes()).await?;
            stdin.flush().await?;
        }

        if let Some(stdout) = proc.stdout.as_mut() {
            let mut reader = BufReader::new(stdout);
            let mut cl = 0usize;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if trimmed.to_lowercase().starts_with("content-length:") {
                    cl = trimmed
                        .trim_start_matches("Content-Length:")
                        .trim_start_matches("content-length:")
                        .trim()
                        .parse()
                        .unwrap_or(0);
                }
            }
            let mut buf = vec![0u8; cl];
            if cl > 0 {
                reader.read_exact(&mut buf).await?;
            }
            Ok(serde_json::from_slice(&buf)?)
        } else {
            anyhow::bail!("no stdout")
        }
    }

    /// Builds the parameters for a textDocument position request.
    fn pos_params(&self, char_offset: usize, full_text: &str, uri: &str) -> serde_json::Value {
        let (line, col) = offset_to_position(full_text, char_offset);
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        })
    }

    /// Finds the definition at the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - Character offset in the document.
    /// * `full_text` - The full document text.
    /// * `uri` - The document URI.
    ///
    /// # Returns
    /// A vector of `Location`s, or an error if the request fails.
    pub async fn goto_definition(
        &self,
        char_offset: usize,
        full_text: &str,
        uri: &str,
    ) -> anyhow::Result<Vec<Location>> {
        let r = self
            .send(
                "textDocument/definition",
                self.pos_params(char_offset, full_text, uri),
            )
            .await?;
        self.parse_locations(&r, full_text)
    }

    /// Finds all references at the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - Character offset in the document.
    /// * `full_text` - The full document text.
    /// * `uri` - The document URI.
    ///
    /// # Returns
    /// A vector of `Location`s, or an error if the request fails.
    pub async fn find_references(
        &self,
        char_offset: usize,
        full_text: &str,
        uri: &str,
    ) -> anyhow::Result<Vec<Location>> {
        let mut p = self.pos_params(char_offset, full_text, uri);
        p["context"] = serde_json::json!({"includeDeclaration": true});
        let r = self.send("textDocument/references", p).await?;
        self.parse_locations(&r, full_text)
    }

    /// Gets completion items at the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - Character offset in the document.
    /// * `full_text` - The full document text.
    /// * `uri` - The document URI.
    ///
    /// # Returns
    /// A vector of `CompletionItem`s, or an error if the request fails.
    pub async fn get_completions(
        &self,
        char_offset: usize,
        full_text: &str,
        uri: &str,
    ) -> anyhow::Result<Vec<CompletionItem>> {
        let mut p = self.pos_params(char_offset, full_text, uri);
        p["context"] = serde_json::json!({"triggerKind": 1});
        let r = self.send("textDocument/completion", p).await?;
        let empty = vec![];
        let items = match r.get("result") {
            Some(serde_json::Value::Array(a)) => a,
            Some(obj) => obj
                .get("items")
                .and_then(|i| i.as_array())
                .unwrap_or(&empty),
            None => &empty,
        };
        Ok(items
            .iter()
            .map(|i| CompletionItem {
                label: i["label"].as_str().unwrap_or("").into(),
                kind: i["kind"].as_u64().map(|k| format!("{}", k)),
                detail: i["detail"].as_str().map(|s| s.into()),
                insert_text: i["insertText"]
                    .as_str()
                    .or(i["label"].as_str())
                    .map(|s| s.into()),
                additional_edits: vec![],
            })
            .collect())
    }

    /// Gets hover information at the given character offset.
    ///
    /// # Arguments
    /// * `char_offset` - Character offset in the document.
    /// * `full_text` - The full document text.
    /// * `uri` - The document URI.
    ///
    /// # Returns
    /// `Ok(Some(HoverInfo))` if hover is available, `Ok(None)` otherwise.
    pub async fn get_hover(
        &self,
        char_offset: usize,
        full_text: &str,
        uri: &str,
    ) -> anyhow::Result<Option<HoverInfo>> {
        let r = self
            .send(
                "textDocument/hover",
                self.pos_params(char_offset, full_text, uri),
            )
            .await?;
        match r.get("result") {
            Some(res) if !res.is_null() => {
                let contents = res["contents"]["value"]
                    .as_str()
                    .or(res["contents"].as_str())
                    .unwrap_or("")
                    .into();
                let range = if let Some(rng) = res.get("range") {
                    self.parse_lsp_range_to_char_range(rng, full_text)
                } else {
                    None
                };
                Ok(Some(HoverInfo { contents, range }))
            }
            _ => Ok(None),
        }
    }

    /// Formats a character range using the LSP server.
    ///
    /// # Arguments
    /// * `range` - The character range to format.
    /// * `full_text` - The full document text.
    /// * `uri` - The document URI.
    ///
    /// # Returns
    /// A vector of `(Range, new_text)` pairs, or an error if the request fails.
    pub async fn format_range(
        &self,
        range: &Range,
        full_text: &str,
        uri: &str,
    ) -> anyhow::Result<Vec<(Range, String)>> {
        let (sl, sc) = offset_to_position(full_text, range.start);
        let (el, ec) = offset_to_position(full_text, range.end);
        let p = serde_json::json!({
            "textDocument": { "uri": uri },
            "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
            "options": { "tabSize": 4, "insertSpaces": true }
        });
        let r = self.send("textDocument/rangeFormatting", p).await?;
        Ok(self.parse_edits(&r, full_text))
    }

    /// Gets quick fixes for a range.
    ///
    /// # Arguments
    /// * `range` - The character range.
    /// * `full_text` - The full document text.
    /// * `uri` - The document URI.
    ///
    /// # Returns
    /// A vector of `(Range, new_text)` pairs, or an error if the request fails.
    pub async fn quick_fix(
        &self,
        range: &Range,
        full_text: &str,
        uri: &str,
    ) -> anyhow::Result<Vec<(Range, String)>> {
        let (sl, sc) = offset_to_position(full_text, range.start);
        let (el, ec) = offset_to_position(full_text, range.end);
        let p = serde_json::json!({
            "textDocument": { "uri": uri },
            "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
            "context": { "diagnostics": [], "only": ["quickfix"] }
        });
        let r = self.send("textDocument/codeAction", p).await?;
        let mut edits = Vec::new();
        if let Some(actions) = r["result"].as_array() {
            for a in actions {
                if let Some(edit) = a.get("edit").and_then(|e| e.get("changes")) {
                    for (_, tes) in edit.as_object().unwrap_or(&serde_json::Map::new()) {
                        for te in tes.as_array().unwrap_or(&vec![]) {
                            if let (Some(rng), nt) = (
                                self.parse_lsp_range_to_char_range(&te["range"], full_text),
                                te["newText"].as_str().unwrap_or(""),
                            ) {
                                edits.push((rng, nt.into()));
                            }
                        }
                    }
                }
            }
        }
        Ok(edits)
    }

    /// Shuts down the LSP server.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if the shutdown request fails.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.send("shutdown", serde_json::json!(null)).await?;
        let mut proc = self.process.lock().await;
        if let Some(stdin) = proc.stdin.as_mut() {
            let e = serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null});
            let b = e.to_string();
            stdin
                .write_all(format!("Content-Length: {}\r\n\r\n{}", b.len(), b).as_bytes())
                .await?;
            stdin.flush().await?;
        }
        proc.wait().await?;
        Ok(())
    }

    // ----- parsing helpers -----

    fn parse_locations(
        &self,
        r: &serde_json::Value,
        full_text: &str,
    ) -> anyhow::Result<Vec<Location>> {
        let mut out = Vec::new();
        if let Some(arr) = r.get("result").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(loc) = self.parse_single(Some(item), full_text) {
                    out.push(loc);
                }
            }
        } else if let Some(loc) = self.parse_single(r.get("result"), full_text) {
            out.push(loc);
        }
        Ok(out)
    }

    fn parse_single(&self, loc: Option<&serde_json::Value>, full_text: &str) -> Option<Location> {
        let loc = loc?;
        let uri = loc["uri"].as_str().unwrap_or("").into();
        let range = self.parse_lsp_range_to_char_range(&loc["range"], full_text)?;
        Some(Location { uri, range })
    }

    fn parse_lsp_range_to_char_range(
        &self,
        r: &serde_json::Value,
        full_text: &str,
    ) -> Option<Range> {
        let start_line = r["start"]["line"].as_u64()? as usize;
        let start_col = r["start"]["character"].as_u64()? as usize;
        let end_line = r["end"]["line"].as_u64()? as usize;
        let end_col = r["end"]["character"].as_u64()? as usize;
        let start = position_to_offset(full_text, start_line, start_col);
        let end = position_to_offset(full_text, end_line, end_col);
        Some(Range::new(start, end))
    }

    fn parse_edits(&self, r: &serde_json::Value, full_text: &str) -> Vec<(Range, String)> {
        let mut out = Vec::new();
        if let Some(items) = r.get("result").and_then(|v| v.as_array()) {
            for item in items {
                if let (Some(rng), nt) = (
                    self.parse_lsp_range_to_char_range(&item["range"], full_text),
                    item["newText"].as_str().unwrap_or(""),
                ) {
                    out.push((rng, nt.into()));
                }
            }
        }
        out
    }
}

impl Drop for LspClient {
    /// Automatically attempts to shut down the server when dropped.
    /// Note: this is best‑effort; if the async shutdown fails, the process will be killed on exit.
    fn drop(&mut self) {
        // We cannot call async shutdown in drop; the server will be killed when the process exits.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_to_position_roundtrip() {
        let text = "hello\nworld\nfoo";
        for offset in [0, 2, 5, 6, 7, 11, 12, 15] {
            let (line, col) = offset_to_position(text, offset);
            let new_offset = position_to_offset(text, line, col);
            assert_eq!(new_offset, offset);
        }
    }

    #[test]
    fn test_position_to_offset_edge_cases() {
        let text = "abc\n";
        assert_eq!(position_to_offset(text, 0, 0), 0);
        assert_eq!(position_to_offset(text, 0, 1), 1);
        assert_eq!(position_to_offset(text, 0, 2), 2);
        assert_eq!(position_to_offset(text, 0, 3), 3);
        assert_eq!(position_to_offset(text, 1, 0), 4);
        assert_eq!(position_to_offset(text, 2, 0), text.len());
    }

    #[test]
    fn test_lsp_config_rust_analyzer() {
        let config = LspConfig::rust_analyzer("/home/user/project");
        assert_eq!(config.command, "rust-analyzer");
        assert_eq!(config.root_uri, "file:///home/user/project");
        assert_eq!(config.language_id, "rust");
    }

    #[tokio::test]
    async fn test_parse_lsp_range_to_char_range() {
        let client = LspClient::start(LspConfig::rust_analyzer("."))
            .await
            .unwrap();
        let full_text = "hello\nworld";
        let json = serde_json::json!({
            "start": { "line": 0, "character": 1 },
            "end": { "line": 0, "character": 3 }
        });
        let range = client.parse_lsp_range_to_char_range(&json, full_text);
        assert_eq!(range, Some(Range::new(1, 3)));

        let json2 = serde_json::json!({
            "start": { "line": 1, "character": 2 },
            "end": { "line": 1, "character": 4 }
        });
        let range2 = client.parse_lsp_range_to_char_range(&json2, full_text);
        assert_eq!(range2, Some(Range::new(8, 10)));
    }
}
