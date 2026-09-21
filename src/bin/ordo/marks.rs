// ---------------------------------------------------------- reviewed-mark persistence
use crate::Item;
use crate::Sources;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Hand-rolled FNV-1a 64-bit. Deliberately not `DefaultHasher` — its output is
/// explicitly unspecified across Rust releases, so a toolchain upgrade would
/// silently invalidate every mark ever written. This is ~10 lines and never
/// changes.
pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// The symbol-identity component of a mark key: every symbol this hunk
/// defines (name + tree-sitter kind + scope — the fields that answer "is
/// this the same symbol"), sorted first so the key never depends on the
/// engine's emission order. Falls back to `enclosing` for a hunk that defines
/// no symbol of its own (a body edit, or a top-level/comment-only hunk), and
/// to the empty string when that's absent too.
///
/// KNOWN LIMITATION: two identical hunks under the same symbol in the same
/// file hash to the same key, so marking one marks both. A positional
/// tiebreak would reintroduce exactly the fragility this key is designed to
/// avoid — a hunk's position shifts whenever anything above it changes, so a
/// position-based key would drop marks on edits that never touched the hunk
/// itself. This case is rare, and its failure is visible (two rows tick
/// together at once) rather than silent.
fn symbol_identity_key(item: &Item) -> String {
    if !item.symbols.is_empty() {
        symbols_identity(&item.symbols)
    } else {
        item.enclosing.clone().unwrap_or_default()
    }
}

/// name + kind + scope for each symbol, order-independent — the identity both
/// `mark_key` and `note_key` are built on.
fn symbols_identity(symbols: &[ordo::model::Symbol]) -> String {
    let mut syms = symbols.to_vec();
    syms.sort();
    syms.iter()
        .map(|s| {
            format!(
                "{}\u{1f}{}\u{1f}{}",
                s.name,
                s.kind,
                s.scope.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\u{1e}")
}

/// Hashes both sides of the hunk's content — old lines, then new — so any
/// change at all (reformat, body edit, signature change) drops the mark. A
/// false "already reviewed" is far worse than a lost one, which is the whole
/// point of covering both sides rather than just the new one. `None` when the
/// item's file content isn't loaded (nothing to hash).
fn hunk_content_hash(item: &Item, sources: &Sources) -> Option<u64> {
    let (ol, nl) = sources.get(&item.path)?;
    let [o0, o1] = item.old_range;
    let [n0, n1] = item.new_range;
    // A side is legitimately empty when its end precedes its start (a pure
    // insert or delete). A *non-empty* side that falls outside the loaded
    // content is stale data, not empty text: hashing it as empty would let two
    // unrelated hunks agree, and a false "already reviewed" is exactly what
    // this hash exists to prevent.
    let side = |lines: &[String], r: [usize; 2]| -> Option<String> {
        let [a, b] = r;
        if a > b {
            return Some(String::new());
        }
        if a < 1 || b > lines.len() {
            return None;
        }
        Some(lines[a - 1..b].iter().map(|l| format!("{l}\n")).collect())
    };
    let (old, new) = (side(ol, [o0, o1])?, side(nl, [n0, n1])?);
    let mut buf = old;
    buf.push('\u{0}');
    buf.push_str(&new);
    Some(fnv1a(buf.as_bytes()))
}

/// The full persistence key: `(rev, path, symbol identity, hunk content
/// hash)`, folded into one FNV-1a hash so the file on disk records only an
/// opaque number — never a path, a symbol name, or source text. `None` when
/// there's nothing to hash the content from.
pub(super) fn mark_key(rev: &str, item: &Item, sources: &Sources) -> Option<u64> {
    let content_hash = hunk_content_hash(item, sources)?;
    let sym_key = symbol_identity_key(item);
    let combined = format!("{rev}\u{0}{}\u{0}{sym_key}\u{0}{content_hash:x}", item.path);
    Some(fnv1a(combined.as_bytes()))
}

/// A review note's key: the symbol's identity and **nothing else**.
///
/// The exact opposite of `mark_key`, and deliberately so. A mark folds in the
/// revision, the path and a hash of both sides of the hunk, because a stale
/// "already reviewed" is worse than a lost one. A note is a thought about a
/// *symbol* — it must outlive a rebase (no revision), a move to another file
/// (no path) and an edit to the body (no content hash). Renames are handled by
/// migrating the old identity's key when the ledger reports one, so the note
/// follows the symbol through its new name too.
///
/// `None` when the hunk declares no symbol: there is nothing stable to anchor
/// to, and anchoring to the enclosing name would silently drift.
pub(super) fn note_key(item: &Item) -> Option<u64> {
    if item.symbols.is_empty() {
        return None;
    }
    Some(fnv1a(symbols_identity(&item.symbols).as_bytes()))
}

/// The key a symbol *would* have had under its previous name, so a note
/// written before a rename can be carried across to it.
pub(super) fn note_key_named(item: &Item, old_name: &str) -> Option<u64> {
    if item.symbols.is_empty() {
        return None;
    }
    let mut renamed = item.symbols.clone();
    for sym in &mut renamed {
        sym.name = old_name.to_string();
    }
    Some(fnv1a(symbols_identity(&renamed).as_bytes()))
}

pub(super) const MARK_TTL_SECS: u64 = 90 * 24 * 60 * 60;

pub(super) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_home() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        let p = PathBuf::from(x.trim());
        // the XDG spec says a relative value is invalid and must be ignored;
        // honouring one would scatter cache directories through the cwd
        if p.is_absolute() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;
    (!home.trim().is_empty()).then(|| PathBuf::from(home).join(".cache"))
}

/// `${XDG_CACHE_HOME:-$HOME/.cache}/ordo/reviewed/<repo>.json` — cache, not
/// `.git/`: derived state, safe to lose, and nothing a user could accidentally
/// commit. `<repo>` is the repo's toplevel path hashed rather than written
/// verbatim, so the filename itself gives nothing away either. `None` when
/// neither env var resolves — callers then just skip persistence.
pub(super) fn marks_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("reviewed");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

/// What one hunk looked like the last time this review was opened: what it
/// contained, where it sat in the reading order, and what it depended on.
#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct Snap {
    /// hex hash of both sides of the hunk
    c: String,
    /// its position in the reading order
    pub(super) p: usize,
    /// keys of the hunks defining what it uses
    pub(super) d: Vec<String>,
}

/// How a hunk differs from the last time this review was opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Delta {
    /// not in the previous run at all
    New,
    /// same symbol, different content
    Changed,
    /// **byte-identical, but it reads in a different place now** — because
    /// what it depends on changed. No other tool reports this: a diff sees
    /// nothing, so the hunk looks untouched while the reason to read it moved.
    Moved,
    /// same content, same position, same dependencies
    Same,
}

