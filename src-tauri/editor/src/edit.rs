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

//! Text editing operations backed by [`editor_core::PieceTable`].

use editor_core::PieceTable;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use thiserror::Error;

/// Character range (start inclusive, end exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    /// Creates a new range with the given start and end positions.
    ///
    /// # Arguments
    /// * `start` - Start offset (inclusive).
    /// * `end` - End offset (exclusive).
    ///
    /// # Panics
    /// Panics if `start > end` in debug builds.
    pub fn new(start: usize, end: usize) -> Self {
        assert!(start <= end);
        Self { start, end }
    }

    /// Creates a collapsed range (zero length) at the given offset.
    ///
    /// # Arguments
    /// * `offset` - The character offset.
    ///
    /// # Returns
    /// A `Range` with `start == end == offset`.
    pub fn collapsed(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Returns the length of the range (number of characters).
    ///
    /// # Returns
    /// `end - start`, or 0 if `end < start`.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` if the range has zero length.
    ///
    /// # Returns
    /// `true` when `start == end`.
    pub fn is_collapsed(&self) -> bool {
        self.start == self.end
    }

    /// Checks whether a character offset lies inside the range.
    ///
    /// # Arguments
    /// * `offset` - The character offset to test.
    ///
    /// # Returns
    /// `true` if `start <= offset < end`.
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }
}

/// A stored undo/redo command containing the affected range and the changed text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCmd {
    pub range: Range,
    pub inserted: String,
    pub deleted: String,
}

/// Manages the undo/redo stacks using a deque for efficient pop_front.
#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    undo: VecDeque<StoredCmd>,
    redo: VecDeque<StoredCmd>,
    max: usize,
}

impl UndoStack {
    /// Creates a new empty undo stack with a maximum of 1000 entries.
    pub fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            max: 1000,
        }
    }

    /// Pushes a new command onto the undo stack and clears the redo stack.
    ///
    /// # Arguments
    /// * `range` - The affected character range.
    /// * `inserted` - The text that was inserted.
    /// * `deleted` - The text that was deleted.
    pub fn push(&mut self, range: Range, inserted: &str, deleted: &str) {
        self.redo.clear();
        if self.undo.len() >= self.max {
            self.undo.pop_front();
        }
        self.undo.push_back(StoredCmd {
            range,
            inserted: inserted.into(),
            deleted: deleted.into(),
        });
    }

    /// Pops the most recent command from the undo stack.
    ///
    /// # Returns
    /// `Some(StoredCmd)` if available, otherwise `None`.
    pub fn pop_undo(&mut self) -> Option<StoredCmd> {
        self.undo.pop_back()
    }

    /// Pushes a command onto the redo stack (used when undoing).
    ///
    /// # Arguments
    /// * `c` - The command to store.
    pub fn push_redo(&mut self, c: StoredCmd) {
        self.redo.push_back(c);
    }

    /// Pops the most recent command from the redo stack.
    ///
    /// # Returns
    /// `Some(StoredCmd)` if available, otherwise `None`.
    pub fn pop_redo(&mut self) -> Option<StoredCmd> {
        self.redo.pop_back()
    }

    /// Returns `true` if there is an operation that can be undone.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Returns `true` if there is an operation that can be redone.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// Errors that can occur when undoing or redoing.
#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoRedoError {
    /// No operation to undo.
    #[error("Nothing to undo")]
    NothingToUndo,
    /// No operation to redo.
    #[error("Nothing to redo")]
    NothingToRedo,
}

/// Text buffer wrapping [`PieceTable`] with undo/redo support and line‑start cache.
pub struct TextEdit {
    table: PieceTable,
    undo_stack: UndoStack,
    line_starts: Vec<usize>, // Character offset of each line start (0‑based)
}

impl std::fmt::Debug for TextEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextEdit")
            .field("len", &self.len())
            .finish()
    }
}

impl TextEdit {
    /// Creates an empty `TextEdit`.
    pub fn new() -> Self {
        let mut this = Self {
            table: PieceTable::empty(),
            undo_stack: UndoStack::new(),
            line_starts: Vec::new(),
        };
        this.rebuild_line_starts();
        this
    }

    /// Creates a `TextEdit` from an initial string.
    ///
    /// # Arguments
    /// * `text` - The initial content.
    pub fn from_str(text: &str) -> Self {
        let mut this = Self {
            table: PieceTable::new(text),
            undo_stack: UndoStack::new(),
            line_starts: Vec::new(),
        };
        this.rebuild_line_starts();
        this
    }

    /// Rebuilds the `line_starts` cache from the current document.
    fn rebuild_line_starts(&mut self) {
        let text = self.full_text();
        self.line_starts.clear();
        self.line_starts.push(0);
        for (i, ch) in text.chars().enumerate() {
            if ch == '\n' {
                self.line_starts.push(i + 1);
            }
        }
    }

