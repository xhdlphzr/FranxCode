// Copyright (C) 2026 xhdlphzr

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.

// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! # FranxCode Editor Core
//! Built on [`editor_core`] (windoze/editor-core).
//! **Thread safety**: `Editor` is not `Send + Sync`. Wrap in `Arc<RwLock<Editor>>` for multi-threaded use.

pub mod cursor;
pub mod edit;
pub mod history;
pub mod lsp;
pub mod project;
pub mod syntax;

pub use cursor::{Cursor, CursorMode, Movement, Selection};
pub use edit::{Range, StoredCmd, TextEdit, UndoRedoError};
pub use history::{EditRecord, EditType, HistoryTimeline, Version};
pub use lsp::{offset_to_position, CompletionItem, HoverInfo, Location, LspClient, LspConfig};
pub use project::{FileNode, GitStatus, ProjectManager, StatusEntry};
pub use syntax::{HighlightStyle, Symbol, SymbolKind, SyntaxNode, SyntaxTree};

/// Main editor instance. Not `Send + Sync` — wrap in `Arc<RwLock<Editor>>` for multi-threaded use.
pub struct Editor {
    pub state: TextEdit,
    pub cursors: Cursor,
    pub syntax: Option<SyntaxTree>,
    pub lsp: Option<LspClient>,
    pub timeline: HistoryTimeline,
    pub project: ProjectManager,
}

impl Editor {
    /// Creates a new empty editor.
    ///
    /// # Returns
    /// A new `Editor` instance.
    pub fn new() -> Self {
        Self {
            state: TextEdit::new(),
            cursors: Cursor::new(),
            syntax: None,
            lsp: None,
            timeline: HistoryTimeline::new(),
            project: ProjectManager::new(),
        }
    }

    /// Creates an editor with initial content.
    ///
    /// # Arguments
    /// * `content` - The initial text content.
    ///
    /// # Returns
    /// A new `Editor` instance containing `content`.
    pub fn with_content(content: &str) -> Self {
        Self {
            state: TextEdit::from_str(content),
            cursors: Cursor::new(),
            syntax: None,
            lsp: None,
            timeline: HistoryTimeline::new(),
            project: ProjectManager::new(),
        }
    }

    /// Enables syntax highlighting for a given language.
    ///
    /// # Arguments
    /// * `language` - The programming language to parse.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if the language parser cannot be loaded.
    pub fn enable_syntax(&mut self, language: syntax::Language) -> anyhow::Result<()> {
        let text = self.state.full_text();
        self.syntax = Some(SyntaxTree::new(language, &text)?);
        Ok(())
    }

    /// Enables LSP client with the given configuration.
    ///
    /// # Arguments
    /// * `config` - The LSP server configuration.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if the server process cannot be started.
    pub async fn enable_lsp(&mut self, config: LspConfig) -> anyhow::Result<()> {
        self.lsp = Some(LspClient::start(config).await?);
        Ok(())
    }

    // Delegated text editing

    /// Inserts text at the current cursor position.
    ///
    /// # Arguments
    /// * `text` - The string to insert.
    pub fn insert(&mut self, text: &str) {
        let pos = self.cursors.position();
        self.state.insert(pos, text);
        self.timeline.record_edit(
            EditType::Insert,
            Range::collapsed(pos),
            text,
            "",
            &self.state.full_text(),
        );
        if let Some(ref mut syn) = self.syntax {
            syn.update(&self.state.full_text());
        }
        self.project.mark_dirty();
    }

    /// Deletes the currently selected text (or character under cursor if no selection).
    pub fn delete(&mut self) {
        let ranges = self.cursors.edit_ranges();
        for r in &ranges {
            let deleted = self.state.get_text(r);
            self.state.delete(r);
            self.timeline
                .record_edit(EditType::Delete, *r, "", &deleted, &self.state.full_text());
        }
        if let Some(ref mut syn) = self.syntax {
            syn.update(&self.state.full_text());
        }
        self.project.mark_dirty();
    }

