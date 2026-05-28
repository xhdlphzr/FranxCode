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

//! Cursor movement and multi-cursor selection management.
//! Word boundaries via [`unicode_segmentation`] scanning only the current line for performance.

use crate::edit::{Range, TextEdit};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

/// Movement direction for cursor actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Movement {
    Up,
    Down,
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    FileStart,
    FileEnd,
}

/// The mode of the cursor (not currently used, reserved for future expansion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorMode {
    Insert,
    Select,
}

/// A single selection (or cursor) defined by anchor and active positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub anchor: usize,
    pub active: usize,
    pub preferred_x: Option<usize>,
}

impl Selection {
    /// Creates a collapsed selection (caret) at the given offset.
    ///
    /// # Arguments
    /// * `o` - Character offset of the caret.
    ///
    /// # Returns
    /// A new `Selection` with both anchor and active set to `o`.
    pub fn caret(o: usize) -> Self {
        Self {
            anchor: o,
            active: o,
            preferred_x: None,
        }
    }

    /// Creates a selection from two offsets.
    ///
    /// # Arguments
    /// * `a` - First offset (anchor).
    /// * `b` - Second offset (active).
    ///
    /// # Returns
    /// A new `Selection` with the given anchor and active.
    pub fn new(a: usize, b: usize) -> Self {
        Self {
            anchor: a,
            active: b,
            preferred_x: None,
        }
    }

    /// Returns the normalized range of this selection (start <= end).
    ///
    /// # Returns
    /// A `Range` where start is less than or equal to end.
    pub fn range(&self) -> Range {
        let s = self.anchor.min(self.active);
        let e = self.anchor.max(self.active);
        Range::new(s, e)
    }

    /// Returns `true` if the selection is collapsed (anchor == active).
    ///
    /// # Returns
    /// `true` if the selection has zero length, `false` otherwise.
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.active
    }

    /// Returns the active offset (the cursor position).
    ///
    /// # Returns
    /// The active offset as a character index.
    pub fn cursor(&self) -> usize {
        self.active
    }

    /// Returns the start offset of the normalized range.
    ///
    /// # Returns
    /// The smaller of anchor and active.
    pub fn start(&self) -> usize {
        self.anchor.min(self.active)
    }

    /// Returns the end offset of the normalized range.
    ///
    /// # Returns
    /// The larger of anchor and active.
    pub fn end(&self) -> usize {
        self.anchor.max(self.active)
    }
}

/// Manages multiple selections and the primary cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    selections: Vec<Selection>,
    primary_index: usize,
}

impl Cursor {
    /// Creates a new `Cursor` with a single caret at offset 0.
    ///
    /// # Returns
    /// A new `Cursor` instance.
    pub fn new() -> Self {
        Self {
            selections: vec![Selection::caret(0)],
            primary_index: 0,
        }
    }

    /// Creates a new `Cursor` with a single caret at the given offset.
    ///
    /// # Arguments
    /// * `o` - Character offset of the caret.
    ///
    /// # Returns
    /// A new `Cursor` instance.
    pub fn at(o: usize) -> Self {
        Self {
            selections: vec![Selection::caret(o)],
            primary_index: 0,
        }
    }

    /// Returns the list of all selections.
    ///
    /// # Returns
    /// A slice of all selections.
    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    /// Returns the primary selection.
    ///
    /// # Returns
    /// A reference to the primary selection.
    pub fn primary(&self) -> &Selection {
        &self.selections[self.primary_index]
    }

    /// Returns a mutable reference to the primary selection.
    ///
    /// # Returns
    /// A mutable reference to the primary selection.
    pub fn primary_mut(&mut self) -> &mut Selection {
        &mut self.selections[self.primary_index]
    }

    /// Replaces all selections with new ranges.
    ///
    /// # Arguments
    /// * `ranges` - Vector of character ranges that become selections.
    pub fn set_selections(&mut self, ranges: Vec<Range>) {
        if ranges.is_empty() {
            return;
        }
        let mut sels: Vec<_> = ranges
            .into_iter()
            .map(|r| Selection::new(r.start, r.end))
            .collect();
        sels.sort_by_key(|s| s.start());
        sels.dedup_by_key(|s| s.start());
        self.selections = sels;
        self.primary_index = 0;
    }

    /// Returns the character offset of the primary cursor.
    ///
    /// # Returns
    /// The primary cursor position as a character offset.
    pub fn position(&self) -> usize {
        self.primary().cursor()
    }

    /// Returns `true` if any selection is non‑collapsed.
    ///
    /// # Returns
    /// `true` if there is at least one selection with length > 0, `false` otherwise.
    pub fn has_selection(&self) -> bool {
        self.selections.iter().any(|s| !s.is_collapsed())
    }

