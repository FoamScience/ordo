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
    /// Reviewing rules: the caller's own conventions, evaluated against the
    /// facts the engine already computes. Empty by default — the engine has no
    /// rules of its own, and reads no config (a client collects them; see
    /// `Rule`).
    #[serde(default)]
    pub rules: Vec<Rule>,
}
impl Default for Options {
    fn default() -> Self {
        Options {
            strategy: Strategy::Comprehension,
            cross_file: true,
            full_context: false,
            only_comments: false,
            rules: vec![],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Import,
    Definition,
    Other,
}

/// Why the engine removed a hunk before ordering — a selection the caller
/// *asked* for, so the difference between the hunks a file had and the hunks it
/// can see is always accountable. An import is no longer among these: it is
/// kept and marked `noise`, because dropping it hid new dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
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
    /// what the engine could not make sense of in the caller's own input — a
    /// rule whose glob or query does not compile, reported rather than silently
    /// never matching. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// P23.1: one entry per *symbol* the change touches, rather than per hunk
    /// — what a reviewer reasons about. A projection of data the engine already
    /// has; see `Output.files` for the hunks each entry's `used_by` names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ledger: Vec<LedgerEntry>,
    /// P13.2: change-shape signals about the changeset as a whole, as facts
    /// rather than judgments — `code changed but no test touched`,
    /// `a.py: 14 hunks (high churn)`. Per-hunk signals live on `hunks[].notes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
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

/// One reviewing rule: what to match, and what to say or do about it.
///
/// A rule is *data*, evaluated deterministically against a hunk's own facts —
/// there is no rule runtime, nothing is executed, and the same input always
/// produces the same output. The engine never reads a rule from disk: a client
/// collects them (per-user, per-repo) and passes them in `Options.rules`, which
/// keeps `ordo order` a function of its arguments.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    /// how the rule identifies itself in `hunks[].rules` — a short slug
    pub name: String,
    #[serde(default)]
    pub when: When,
    /// something worth knowing about this hunk
    #[serde(default)]
    pub note: Option<String>,
    /// something worth stopping at — reported at `warn` level
    #[serde(default)]
    pub warn: Option<String>,
    /// treat a matching hunk as skippable (the caller's own noise policy, on
    /// top of the engine's formatting/generated detection)
    #[serde(default)]
    pub noise: bool,
    /// Ordering influence. Higher sorts earlier, but **only among hunks the
    /// dependency graph has already freed**: priority replaces the file-position
    /// tiebreaker, it never reorders a definition after its use. A preference
    /// cannot break P2.
    #[serde(default)]
    pub priority: i64,
}

/// A rule's conditions. Every field given must hold (they are ANDed); a rule
/// with no conditions matches every hunk, which is occasionally what you want
/// (a whole-changeset note) and otherwise a mistake the rule's own name makes
/// obvious.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct When {
    /// glob against the file path, e.g. `src/security/**`
    #[serde(default)]
    pub path: Option<String>,
    /// language name as `src/lang.rs` knows it (`python`, `cpp`, `markdown`, …)
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub category: Option<Category>,
    /// what holds the hunk; `definition` matches a plain definition
    #[serde(default)]
    pub enclosing_kind: Option<String>,
    /// glob against any name the hunk defines / uses / imports
    #[serde(default)]
    pub defines: Option<String>,
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub imports: Option<String>,
    /// require (or forbid) the engine's own noise / comment classification
    #[serde(default)]
    pub noise: Option<bool>,
    #[serde(default)]
    pub comment: Option<bool>,
    /// a tree-sitter query over the hunk's own lines — the pattern half of a
    /// rule, for conventions that are about code shape rather than about paths
    /// and names (see `docs/rules.md`). The query source itself, not a path:
    /// the engine reads no files.
    #[serde(default)]
    pub query: Option<String>,

