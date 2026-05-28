// Copyright (C) 2026 xhdlphzr
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Timestamped edit history with sequence-based time-travel and compressed snapshots.

use crate::edit::{Range, TextEdit};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use zstd::{decode_all, encode_all};

static RECORD_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A version identifier with sequence number and timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub sequence: u64,
}

impl Version {
    /// Creates a new version with the given sequence number.
    ///
    /// # Arguments
    /// * `seq` - The sequence number (monotonically increasing).
    ///
    /// # Returns
    /// A `Version` with unique id, current UTC timestamp, and the given sequence.
    pub fn new(seq: u64) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self {
            id: format!("v{}-{}", seq, nanos),
            timestamp: Utc::now(),
            sequence: seq,
        }
    }
}

/// Type of edit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditType {
    Insert,
    Delete,
    Replace,
    Undo,
    Redo,
}

impl std::fmt::Display for EditType {
    /// Formats the edit type as a string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditType::Insert => write!(f, "insert"),
            EditType::Delete => write!(f, "delete"),
            EditType::Replace => write!(f, "replace"),
            EditType::Undo => write!(f, "undo"),
            EditType::Redo => write!(f, "redo"),
        }
    }
}

/// A single recorded edit operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditRecord {
    pub id: String,
    pub edit_type: EditType,
    pub range: Range,
    pub inserted_text: String,
    pub deleted_text: String,
    pub timestamp: DateTime<Utc>,
    pub version_before: Version,
    pub version_after: Version,
}

impl EditRecord {
    /// Creates a new `EditRecord` from its components.
    ///
    /// # Arguments
    /// * `et` - The type of edit.
    /// * `range` - The character range affected.
    /// * `inserted` - The inserted text (if any).
    /// * `deleted` - The deleted text (if any).
    /// * `before` - The version before the edit.
    /// * `after` - The version after the edit.
    ///
    /// # Returns
    /// A new `EditRecord` with a unique id and current timestamp.
    pub fn from_parts(
        et: EditType,
        range: Range,
        inserted: &str,
        deleted: &str,
        before: Version,
        after: Version,
    ) -> Self {
        let id = format!("rec-{}", RECORD_COUNTER.fetch_add(1, Ordering::Relaxed));
        Self {
            id,
            edit_type: et,
            range,
            inserted_text: inserted.into(),
            deleted_text: deleted.into(),
            timestamp: Utc::now(),
            version_before: before,
            version_after: after,
        }
    }
}

/// Manages edit history with compressed snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryTimeline {
    records: VecDeque<EditRecord>,
    snapshots: VecDeque<(Version, Vec<u8>)>, // compressed text
    current_version: Version,
    next_sequence: u64,
    max_records: usize,
    snapshot_interval: usize,
    edits_since_snapshot: usize,
}

impl HistoryTimeline {
    /// Creates a new timeline with default settings (max 10000 records, snapshot every 100 edits).
    pub fn new() -> Self {
        Self {
            records: VecDeque::new(),
            snapshots: VecDeque::new(),
            current_version: Version::new(0),
            next_sequence: 1,
            max_records: 10000,
            snapshot_interval: 100,
            edits_since_snapshot: 0,
        }
    }

    /// Creates a new timeline with custom capacity and snapshot interval.
    ///
    /// # Arguments
    /// * `max_records` - Maximum number of edit records to keep.
    /// * `snapshot_interval` - How many edits between automatic snapshots.
    pub fn with_config(max_records: usize, snapshot_interval: usize) -> Self {
        Self {
            records: VecDeque::with_capacity(max_records),
            snapshots: VecDeque::new(),
            current_version: Version::new(0),
            next_sequence: 1,
            max_records,
            snapshot_interval,
            edits_since_snapshot: 0,
        }
    }

    /// Records an edit operation.
    ///
    /// # Arguments
    /// * `et` - The edit type.
    /// * `range` - The affected character range.
    /// * `inserted` - Inserted text (may be empty).
    /// * `deleted` - Deleted text (may be empty).
    /// * `current_text` - The full text after the edit (for snapshotting).
    pub fn record_edit(
        &mut self,
        et: EditType,
        range: Range,
        inserted: &str,
        deleted: &str,
        current_text: &str,
    ) {
        let before = self.current_version.clone();
        let after = Version::new(self.next_sequence);
        self.next_sequence += 1;
        let rec = EditRecord::from_parts(et, range, inserted, deleted, before, after.clone());
        if self.records.len() >= self.max_records {
            self.records.pop_front();
            if self.snapshots.len() > 1 {
                self.snapshots.pop_front();
            }
        }
        self.records.push_back(rec);
        self.current_version = after;
        self.edits_since_snapshot += 1;
        if self.edits_since_snapshot >= self.snapshot_interval {
            self.take_snapshot(current_text);
        }
    }