    /// Selects the entire document.
    ///
    /// # Arguments
    /// * `doc_len` - The total length of the document in characters.
    pub fn select_all(&mut self, doc_len: usize) {
        self.selections = vec![Selection::new(0, doc_len)];
        self.primary_index = 0;
    }

    /// Selects the line containing the primary cursor.
    ///
    /// # Arguments
    /// * `text` - The text buffer providing line mapping.
    pub fn select_line(&mut self, text: &TextEdit) {
        let pos = self.position();
        if let Some(line) = text.char_to_line(pos) {
            let a = text.line_to_char(line).unwrap_or(0);
            let b = text.line_to_char(line + 1).unwrap_or(text.len());
            self.selections = vec![Selection::new(a, b)];
            self.primary_index = 0;
        }
    }

    /// Moves the primary cursor according to a movement command.
    ///
    /// # Arguments
    /// * `m` - The movement direction.
    /// * `text` - The text buffer for layout calculations.
    /// * `extend` - If `true`, extends the current selection; otherwise replaces it.
    pub fn move_cursor(&mut self, m: Movement, text: &TextEdit, extend: bool) {
        let dl = text.len();
        let pos = self.primary().cursor();
        let np = match m {
            Movement::Left => pos.saturating_sub(1),
            Movement::Right => (pos + 1).min(dl),
            Movement::Up => self.vert(pos, -1, text),
            Movement::Down => self.vert(pos, 1, text),
            Movement::WordLeft => self.word_left(pos, text),
            Movement::WordRight => self.word_right(pos, text),
            Movement::LineStart => self.line_start(pos, text),
            Movement::LineEnd => self.line_end(pos, text),
            Movement::FileStart => 0,
            Movement::FileEnd => dl,
        };
        let px = self.h_offset(np, text);
        if extend {
            let s = self.primary_mut();
            s.active = np;
            s.preferred_x = Some(px);
        } else {
            let mut s = Selection::caret(np);
            s.preferred_x = Some(px);
            self.selections[self.primary_index] = s;
        }
    }

    /// Moves the primary cursor up.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_up(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::Up, t, e);
    }

    /// Moves the primary cursor down.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_down(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::Down, t, e);
    }

    /// Moves the primary cursor left.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_left(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::Left, t, e);
    }

    /// Moves the primary cursor right.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_right(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::Right, t, e);
    }

    /// Moves the primary cursor one word left.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_word_left(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::WordLeft, t, e);
    }

    /// Moves the primary cursor one word right.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_word_right(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::WordRight, t, e);
    }

    /// Moves the primary cursor to the start of the current line.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_to_line_start(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::LineStart, t, e);
    }

    /// Moves the primary cursor to the end of the current line.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_to_line_end(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::LineEnd, t, e);
    }

    /// Moves the primary cursor to the start of the file.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_to_file_start(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::FileStart, t, e);
    }

    /// Moves the primary cursor to the end of the file.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    /// * `e` - Extend selection if `true`.
    pub fn move_to_file_end(&mut self, t: &TextEdit, e: bool) {
        self.move_cursor(Movement::FileEnd, t, e);
    }

    /// Extends the primary selection upward.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    pub fn extend_selection_up(&mut self, t: &TextEdit) {
        self.move_cursor(Movement::Up, t, true);
    }

    /// Extends the primary selection downward.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    pub fn extend_selection_down(&mut self, t: &TextEdit) {
        self.move_cursor(Movement::Down, t, true);
    }

    /// Extends the primary selection to the left.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    pub fn extend_selection_left(&mut self, t: &TextEdit) {
        self.move_cursor(Movement::Left, t, true);
    }

    /// Extends the primary selection to the right.
    ///
    /// # Arguments
    /// * `t` - The text buffer.
    pub fn extend_selection_right(&mut self, t: &TextEdit) {
        self.move_cursor(Movement::Right, t, true);
    }

    /// Adds a new cursor above the primary cursor.
    ///
    /// # Arguments
    /// * `t` - The text buffer for vertical positioning.
    pub fn add_cursor_above(&mut self, t: &TextEdit) {
        let p = self.position();
        let np = self.vert(p, -1, t);
        if np != p {
            self.selections.push(Selection::caret(np));
        }
    }

    /// Adds a new cursor below the primary cursor.
    ///
    /// # Arguments
    /// * `t` - The text buffer for vertical positioning.
    pub fn add_cursor_below(&mut self, t: &TextEdit) {
        let p = self.position();
        let np = self.vert(p, 1, t);
        if np != p {
            self.selections.push(Selection::caret(np));
        }
    }

    /// Splits **all** selections into per‑line carets, preserving each selection's preferred x‑offset.
    ///
    /// # Arguments
    /// * `t` - The text buffer for line layout.
    pub fn split_selection_into_lines(&mut self, t: &TextEdit) {
        let mut new_selections = Vec::new();
        for sel in &self.selections {
            if sel.is_collapsed() {
                new_selections.push(*sel);
                continue;
            }
            let r = sel.range();
            let l0 = t.char_to_line(r.start).unwrap_or(0);
            let l1 = t.char_to_line(r.end).unwrap_or(0);
            for line in l0..=l1 {
                let ls = t.line_to_char(line).unwrap_or(0);
                let le = t.line_to_char(line + 1).unwrap_or(t.len());
                let ll = le.saturating_sub(ls);
                let ho = sel.preferred_x.unwrap_or(0).min(ll.saturating_sub(1));
                let caret_pos = (ls + ho).min(t.len());
                new_selections.push(Selection::caret(caret_pos));
            }
        }
        if !new_selections.is_empty() {
            new_selections.sort_by_key(|s| s.cursor());
            new_selections.dedup_by_key(|s| s.cursor());
            self.selections = new_selections;
            self.primary_index = 0;
        }
    }

    /// Returns a list of normalized ranges for editing, sorted from largest start to smallest.
    ///
    /// # Returns
    /// A vector of `Range` suitable for applying edits in reverse order.
    pub fn edit_ranges(&self) -> Vec<Range> {
        let mut r: Vec<Range> = self.selections.iter().map(|s| s.range()).collect();
        r.sort_by(|a, b| b.start.cmp(&a.start));
        r
    }
}