/// A hunk's identity across runs: its symbol, and the file it lives in. Not
/// the content and not the revision — those are what the delta is measuring.
fn snap_key(item: &Item) -> String {
    format!(
        "{:016x}",
        fnv1a(format!("{}\u{0}{}", item.path, symbol_identity_key(item)).as_bytes())
    )
}

/// `${XDG_CACHE_HOME:-$HOME/.cache}/ordo/runs/<repo>.json` — the previous run,
/// so the next one can say what moved. Overwritten each time the review is
/// opened, so a delta always answers "since I last looked".
pub(super) fn runs_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("runs");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

pub(super) fn load_snaps(path: &Path) -> HashMap<String, Snap> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(super) fn save_snaps(path: &Path, snaps: &HashMap<String, Snap>) {
    if let Ok(text) = serde_json::to_string(snaps) {
        // best-effort: a mark that cannot be persisted still works this session
        let _ = write_atomic(path, &text);
    }
}

/// Write through a temporary file in the same directory and rename it over the
/// target, so the file on disk is always either the previous contents or the
/// new ones. `std::fs::write` truncates in place: a process killed mid-write
/// leaves an empty or half-written file, which for marks means losing every
/// mark ever made rather than the ones from this session. These are written on
/// every toggle, so that window is open constantly.
///
/// Best-effort throughout — a read-only filesystem, a missing `$HOME` or a
/// failed rename all just mean the state is not persisted. A lost mark is the
/// accepted cost; a blocked review is not.
pub(super) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("no parent directory"));
    };
    std::fs::create_dir_all(parent)?;
    // same directory, so the rename stays within one filesystem; the pid keeps
    // two ordo processes on one repo from writing the same temporary
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        std::process::id()
    ));
    if let Err(e) = std::fs::write(&tmp, text) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// This run's snapshot, and how each item differs from the stored one.