    /// Replaces the current selection(s) with new text.
    ///
    /// # Arguments
    /// * `new_text` - The text to replace the selection with.
    pub fn replace(&mut self, new_text: &str) {
        let ranges = self.cursors.edit_ranges();
        for r in &ranges {
            let deleted = self.state.get_text(r);
            self.state.replace(r, new_text);
            self.timeline.record_edit(
                EditType::Replace,
                *r,
                new_text,
                &deleted,
                &self.state.full_text(),
            );
        }
        if let Some(ref mut syn) = self.syntax {
            syn.update(&self.state.full_text());
        }
        self.project.mark_dirty();
    }

    /// Undoes the last editing operation.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if there is nothing to undo.
    pub fn undo(&mut self) -> anyhow::Result<()> {
        self.state.undo()?;
        self.timeline.record_edit(
            EditType::Undo,
            Range::collapsed(0),
            "",
            "",
            &self.state.full_text(),
        );
        if let Some(ref mut syn) = self.syntax {
            syn.update(&self.state.full_text());
        }
        Ok(())
    }

    /// Redoes the last undone editing operation.
    ///
    /// # Returns
    /// `Ok(())` on success, or an error if there is nothing to redo.
    pub fn redo(&mut self) -> anyhow::Result<()> {
        self.state.redo()?;
        self.timeline.record_edit(
            EditType::Redo,
            Range::collapsed(0),
            "",
            "",
            &self.state.full_text(),
        );
        if let Some(ref mut syn) = self.syntax {
            syn.update(&self.state.full_text());
        }
        Ok(())
    }

    // Delegated cursor