impl Cursor {
    /// Computes the character offset after moving vertically by `delta` lines.
    ///
    /// # Arguments
    /// * `pos` - Current character offset.
    /// * `delta` - Number of lines to move (positive down, negative up).
    /// * `t` - Text buffer for line layout.
    ///
    /// # Returns
    /// The new character offset after vertical movement.
    fn vert(&self, pos: usize, delta: isize, t: &TextEdit) -> usize {
        let dl = t.len();
        if dl == 0 {
            return 0;
        }
        let cl = t.char_to_line(pos).unwrap_or(0) as isize;
        let tl = (cl + delta).max(0) as usize;
        let lc = t.line_count();
        let tl = tl.min(lc.saturating_sub(1));
        let ls = t.line_to_char(tl).unwrap_or(0);
        let nx = t.line_to_char(tl + 1).unwrap_or(dl);
        let ll = nx.saturating_sub(ls);
        let ho = self
            .primary()
            .preferred_x
            .unwrap_or_else(|| self.h_offset(pos, t));
        if ll == 0 {
            ls
        } else {
            (ls + ho.min(ll.saturating_sub(1))).min(dl)
        }
    }

    /// Returns the horizontal offset (column) of the given character offset within its line.
    ///
    /// # Arguments
    /// * `pos` - Character offset.
    /// * `t` - Text buffer.
    ///
    /// # Returns
    /// The column index (0‑based) within the line.
    fn h_offset(&self, pos: usize, t: &TextEdit) -> usize {
        t.char_to_line(pos)
            .map_or(0, |l| pos.saturating_sub(t.line_to_char(l).unwrap_or(0)))
    }

    /// Returns the character offset of the start of the line containing `pos`.
    ///
    /// # Arguments
    /// * `pos` - Current character offset.
    /// * `t` - Text buffer.
    ///
    /// # Returns
    /// The offset of the first character of the line.
    fn line_start(&self, pos: usize, t: &TextEdit) -> usize {
        t.char_to_line(pos)
            .map_or(0, |l| t.line_to_char(l).unwrap_or(0))
    }

    /// Returns the character offset of the end of the line containing `pos`.
    ///
    /// # Arguments
    /// * `pos` - Current character offset.
    /// * `t` - Text buffer.
    ///
    /// # Returns
    /// The offset of the last character of the line (before newline, if present).
    fn line_end(&self, pos: usize, t: &TextEdit) -> usize {
        let dl = t.len();
        t.char_to_line(pos).map_or(dl, |l| {
            let nx = t.line_to_char(l + 1).unwrap_or(dl);
            if nx > 0 && t.char_at(nx.saturating_sub(1)) == Some('\n') {
                nx.saturating_sub(1)
            } else {
                nx
            }
        })
    }

