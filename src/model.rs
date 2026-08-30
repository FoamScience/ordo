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
    /// Drop every non-comment/docstring hunk before grouping, so `order`,
    /// `groups`, `edges` and `clusters` cover only comment-only hunks. Off by
    /// default.
    #[serde(default)]
    pub only_comments: bool,
}
impl Default for Options {
    fn default() -> Self {
        Options {
            strategy: Strategy::Comprehension,
            cross_file: true,
            full_context: false,
            only_comments: false,
        }
    }
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    #[default]
    Comprehension,
    DefsFirst,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Import,
    Definition,
    Other,
}

/// Why the engine removed a hunk before ordering. Both reasons are stated
/// selections (imports are always dropped; `only_comments` is asked for), so a
/// consumer can always account for the difference between the hunks a file had
/// and the hunks it can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    Import,
    NonComment,
}

/// A hunk that never reached the reading order, with the range it covered — so
/// a caller can tell a hunk that was dropped from one that was never found.
#[derive(Debug, Clone, Serialize)]
pub struct DroppedHunk {
    pub reason: DropReason,
    pub old_range: [usize; 2],
    pub new_range: [usize; 2],
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
    /// true when the file's extension has no tree-sitter grammar, so hunks
    /// carry no structural analysis at all. Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unsupported: bool,
    /// hunks this file had that were dropped before ordering, with the reason.
    /// `hunks.len() + dropped.len()` is what the diff actually produced, so a
    /// consumer can prove nothing went missing silently. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<DroppedHunk>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// P14: an advanced-construct advisory — a powerful/overusable language
/// construct flagged with escalation-ladder guidance. `verdict` = true when a
/// concrete downgrade is suggested (a signal backs it), else informational.
#[derive(Debug, Clone, Serialize)]
pub struct Advisory {
    pub construct: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub verdict: bool,
}

/// What kind of thing an `enclosing` name refers to. Only `Definition` is a
/// declaration a reviewer can navigate to; the rest are *regions* — real
/// containers that hold a hunk and are worth naming, but declare nothing. The
/// distinction matters to a consumer: a region name must never be looked up as
/// a symbol, and must never seed a def→use edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerKind {
    /// a function, class, macro, … — `enclosing` names a real symbol
    Definition,
    /// `describe("…", …)` and friends: a named block, not a declaration
    Test,
    /// a conditional-compilation region, e.g. `#ifdef CURL_DISABLE_HTTP`
    Region,
    /// prose before a document's first heading
    Preamble,
    /// a document's `---` metadata block
    FrontMatter,
    /// a top-level binding whose multi-line value holds the hunk
    Binding,
    /// a top-level call whose multi-line arguments hold the hunk
    Call,
}

/// A defined symbol's identity: name + tree-sitter node kind + enclosing
/// scope. Lets a consumer tell apart same-named symbols across commits (e.g.
/// a method `run` on class `A` vs a module-level function `run`), per the
/// requirement "tree sitter type + scope for the symbol must match, otherwise
/// it's a different symbol".
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Symbol {
    pub name: String,
    /// raw tree-sitter node kind of the defining node, e.g. `function_definition`
    pub kind: String,
    /// qualified enclosing-definition name at the point of definition, or null
    /// at top level (not always the same as the hunk's `enclosing`)
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HunkOut {
    pub id: String,
    pub old_range: [usize; 2],
    pub new_range: [usize; 2],
    pub category: Category,
    pub enclosing: Option<String>,
    /// what `enclosing` names, when it is not a plain definition — a region
    /// that holds the hunk but declares nothing (see `ContainerKind`).
    /// Omitted for a definition, and when there is no enclosing at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enclosing_kind: Option<ContainerKind>,
    pub defines: Vec<String>,
    pub uses: Vec<String>,
    pub group: String,
    pub order_index: usize,
    pub rationale: String,
    /// formatting-only or generated-file hunk — skippable for review (P12.2)
    #[serde(default, skip_serializing_if = "is_false")]
    pub noise: bool,
    /// every changed line is a comment or docstring — drives `--only-comments`
    /// filtering. Omitted when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub comment: bool,
    /// P15: what the hunk did to the members of its enclosing container —
    /// "adds Serve, Watch to Cli". Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
    /// structural smells for a def introduced here (P13.1); omitted when empty
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// advanced-construct advisories in this hunk (P14); omitted when empty
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advisories: Vec<Advisory>,
    /// symbol identity (name + tree-sitter kind + enclosing scope) for each
    /// definition this hunk introduces; omitted when empty
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<Symbol>,
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