    /// Returns the number of characters in the buffer.
    pub fn len(&self) -> usize {
        self.table.char_count()
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the whole text as a `String`.
    pub fn full_text(&self) -> String {
        self.table.get_text()
    }

    /// Inserts text at the given character offset.
    ///
    /// # Arguments
    /// * `offset` - Character index where to insert (clamped to buffer length).
    /// * `text` - The text to insert.
    pub fn insert(&mut self, offset: usize, text: &str) {
        let o = offset.min(self.len());
        self.table.insert(o, text);
        self.undo_stack.push(Range::collapsed(o), text, "");
        self.rebuild_line_starts();
    }

    /// Deletes a range of text specified by start and length.
    ///
    /// # Arguments
    /// * `start` - Start character offset (inclusive).
    /// * `length` - Number of characters to delete.
    pub fn delete_range(&mut self, start: usize, length: usize) {
        let s = start.min(self.len());
        let l = length.min(self.len() - s);
        if l == 0 {
            return;
        }
        let deleted = self.table.get_range(s, l);
        self.table.delete(s, l);
        self.undo_stack.push(Range::new(s, s + l), "", &deleted);
        self.rebuild_line_starts();
    }

    /// Deletes the text in the given range.
    ///
    /// # Arguments
    /// * `range` - The character range to delete.
    pub fn delete(&mut self, range: &Range) {
        self.delete_range(range.start, range.len());
    }

    /// Replaces the text in the given range with new text.
    ///
    /// # Arguments
    /// * `range` - The character range to replace.
    /// * `new_text` - The replacement text.
    pub fn replace(&mut self, range: &Range, new_text: &str) {
        let s = range.start.min(self.len());
        let ol = range.len().min(self.len() - s);
        let deleted = if ol > 0 {
            self.table.get_range(s, ol)
        } else {
            String::new()
        };
        self.table.delete(s, ol);
        self.table.insert(s, new_text);
        self.undo_stack
            .push(Range::new(s, s + ol), new_text, &deleted);
        self.rebuild_line_starts();
    }

    /// Retrieves the text from the given range.
    ///
    /// # Arguments
    /// * `range` - The character range to extract.
    ///
    /// # Returns
    /// The substring as a `String`.
    pub fn get_text(&self, range: &Range) -> String {
        let s = range.start.min(self.len());
        let l = range.len().min(self.len() - s);
        if l == 0 {
            String::new()
        } else {
            self.table.get_range(s, l)
        }
    }

    /// Returns the character at the given offset, if any.
    ///
    /// # Arguments
    /// * `offset` - Character offset.
    ///
    /// # Returns
    /// `Some(char)` if the offset is valid, otherwise `None`.
    pub fn char_at(&self, offset: usize) -> Option<char> {
        if offset >= self.len() {
            None
        } else {
            self.table.get_range(offset, 1).chars().next()
        }
    }

    /// Returns the number of lines in the buffer.
    ///
    /// # Returns
    /// The line count (always ≥ 1).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Finds the character offset of the start of a given line.
    ///
    /// # Arguments
    /// * `line` - Line index (0‑based).
    ///
    /// # Returns
    /// `Some(offset)` if the line exists, otherwise `None`.
    pub fn line_to_char(&self, line: usize) -> Option<usize> {
        self.line_starts.get(line).copied()
    }

    /// Finds the line number that contains the given character offset.
    ///
    /// # Arguments
    /// * `offset` - Character offset.
    ///
    /// # Returns
    /// `Some(line)` (0‑based) if the offset is within the document, otherwise `None`.
    ///
    /// # Behavior
    /// * If the document is empty, returns `Some(0)` for `offset == 0`.
    /// * If `offset` points exactly to the start of a newline character, it belongs to the line **before** the newline.
    /// * If `offset` equals the document length, returns `None` (except empty document).
    pub fn char_to_line(&self, offset: usize) -> Option<usize> {
        let len = self.len();
        if len == 0 {
            return if offset == 0 { Some(0) } else { None };
        }
        if offset > len {
            return None;
        }
        if offset == len {
            return None;
        }
        match self.line_starts.binary_search(&offset) {
            Ok(idx) => Some(idx),
            Err(idx) => Some(idx - 1),
        }
    }

    /// Undoes the last edit operation.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(UndoRedoError::NothingToUndo)` if there is nothing to undo.
    pub fn undo(&mut self) -> Result<(), UndoRedoError> {
        let Some(c) = self.undo_stack.pop_undo() else {
            return Err(UndoRedoError::NothingToUndo);
        };
        if c.inserted.is_empty() {
            self.table.insert(c.range.start, &c.deleted);
        } else if c.deleted.is_empty() {
            self.table.delete(c.range.start, c.inserted.len());
        } else {
            self.table.delete(c.range.start, c.inserted.len());
            self.table.insert(c.range.start, &c.deleted);
        }
        self.undo_stack.push_redo(c);
        self.rebuild_line_starts();
        Ok(())
    }

    /// Redoes the last undone edit operation.
    ///
    /// # Returns
    /// `Ok(())` on success, or `Err(UndoRedoError::NothingToRedo)` if there is nothing to redo.
    pub fn redo(&mut self) -> Result<(), UndoRedoError> {
        let Some(c) = self.undo_stack.pop_redo() else {
            return Err(UndoRedoError::NothingToRedo);
        };
        if c.deleted.is_empty() {
            self.table.insert(c.range.start, &c.inserted);
        } else if c.inserted.is_empty() {
            self.table.delete(c.range.start, c.range.len());
        } else {
            self.table.delete(c.range.start, c.range.len());
            self.table.insert(c.range.start, &c.inserted);
        }
        let ne = c.range.start + c.inserted.len();
        self.undo_stack
            .push(Range::new(c.range.start, ne), &c.inserted, &c.deleted);
        self.rebuild_line_starts();
        Ok(())
    }

    /// Returns `true` if an undo operation is possible.
    pub fn can_undo(&self) -> bool {
        self.undo_stack.can_undo()
    }

    /// Returns `true` if a redo operation is possible.
    pub fn can_redo(&self) -> bool {
        self.undo_stack.can_redo()
    }
}

impl Clone for TextEdit {
    /// Creates a deep copy by rebuilding the `PieceTable` from the current text.
    fn clone(&self) -> Self {
        let mut new = Self {
            table: PieceTable::new(&self.full_text()),
            undo_stack: self.undo_stack.clone(),
            line_starts: Vec::new(),
        };
        new.rebuild_line_starts();
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let t = TextEdit::new();
        assert!(t.is_empty());
        assert_eq!(t.line_count(), 1);
    }