pub(super) fn compare_runs(
    items: &[Item],
    order: &[usize],
    sources: &Sources,
    prev: &HashMap<String, Snap>,
) -> (HashMap<String, Snap>, Vec<Delta>) {
    let key_of: Vec<String> = items.iter().map(snap_key).collect();
    let pos_of: HashMap<usize, usize> = order.iter().enumerate().map(|(p, &i)| (i, p)).collect();
    let mut now: HashMap<String, Snap> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        let deps: Vec<String> = it
            .edges
            .iter()
            .filter(|e| e.dependency)
            .filter_map(|e| e.target)
            .map(|t| key_of[t].clone())
            .collect();
        now.insert(
            key_of[i].clone(),
            Snap {
                c: format!("{:016x}", hunk_content_hash(it, sources).unwrap_or(0)),
                p: pos_of.get(&i).copied().unwrap_or(usize::MAX),
                d: deps,
            },
        );
    }
    let deltas = items
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let k = &key_of[i];
            let (Some(was), Some(is)) = (prev.get(k), now.get(k)) else {
                return Delta::New;
            };
            if was.c != is.c {
                Delta::Changed
            } else if was.p != is.p || was.d != is.d {
                Delta::Moved
            } else {
                Delta::Same
            }
        })
        .collect();
    (now, deltas)
}

/// `${XDG_CACHE_HOME:-$HOME/.cache}/ordo/notes/<repo>.json` — beside the marks,
/// same reasoning about the cache and the hashed repo name. Kept in its own
/// file because a note outlives the mark on the same hunk: marks expire with
/// the content, notes follow the symbol.
pub(super) fn notes_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("notes");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

/// Reads the note file: hex key -> note text. Degrades to "no notes" on any
/// failure, exactly as the marks do — the cache is a convenience.
pub(super) fn load_notes(path: &Path) -> HashMap<u64, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<HashMap<String, String>>(&text) else {
        return HashMap::new();
    };
    raw.into_iter()
        .filter_map(|(k, v)| u64::from_str_radix(&k, 16).ok().map(|k| (k, v)))
        .collect()
}

pub(super) fn save_notes(path: &Path, notes: &HashMap<u64, String>) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let body: HashMap<String, String> = notes
        .iter()
        .map(|(k, v)| (format!("{k:016x}"), v.clone()))
        .collect();
    if let Ok(text) = serde_json::to_string(&body) {
        let _ = std::fs::write(path, text);
    }
}

/// Reads the mark file: hex key -> unix-seconds written. Any failure at all —
/// missing file, unreadable, corrupt JSON — degrades to "no marks" rather
/// than a crash or a blocked UI; the cache is a convenience, the review is
/// the product.
pub(super) fn load_marks(path: &Path) -> HashMap<u64, u64> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<HashMap<String, u64>>(&text) else {
        return HashMap::new();
    };
    raw.into_iter()
        .filter_map(|(k, v)| u64::from_str_radix(&k, 16).ok().map(|k| (k, v)))
        .collect()
}

/// Drops anything older than `MARK_TTL_SECS` so the file doesn't grow
/// unbounded across every repo/review a user ever runs.
pub(super) fn prune_marks(marks: &mut HashMap<u64, u64>, now: u64) {
    marks.retain(|_, ts| now.saturating_sub(*ts) <= MARK_TTL_SECS);
}

/// The files are tiny, so this runs on every toggle rather than only at quit,
/// and a panic or killed terminal never loses the session's marks — which is
/// only true because the write is atomic (see `write_atomic`).
pub(super) fn save_marks(path: &Path, marks: &HashMap<u64, u64>) {
    let body: HashMap<String, u64> = marks
        .iter()
        .map(|(k, v)| (format!("{k:016x}"), *v))
        .collect();
    if let Ok(text) = serde_json::to_string(&body) {
        // best-effort: a mark that cannot be persisted still works this session
        let _ = write_atomic(path, &text);
    }
}
