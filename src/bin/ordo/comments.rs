// ------------------------------------------------------------- line comments
//! Comments on a line or a range of the new side, the unit a review is written
//! in when the point is narrower than a symbol. A note (`:note`) follows a
//! symbol through renames and moves; a comment stays on its lines, and follows
//! them only while they read the same.

use crate::marks::fnv1a;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct LineComment {
    pub(super) path: String,
    /// 1-based, inclusive, new side
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) text: String,
    /// a hash of the lines it was written on, so it can find them again after
    /// an edit above moves them — the lines themselves are not stored
    anchor: u64,
    /// its lines were edited, or are gone: it still says where they were
    #[serde(default)]
    pub(super) stale: bool,
}

impl LineComment {
    pub(super) fn new(path: &str, start: usize, end: usize, text: &str, lines: &[String]) -> Self {
        LineComment {
            path: path.to_string(),
            start,
            end,
            text: text.to_string(),
            anchor: anchor_of(lines, start, end),
            stale: false,
        }
    }

    /// `path:12` or `path:12-18`
    pub(super) fn at(&self) -> String {
        if self.end > self.start {
            format!("{}:{}-{}", self.path, self.start, self.end)
        } else {
            format!("{}:{}", self.path, self.start)
        }
    }

    /// Put the comment back on its lines in `lines`, the file as it now reads:
    /// where it was, or the nearest place the same lines are found. Neither
    /// found, it stays put and is marked stale rather than dropped — a
    /// comment on code that changed is still worth reading.
    pub(super) fn relocate(&mut self, lines: &[String]) {
        // a hand-edited file can hold a backwards range
        let len = self.end.saturating_sub(self.start) + 1;
        if anchor_of(lines, self.start, self.end) == self.anchor {
            self.stale = false;
            return;
        }
        let found = (1..=lines.len().saturating_sub(len - 1))
            .filter(|&s| anchor_of(lines, s, s + len - 1) == self.anchor)
            .min_by_key(|&s| s.abs_diff(self.start));
        match found {
            Some(s) => {
                (self.start, self.end, self.stale) = (s, s + len - 1, false);
            }
            None => self.stale = true,
        }
    }
}

fn anchor_of(lines: &[String], start: usize, end: usize) -> u64 {
    match lines.get(start.saturating_sub(1)..end.min(lines.len())) {
        Some(ls) if start >= 1 && !ls.is_empty() => fnv1a(ls.join("\n").as_bytes()),
        _ => 0,
    }
}

/// Beside the notes, one file per repository.
pub(super) fn comments_file_path(notes_path: &Path) -> Option<PathBuf> {
    let name = notes_path.file_name()?;
    Some(notes_path.parent()?.parent()?.join("comments").join(name))
}

/// Degrades to "no comments" on any failure, as the notes do.
pub(super) fn load_comments(path: &Path) -> Vec<LineComment> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(super) fn save_comments(path: &Path, comments: &[LineComment]) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(text) = serde_json::to_string(comments) {
        let _ = std::fs::write(path, text);
    }
}