    #[test]
    fn test_insert() {
        let mut t = TextEdit::from_str("ab");
        t.insert(1, "X");
        assert_eq!(t.full_text(), "aXb");
    }

    #[test]
    fn test_undo_redo() {
        let mut t = TextEdit::from_str("hi");
        t.insert(2, "!");
        assert!(t.undo().is_ok());
        assert_eq!(t.full_text(), "hi");
        assert!(t.redo().is_ok());
        assert_eq!(t.full_text(), "hi!");
    }

    #[test]
    fn test_undo_error() {
        let mut t = TextEdit::from_str("hi");
        let err = t.undo().unwrap_err();
        assert_eq!(err, UndoRedoError::NothingToUndo);
    }

    #[test]
    fn test_redo_error() {
        let mut t = TextEdit::from_str("hi");
        let err = t.redo().unwrap_err();
        assert_eq!(err, UndoRedoError::NothingToRedo);
    }

    #[test]
    fn test_lines() {
        let t = TextEdit::from_str("a\nb\nc");
        assert_eq!(t.line_count(), 3);
    }

    #[test]
    fn test_line_to_char() {
        let t = TextEdit::from_str("a\nb\nc");
        assert_eq!(t.line_to_char(0), Some(0));
        assert_eq!(t.line_to_char(1), Some(2));
        assert_eq!(t.line_to_char(2), Some(4));
        assert_eq!(t.line_to_char(3), None);
    }

    #[test]
    fn test_char_to_line() {
        let t = TextEdit::from_str("a\nb\nc");
        assert_eq!(t.char_to_line(0), Some(0));
        assert_eq!(t.char_to_line(1), Some(0));
        assert_eq!(t.char_to_line(2), Some(1));
        assert_eq!(t.char_to_line(3), Some(1));
        assert_eq!(t.char_to_line(4), Some(2));
        assert_eq!(t.char_to_line(5), None);
        assert_eq!(t.char_to_line(10), None);
    }

    #[test]
    fn test_empty() {
        let t = TextEdit::from_str("");
        assert_eq!(t.line_count(), 1);
        assert_eq!(t.line_to_char(0), Some(0));
        assert_eq!(t.line_to_char(1), None);
        assert_eq!(t.char_to_line(0), Some(0));
        assert_eq!(t.char_to_line(1), None);
    }

    #[test]
    fn test_replace() {
        let mut t = TextEdit::from_str("hello world");
        t.replace(&Range::new(6, 11), "Rust");
        assert_eq!(t.full_text(), "hello Rust");
    }

    #[test]
    fn test_get_text() {
        let t = TextEdit::from_str("abcde");
        assert_eq!(t.get_text(&Range::new(1, 4)), "bcd");
    }

    #[test]
    fn test_char_at() {
        let t = TextEdit::from_str("abc");
        assert_eq!(t.char_at(1), Some('b'));
        assert_eq!(t.char_at(3), None);
    }

    #[test]
    fn test_delete_range() {
        let mut t = TextEdit::from_str("hello world");
        t.delete_range(5, 6);
        assert_eq!(t.full_text(), "hello");
    }

    #[test]
    fn test_replace_undo() {
        let mut t = TextEdit::from_str("foo");
        t.replace(&Range::new(0, 3), "bar");
        assert!(t.undo().is_ok());
        assert_eq!(t.full_text(), "foo");
    }
}