    /// Move left to the previous word boundary, scanning only the current line.
    ///
    /// # Arguments
    /// * `pos` - Current character offset.
    /// * `t` - Text buffer.
    ///
    /// # Returns
    /// The new character offset at the previous word boundary.
    fn word_left(&self, pos: usize, t: &TextEdit) -> usize {
        if pos == 0 {
            return 0;
        }
        let line_idx = t.char_to_line(pos).unwrap_or(0);
        let line_start = t.line_to_char(line_idx).unwrap_or(0);
        let line_end = t.line_to_char(line_idx + 1).unwrap_or(t.len());
        if pos == line_start {
            if line_idx == 0 {
                return 0;
            }
            return t.line_to_char(line_idx).unwrap_or(0);
        }
        let line_text = t.get_text(&Range::new(line_start, line_end));
        let local_pos = pos - line_start;
        let mut boundaries = Vec::new();
        for (i, _is_word) in line_text.split_word_bound_indices() {
            boundaries.push(i);
        }
        boundaries.push(line_text.len());
        let mut prev = 0;
        for &b in &boundaries {
            if b >= local_pos {
                break;
            }
            prev = b;
        }
        line_start + prev
    }

    /// Move right to the next word boundary, scanning only the current line.
    ///
    /// # Arguments
    /// * `pos` - Current character offset.
    /// * `t` - Text buffer.
    ///
    /// # Returns
    /// The new character offset at the next word boundary.
    fn word_right(&self, pos: usize, t: &TextEdit) -> usize {
        let dl = t.len();
        if pos >= dl {
            return dl;
        }
        let line_idx = t.char_to_line(pos).unwrap_or(0);
        let line_start = t.line_to_char(line_idx).unwrap_or(0);
        let line_end = t.line_to_char(line_idx + 1).unwrap_or(dl);
        if pos == line_end {
            return t.line_to_char(line_idx + 1).unwrap_or(dl);
        }
        let line_text = t.get_text(&Range::new(line_start, line_end));
        let local_pos = pos - line_start;
        for (i, _is_word) in line_text.split_word_bound_indices() {
            if i > local_pos {
                return line_start + i;
            }
        }
        line_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(s: &str) -> TextEdit {
        TextEdit::from_str(s)
    }

    #[test]
    fn test_caret() {
        let s = Selection::caret(5);
        assert!(s.is_collapsed());
        assert_eq!(s.cursor(), 5);
    }

    #[test]
    fn test_move_lr() {
        let tx = t("hello");
        let mut c = Cursor::at(2);
        c.move_left(&tx, false);
        assert_eq!(c.position(), 1);
        c.move_right(&tx, false);
        assert_eq!(c.position(), 2);
    }

    #[test]
    fn test_move_ud() {
        let tx = t("a\nb\nc");
        let mut c = Cursor::at(2);
        c.move_up(&tx, false);
        assert_eq!(c.position(), 0);
        c.move_down(&tx, false);
        assert_eq!(c.position(), 2);
    }

    #[test]
    fn test_select_all() {
        let tx = t("abc");
        let mut c = Cursor::at(0);
        c.select_all(tx.len());
        assert!(c.has_selection());
    }

    #[test]
    fn test_extend() {
        let tx = t("ab");
        let mut c = Cursor::at(0);
        c.extend_selection_right(&tx);
        assert_eq!(c.primary().range(), Range::new(0, 1));
    }

    #[test]
    fn test_multi_cursor() {
        let tx = t("a\nb\nc");
        let mut c = Cursor::at(0);
        c.add_cursor_below(&tx);
        assert_eq!(c.selections().len(), 2);
    }

    #[test]
    fn test_word_movement() {
        let tx = t("hello world");
        let mut c = Cursor::at(0);
        c.move_word_right(&tx, false);
        assert_eq!(c.position(), 5);
        c.move_word_right(&tx, false);
        assert_eq!(c.position(), 6);
    }

    #[test]
    fn test_set_selections() {
        let mut c = Cursor::new();
        c.set_selections(vec![Range::new(0, 5), Range::new(6, 11)]);
        assert_eq!(c.selections.len(), 2);
        assert_eq!(c.selections[0].range(), Range::new(0, 5));
        assert_eq!(c.selections[1].range(), Range::new(6, 11));
        assert_eq!(c.primary().cursor(), 5);
    }

    #[test]
    fn test_select_line() {
        let tx = t("line1\nline2\nline3");
        let mut c = Cursor::at(8);
        c.select_line(&tx);
        assert_eq!(c.selections.len(), 1);
        assert_eq!(c.primary().range(), Range::new(6, 12));
        let tx2 = t("a\nb\nc");
        let mut c2 = Cursor::at(2);
        c2.select_line(&tx2);
        assert_eq!(c2.primary().range(), Range::new(2, 4));
    }

    #[test]
    fn test_split_selection_into_lines() {
        let tx = t("line1\nline2\nline3\nline4");
        let mut c = Cursor::new();
        c.set_selections(vec![Range::new(6, 12)]);
        c.split_selection_into_lines(&tx);
        assert_eq!(c.selections.len(), 2);
        assert_eq!(c.selections[0].cursor(), 6);
        assert_eq!(c.selections[1].cursor(), 12);
    }
}
