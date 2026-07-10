//! Serde types for the v1 data contract (see `schema/v1.json`).
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Input {
    pub changes: Vec<Change>,
    #[serde(default)]
    pub options: Options,
}

#[derive(Debug, Deserialize)]
pub struct Change {
    pub path: String,
    #[serde(default)]
    pub old: Option<String>,
    #[serde(default)]
    pub new: Option<String>,
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub strategy: Strategy,
    #[serde(default = "yes")]
    pub cross_file: bool,
    /// Caller asserts each `diff` is a complete (full-context) patch, so a
    /// modified file can be reconstructed for full semantics. Off by default.
    #[serde(default)]
    pub full_context: bool,
}
impl Default for Options {
    fn default() -> Self {
        Options {
            strategy: Strategy::Comprehension,
            cross_file: true,
            full_context: false,
        }
    }
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    Comprehension,
    DefsFirst,
    File,
}
impl Default for Strategy {
    fn default() -> Self {
        Strategy::Comprehension
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Import,
    Definition,
    Other,
}

#[derive(Debug, Serialize)]
pub struct Output {
    pub schema: u32,
    pub order: Vec<OrderItem>,
    pub files: Vec<FileOut>,
    pub groups: Vec<Group>,
    pub edges: Vec<Edge>,
    /// P12.3: independent parts of the change (hunk ids per cluster). One
    /// cluster ⇒ atomic; multiple ⇒ candidate PR split.
    pub clusters: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct OrderItem {
    pub path: String,
    pub hunk: String,
}

#[derive(Debug, Serialize)]
pub struct FileOut {
    pub path: String,
    pub hunks: Vec<HunkOut>,
    /// true when this file could only be ordered positionally (a diff without
    /// full context and no old/new to reconstruct from). Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub degraded: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Serialize)]
pub struct HunkOut {
    pub id: String,
    pub old_range: [usize; 2],
    pub new_range: [usize; 2],
    pub category: Category,
    pub enclosing: Option<String>,
    pub defines: Vec<String>,
    pub uses: Vec<String>,
    pub group: String,
    pub order_index: usize,
    pub rationale: String,
    /// formatting-only or generated-file hunk — skippable for review (P12.2)
    #[serde(default, skip_serializing_if = "is_false")]
    pub noise: bool,
}

#[derive(Debug, Serialize)]
pub struct Group {
    pub id: String,
    pub reason: String,
    pub members: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub why: String,
}