    /// Returns all edit records whose timestamps lie in the given interval.
    ///
    /// # Arguments
    /// * `from` - Start timestamp (inclusive).
    /// * `to` - End timestamp (inclusive).
    ///
    /// # Returns
    /// A vector of references to matching `EditRecord`s.
    pub fn get_history(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<&EditRecord> {
        self.records
            .iter()
            .filter(|r| r.timestamp >= from && r.timestamp <= to)
            .collect()
    }

    /// Returns the most recent `n` edit records.
    ///
    /// # Arguments
    /// * `n` - Number of records to return.
    ///
    /// # Returns
    /// A vector of references to the most recent records (most recent first).
    pub fn recent_records(&self, n: usize) -> Vec<&EditRecord> {
        self.records.iter().rev().take(n).collect()
    }

    /// Returns all edit records in chronological order.
    ///
    /// # Returns
    /// A vector of references to all records.
    pub fn all_records(&self) -> Vec<&EditRecord> {
        self.records.iter().collect()
    }

    /// Finds the latest snapshot taken at or before the given timestamp.
    ///
    /// # Arguments
    /// * `ts` - The timestamp.
    ///
    /// # Returns
    /// An optional tuple of version and decompressed text.
    pub fn get_snapshot_at(&self, ts: DateTime<Utc>) -> Option<(Version, String)> {
        self.snapshots
            .iter()
            .filter(|(v, _)| v.timestamp <= ts)
            .last()
            .and_then(|(v, compressed)| {
                decode_all(&compressed[..])
                    .ok()
                    .map(|bytes| (v.clone(), String::from_utf8(bytes).unwrap_or_default()))
            })
    }

    /// Returns the current version.
    pub fn get_current_version(&self) -> &Version {
        &self.current_version
    }

    /// Returns the number of stored edit records.
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Clears all history and snapshots, resetting to version 0.
    pub fn clear(&mut self) {
        self.records.clear();
        self.snapshots.clear();
        self.current_version = Version::new(0);
        self.next_sequence = 1;
        self.edits_since_snapshot = 0;
    }

    /// Takes a full‑text snapshot of the current state (compressed).
    ///
    /// # Arguments
    /// * `text` - The full document text to store.
    pub fn take_snapshot(&mut self, text: &str) {
        if let Ok(compressed) = encode_all(text.as_bytes(), 3) {
            self.snapshots
                .push_back((self.current_version.clone(), compressed));
        }
        self.edits_since_snapshot = 0;
        while self.snapshots.len() > self.max_records / self.snapshot_interval + 10 {
            self.snapshots.pop_front();
        }
    }

    /// Restores the editor to a specific version by its ID.
    ///
    /// # Arguments
    /// * `version_id` - The version identifier string.
    /// * `text` - The mutable `TextEdit` to modify.
    ///
    /// # Returns
    /// `Ok(true)` if the version was found and restored, `Ok(false)` otherwise.
    pub fn goto_version(&mut self, version_id: &str, text: &mut TextEdit) -> anyhow::Result<bool> {
        let target = self
            .records
            .iter()
            .find(|r| r.version_after.id == version_id)
            .map(|r| r.version_after.sequence)
            .or_else(|| {
                self.snapshots
                    .iter()
                    .find(|(v, _)| v.id == version_id)
                    .map(|(v, _)| v.sequence)
            });
        let Some(target_seq) = target else {
            return Ok(false);
        };
        self.goto_sequence(target_seq, text)
    }

    /// Restores the editor to the state at a given timestamp.
    ///
    /// # Arguments
    /// * `ts` - The target timestamp.
    /// * `text` - The mutable `TextEdit` to modify.
    ///
    /// # Returns
    /// `Ok(true)` if the time was reached, `Ok(false)` otherwise.
    pub fn goto_time(&mut self, ts: DateTime<Utc>, text: &mut TextEdit) -> anyhow::Result<bool> {
        let snap_idx = self.snapshots.iter().rposition(|(v, _)| v.timestamp <= ts);
        let (start_seq, snap_text) = if let Some(idx) = snap_idx {
            let (v, compressed) = &self.snapshots[idx];
            let decompressed = decode_all(&compressed[..])?;
            (
                v.sequence,
                String::from_utf8(decompressed).unwrap_or_default(),
            )
        } else {
            (0, String::new())
        };

        let full = Range::new(0, text.len());
        text.replace(&full, &snap_text);

        for rec in &self.records {
            if rec.version_after.sequence <= start_seq {
                continue;
            }
            if rec.timestamp > ts {
                break;
            }
            match rec.edit_type {
                EditType::Insert => text.insert(rec.range.start, &rec.inserted_text),
                EditType::Delete => text.delete_range(rec.range.start, rec.range.len()),
                EditType::Replace => text.replace(&rec.range, &rec.inserted_text),
                _ => {}
            }
        }
        self.update_current_version_after_sequence(start_seq);
        Ok(true)
    }

    /// Internal method: restores to a specific sequence number.
    ///
    /// # Arguments
    /// * `target_seq` - The target sequence number.
    /// * `text` - The mutable `TextEdit` to modify.
    ///
    /// # Returns
    /// `Ok(true)` if the sequence was found and restored, `Ok(false)` otherwise.
    pub fn goto_sequence(&mut self, target_seq: u64, text: &mut TextEdit) -> anyhow::Result<bool> {
        let snap = self
            .snapshots
            .iter()
            .rev()
            .find(|(v, _)| v.sequence <= target_seq);
        let (start_seq, snap_text) = if let Some((v, compressed)) = snap {
            let decompressed = decode_all(&compressed[..])?;
            (
                v.sequence,
                String::from_utf8(decompressed).unwrap_or_default(),
            )
        } else {
            (0, String::new())
        };

        let full = Range::new(0, text.len());
        text.replace(&full, &snap_text);

        for rec in &self.records {
            if rec.version_after.sequence <= start_seq {
                continue;
            }
            if rec.version_after.sequence > target_seq {
                break;
            }
            match rec.edit_type {
                EditType::Insert => text.insert(rec.range.start, &rec.inserted_text),
                EditType::Delete => text.delete_range(rec.range.start, rec.range.len()),
                EditType::Replace => text.replace(&rec.range, &rec.inserted_text),
                _ => {}
            }
        }
        self.update_current_version_after_sequence(target_seq);
        Ok(true)
    }

    /// Helper to update `current_version` and `next_sequence` after a sequence‑based restore.
    fn update_current_version_after_sequence(&mut self, target_seq: u64) {
        if let Some(last) = self
            .records
            .iter()
            .rev()
            .find(|r| r.version_after.sequence <= target_seq)
        {
            self.current_version = last.version_after.clone();
            self.next_sequence = last.version_after.sequence + 1;
        } else {
            self.current_version = Version::new(0);
            self.next_sequence = 1;
        }
    }
}

impl Default for HistoryTimeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        let v = Version::new(1);
        assert_eq!(v.sequence, 1);
    }

    #[test]
    fn test_new() {
        assert_eq!(HistoryTimeline::new().record_count(), 0);
    }

    #[test]
    fn test_record() {
        let mut tl = HistoryTimeline::new();
        tl.record_edit(EditType::Insert, Range::collapsed(0), "hello", "", "hello");
        assert_eq!(tl.record_count(), 1);
    }

    #[test]
    fn test_goto_sequence() {
        let mut tl = HistoryTimeline::new();
        let mut t = TextEdit::from_str("");
        tl.take_snapshot("");
        tl.record_edit(EditType::Insert, Range::collapsed(0), "hello", "", "hello");
        tl.record_edit(
            EditType::Insert,
            Range::collapsed(5),
            " world",
            "",
            "hello world",
        );
        tl.take_snapshot("hello world");
        assert!(tl.goto_sequence(1, &mut t).unwrap());
        assert_eq!(t.full_text(), "hello");
        assert_eq!(tl.current_version.sequence, 1);
        assert_eq!(tl.next_sequence, 2);
    }

    #[test]
    fn test_goto_time() {
        let mut tl = HistoryTimeline::new();
        let mut t = TextEdit::from_str("");
        tl.take_snapshot("");
        tl.record_edit(EditType::Insert, Range::collapsed(0), "hello", "", "hello");
        let ts = Utc::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        tl.record_edit(
            EditType::Insert,
            Range::collapsed(5),
            " world",
            "",
            "hello world",
        );
        assert!(tl.goto_time(ts, &mut t).unwrap());
        assert_eq!(t.full_text(), "hello");
    }

    #[test]
    fn test_clear() {
        let mut tl = HistoryTimeline::new();
        tl.record_edit(EditType::Insert, Range::collapsed(0), "a", "", "a");
        assert_eq!(tl.record_count(), 1);
        tl.clear();
        assert_eq!(tl.record_count(), 0);
        assert_eq!(tl.current_version.sequence, 0);
        assert_eq!(tl.next_sequence, 1);
    }

    #[test]
    fn test_snapshot_compression() {
        let mut tl = HistoryTimeline::new();
        let long_text = "A".repeat(1000);
        tl.take_snapshot(&long_text);
        assert_eq!(tl.snapshots.len(), 1);
        let (_, compressed) = tl.snapshots.front().unwrap();
        assert!(compressed.len() < long_text.len());
        let decompressed = decode_all(&compressed[..]).unwrap();
        assert_eq!(String::from_utf8(decompressed).unwrap(), long_text);
    }
}