    // ---- introduced shapes: fires when the hunk introduces a node …
    /// … of one of these kinds
    #[serde(default, deserialize_with = "string_or_vec")]
    pub kind: Option<Vec<String>>,
    /// … whose direct children include each of these — a named kind, or a
    /// keyword token such as `virtual`
    #[serde(default, deserialize_with = "string_or_vec")]
    pub with: Option<Vec<String>>,
    /// … and none of these. Absence, as a table entry
    #[serde(default, deserialize_with = "string_or_vec")]
    pub without: Option<Vec<String>>,
    /// … and whose text matches / does not match this regex
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub text_not: Option<String>,
    /// glob the file path must NOT match — third-party code, a framework carve-out
    #[serde(default)]
    pub path_not: Option<String>,

    // ---- limits: fires when a definition the hunk introduces exceeds one
    #[serde(default)]
    pub max_params: Option<usize>,
    #[serde(default)]
    pub max_lines: Option<usize>,
    /// deepest control-flow nesting any row of the hunk sits at
    #[serde(default)]
    pub max_nesting: Option<usize>,
    /// fires on the hunks of a file this change pushed past the limit
    #[serde(default)]
    pub max_file_lines: Option<usize>,

    // ---- relationships the engine already knows
    /// a definition starting in the hunk calls itself
    #[serde(default)]
    pub recursive: Option<bool>,
    /// glob against the member names of the container the hunk defines into
    #[serde(default)]
    pub container_with: Option<String>,
    #[serde(default)]
    pub container_without: Option<String>,
    /// the hunk adds a data member that nothing in this change initializes
    #[serde(default)]
    pub member_uninitialized: Option<bool>,
}

/// `kind = "x"` and `kind = ["x", "y"]` both read; a one-entry list is the
/// common case and shouldn't need brackets.
fn string_or_vec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<V>::deserialize(d)? {
        None => None,
        Some(V::One(s)) => Some(vec![s]),
        Some(V::Many(v)) => Some(v),
    })
}

/// A rule that matched, on the hunk it matched.
#[derive(Debug, Clone, Serialize)]
pub struct RuleHit {
    pub rule: String,
    pub message: String,
    /// `note` | `warn`
    pub level: &'static str,
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
    /// one `---` document of a multi-document file. Unlike every other region
    /// this one *scopes*: its name joins the path of what it contains, because
    /// two documents' top-level keys are genuinely different things.
    Document,
    /// a top-level binding whose multi-line value holds the hunk
    Binding,
    /// a top-level call whose multi-line arguments hold the hunk
    Call,
}

/// What happened to a symbol across the whole change. Ordered from "this is
/// new code" to "this already existed and only its body moved", which is also
/// roughly the order a reviewer cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolChange {
    Added,
    /// arrived from another file — `from` is that path
    Moved,
    /// pulled out of a definition that is still present — `from` is that name
    Extracted,
    /// `from` is the name it had before
    Renamed,
    /// existed already; its declaration line changed
    Signature,
    /// existed already; only its body did
    Body,
    Removed,
}

/// One symbol's line in the change ledger (P23.1). A forty-hunk diff has a
/// twelve-line ledger, read before any hunk.
#[derive(Debug, Clone, Serialize)]
pub struct LedgerEntry {
    pub name: String,
    /// raw tree-sitter kind of the defining node; absent for a removed symbol,
    /// whose defining node no longer exists to be asked
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub path: String,
    /// id of the hunk this entry is anchored to — the one that defines it, or
    /// deletes it. A ledger line that cannot point at a hunk is not actionable,
    /// so every entry has one.
    pub at: String,
    pub change: SymbolChange,
    /// the old name, or the file/definition it came from — meaning depends on
    /// `change`, and it is absent for the kinds that have no source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// ids of the hunks in this change that *use* it — the fan-in. Empty is
    /// itself a signal: a symbol added and used nowhere in the change.
    pub used_by: Vec<String>,
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
    /// reviewing rules (`Options.rules`) that matched this hunk; omitted when
    /// none did
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleHit>,
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