    /// Moves the primary cursor left.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_left(&mut self, extend: bool) {
        self.cursors.move_left(&self.state, extend);
    }

    /// Moves the primary cursor right.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_right(&mut self, extend: bool) {
        self.cursors.move_right(&self.state, extend);
    }

    /// Moves the primary cursor up.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_up(&mut self, extend: bool) {
        self.cursors.move_up(&self.state, extend);
    }

    /// Moves the primary cursor down.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_down(&mut self, extend: bool) {
        self.cursors.move_down(&self.state, extend);
    }

    /// Moves the primary cursor one word left.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_word_left(&mut self, extend: bool) {
        self.cursors.move_word_left(&self.state, extend);
    }

    /// Moves the primary cursor one word right.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_word_right(&mut self, extend: bool) {
        self.cursors.move_word_right(&self.state, extend);
    }

    /// Moves the primary cursor to the start of the current line.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_to_line_start(&mut self, extend: bool) {
        self.cursors.move_to_line_start(&self.state, extend);
    }

    /// Moves the primary cursor to the end of the current line.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_to_line_end(&mut self, extend: bool) {
        self.cursors.move_to_line_end(&self.state, extend);
    }

    /// Moves the primary cursor to the start of the file.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_to_file_start(&mut self, extend: bool) {
        self.cursors.move_to_file_start(&self.state, extend);
    }

    /// Moves the primary cursor to the end of the file.
    ///
    /// # Arguments
    /// * `extend` - If `true`, extends the selection; otherwise collapses.
    pub fn move_to_file_end(&mut self, extend: bool) {
        self.cursors.move_to_file_end(&self.state, extend);
    }

    /// Selects the entire document.
    pub fn select_all(&mut self) {
        self.cursors.select_all(self.state.len());
    }

    /// Selects the line containing the primary cursor.
    pub fn select_line(&mut self) {
        self.cursors.select_line(&self.state);
    }

    /// Adds a new cursor above the primary cursor.
    pub fn add_cursor_above(&mut self) {
        self.cursors.add_cursor_above(&self.state);
    }

    /// Adds a new cursor below the primary cursor.
    pub fn add_cursor_below(&mut self) {
        self.cursors.add_cursor_below(&self.state);
    }

    /// Splits the current selection into per‑line carets.
    pub fn split_selection_into_lines(&mut self) {
        self.cursors.split_selection_into_lines(&self.state);
    }

    // Convenience

    /// Returns the full document text.
    ///
    /// # Returns
    /// A `String` containing the entire document.
    pub fn full_text(&self) -> String {
        self.state.full_text()
    }

    /// Returns the length of the document in characters.
    ///
    /// # Returns
    /// The number of characters.
    pub fn len(&self) -> usize {
        self.state.len()
    }

    /// Returns `true` if the document is empty.
    ///
    /// # Returns
    /// `true` if `len() == 0`, otherwise `false`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the current primary cursor position.
    ///
    /// # Returns
    /// The character offset of the active cursor.
    pub fn cursor_position(&self) -> usize {
        self.cursors.position()
    }

    // LSP proxying

    /// Returns the `file://` URI of the currently open file, if any.
    fn document_uri(&self) -> Option<String> {
        self.project.current_file.as_ref().map(|path| {
            let path_str = path.to_str().expect("non-UTF-8 path");
            format!("file://{}", path_str)
        })
    }

    /// Finds the definition at the current cursor position.
    pub async fn goto_definition(&self) -> anyhow::Result<Vec<Location>> {
        if let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.document_uri()) {
            lsp.goto_definition(self.cursor_position(), &self.full_text(), &uri)
                .await
        } else {
            Ok(vec![])
        }
    }

    /// Finds all references at the current cursor position.
    pub async fn find_references(&self) -> anyhow::Result<Vec<Location>> {
        if let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.document_uri()) {
            lsp.find_references(self.cursor_position(), &self.full_text(), &uri)
                .await
        } else {
            Ok(vec![])
        }
    }

    /// Gets completion items at the current cursor position.
    pub async fn get_completions(&self) -> anyhow::Result<Vec<CompletionItem>> {
        if let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.document_uri()) {
            lsp.get_completions(self.cursor_position(), &self.full_text(), &uri)
                .await
        } else {
            Ok(vec![])
        }
    }

    /// Gets hover information at the current cursor position.
    pub async fn get_hover(&self) -> anyhow::Result<Option<HoverInfo>> {
        if let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.document_uri()) {
            lsp.get_hover(self.cursor_position(), &self.full_text(), &uri)
                .await
        } else {
            Ok(None)
        }
    }

    /// Formats the given character range using the LSP server.
    pub async fn format_range(&self, range: &Range) -> anyhow::Result<Vec<(Range, String)>> {
        if let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.document_uri()) {
            lsp.format_range(range, &self.full_text(), &uri).await
        } else {
            Ok(vec![])
        }
    }

    /// Gets quick fixes for a given character range.
    pub async fn quick_fix(&self, range: &Range) -> anyhow::Result<Vec<(Range, String)>> {
        if let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.document_uri()) {
            lsp.quick_fix(range, &self.full_text(), &uri).await
        } else {
            Ok(vec![])
        }
    }
}

impl Default for Editor {
    /// Creates a default empty editor using `new()`.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let e = Editor::new();
        assert!(e.is_empty());
    }

    #[test]
    fn test_content() {
        let e = Editor::with_content("hi");
        assert_eq!(e.full_text(), "hi");
    }

    #[test]
    fn test_insert() {
        let mut e = Editor::with_content("hello");
        e.cursors = Cursor::at(5);
        e.insert(" world");
        assert_eq!(e.full_text(), "hello world");
    }

    #[test]
    fn test_undo_redo() {
        let mut e = Editor::with_content("hello");
        e.cursors = Cursor::at(5);
        e.insert(" world");
        assert!(e.undo().is_ok());
        assert_eq!(e.full_text(), "hello");
        assert!(e.redo().is_ok());
        assert_eq!(e.full_text(), "hello world");
    }

    #[test]
    fn test_undo_error() {
        let mut e = Editor::with_content("hello");
        let err = e.undo().unwrap_err();
        assert_eq!(
            err.downcast_ref::<UndoRedoError>(),
            Some(&UndoRedoError::NothingToUndo)
        );
    }
}
