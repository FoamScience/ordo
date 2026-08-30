//! ordo-tui — a terminal reviewer that is a pure client of the ordo engine.
//! It owns git (shells out for a commit's blobs), calls `ordo::run`, and renders
//! the change in comprehension order: the full file with the changed hunk
//! highlighted in context, plus rationale, advisories and def→use edges. The
//! engine stays git-free; gated behind the `tui` feature so the default build
//! never pulls a UI stack.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ordo::model::{Change, HunkOut, Input, Options, Output, Strategy, Symbol};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use tree_sitter::{Node, Parser, Point, Tree};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const PAGE: u16 = 15;

const USAGE: &str = "\
ordo-tui — interactive review of a commit, ordered for comprehension.

usage:
  ordo-tui [<rev>] [<glob>...] [--keys <preset>] [--theme <dark|light>] [--all] [--only-comments]
  ordo-tui --help
  ordo-tui --version

<rev> is any git commit-ish (a sha, HEAD~2, a tag), a commit range (main..branch,
or main...branch to diff from the merge base), or `zz` for the uncommitted area.
base..zz (or base...zz for the merge base of base and HEAD) reviews everything
done since base including uncommitted work — base's tree versus the worktree.
Default: HEAD. Its diff is ordered by def→use with rationale, structural notes,
advisories and PR-split clusters.

On a GitButler-managed repo <rev> also takes the CLI IDs `but status` prints: a
branch (by ID or name) reviews that branch's own commits, a commit ID (or a
change-ID prefix) reviews that commit.

Generated and lock files (Cargo.lock, package-lock.json, vendor/, node_modules/,
.min.js, …) are skipped, as is anything .gitattributes marks `linguist-generated`
or `-diff`; --all keeps them all. Any <glob> after <rev> confines the
review to paths matching at least one of them — `*` crosses `/`, so `src/*` is
everything under src/ and `*.rs` matches at any depth. A glob prefixed `!` is
negative and excludes a path that matches it, e.g. `'src/*' '!tests/*'`; with
only negative globs given, everything except those is kept. `\\!literal` escapes
a leading bang for a path genuinely named that way. Matching is order-independent
and deliberately unlike .gitignore: any negative match excludes a path no matter
where it appears relative to the positives, and a later positive never
re-includes something a negative excluded. Quote them so the shell doesn't
expand them first.

--theme selects the diff/selection background palette (also read from
$ORDO_TUI_THEME, default dark) — dark or light, matched to a dark or light
terminal background. Only the five hardcoded backgrounds change; every
foreground colour already follows the terminal's own palette.

--only-comments limits the review to comment/docstring-only hunks (ordered
among themselves, as the engine's --only-comments does) — a starting point
only; `:only-comments` in command mode toggles it live, same as `--all`
and a path filter (see below).

--keys selects a keystroke preset (also read from $ORDO_TUI_KEYS, default vim).
The three panes — reading order, code, why — take focus one at a time; motion
keys act on the focused pane, paging always drives the code pane.

Command mode (`:` in vim, Ctrl+Shift+P in vscode) turns launch-time choices
into live controls: `:only-comments`, `:all`, `:filter <glob>` (empty clears,
same negative-glob syntax as the CLI), `:keys <preset>`, `:strategy
<comprehension|defs-first|file>` (re-orders in place), `:group` (toggle group
headers in the reading-order list), `:goto <path>`, `:e <rev>` (review a
different revision without restarting), `:q`, `:help` (lists these, generated
from the same table `:help`'s popup shows). Typing completes command names,
then — after a space — each command's own arguments (paths, presets, strategy
names, revisions); Tab/C-n/Down cycle the menu forward, C-p/Up back, Enter
accepts the highlighted entry or runs the line when nothing is highlighted,
Esc cancels.

  vim      j/k move · C-w C-w (or C-w h/j/k/l) pane · gg/G ends · C-d/C-u half
           space/f/C-f, C-b page · x toggle reviewed · q / Esc quit · ?
           opens a keybinding help popup (Esc/q close it without quitting)
           : opens the command bar (see above)
           in the code pane: h/l/w/b/e/0/$/{/} move the cursor, zh/zl scroll
           the pane horizontally without moving it, K shows the symbol under
           it (Esc/q close that popup without quitting)
           / opens a text-search prompt (Enter jumps to the first match at or
           after the cursor, Esc cancels, Backspace edits); * and # jump to
           the next/previous occurrence of the symbol under the cursor,
           resolved via tree-sitter identifiers rather than text; n/N cycle
           whichever search — text or symbol — is currently active, wrapping
           ge opens the selected hunk's file in $VISUAL/$EDITOR at its line
           (a chord, not bare `e`, which is already word-end in the code pane)
           in the why pane: j/k move a line cursor over reason/details/dep
           lines; on a `dep` (def→use edge) line, K previews the referenced
           hunk and gd (or Enter) jumps to it and focuses the code pane; C-o
           jumps back. A dep line whose hunk isn't part of this review (a
           glob filter, --only-comments, or an unreviewed file) previews as
           such and gd/Enter does nothing rather than guessing where to go
  vscode   up/down move · F6 / shift-F6 pane · C-1/C-2/C-3 pane · PageUp/PageDown
           page · space / enter toggle reviewed · C-q / Esc quit · F1 opens a
           keybinding help popup (Esc closes it without quitting)
           C-Shift-P opens the command bar (vscode's own command-palette key;
           C-P, its \"Go to File\", opens the bar pre-filled with `goto `)
           in the code pane: left/right/C-left/C-right/Home/End move the
           cursor, shift-left/shift-right scroll the pane horizontally
           without moving it, F12 shows the symbol under it (Esc closes that
           popup)
           C-f opens the text-search prompt, F3/shift-F3 cycle its matches;
           C-F12/shift-C-F12 jump to the next/previous occurrence of the
           symbol under the cursor (same tree-sitter search as vim's */#)
           C-o opens the selected hunk's file in $VISUAL/$EDITOR at its line
           in the why pane: up/down move a line cursor over reason/details/dep
           lines; on a `dep` (def→use edge) line, F12 previews the referenced
           hunk and C-Enter jumps to it and focuses the code pane; Alt-Left
           (vscode's own \"Go Back\") jumps back. A dep line whose hunk isn't
           part of this review previews as such and C-Enter does nothing
           rather than guessing where to go

Long lines are clipped, not wrapped, so the line-number gutter stays put; a
‹/› marker appears in the code pane's title whenever the current file has
content scrolled past the left/right edge.
";

// A path filter's compiled matcher: an optional include set (empty/absent
// means "everything") and an optional exclude set. A path is kept when it
// matches at least one include pattern (or none were given) AND matches no
// exclude pattern.
//
// Deliberately unlike `.gitignore`: matching is order-independent. `.gitignore`
// lets a later pattern re-include something an earlier one excluded (the last
// match wins); here any negative match excludes a path, full stop, regardless
// of where it appeared relative to the positives on the command line. This is
// a real, deliberate difference from what a shell-glob-literate user might
// expect from `!pattern`, so it's worth restating at every place a glob list
// turns into a `PathGlobs` — see `build_globs` below, where the patterns are
// actually parsed.
#[derive(Clone, Default)]
struct PathGlobs {
    include: Option<globset::GlobSet>,
    exclude: Option<globset::GlobSet>,
}

impl PathGlobs {
    fn is_match(&self, path: &str) -> bool {
        let included = self.include.as_ref().is_none_or(|g| g.is_match(path));
        let excluded = self.exclude.as_ref().is_some_and(|g| g.is_match(path));
        included && !excluded
    }

    fn is_empty(&self) -> bool {
        self.include.is_none() && self.exclude.is_none()
    }
}

// Which changed files reach the engine. Generated and lock files are dropped
// before their blobs are even read — parsing a lock file costs more than the
// review it would add. Globs, when given, keep a path that matches at least
// one positive pattern (or there are none) and no negative one.
/// What never reached the screen, and why — the numbers `:audit` reports.
/// File-level counts are filled in by `Filter::apply` and the gather functions;
/// hunk-level ones come from the engine's own `dropped` record. Anything hidden
/// that no field here explains is a bug, and `:audit` says so rather than
/// quietly rounding it away.
#[derive(Clone, Copy, Default)]
struct Ledger {
    /// paths git reported changed, before any client-side filtering
    files_seen: usize,
    /// dropped by the built-in generated/lock set
    files_generated: usize,
    /// dropped because the repo's own .gitattributes declares them generated
    files_declared: usize,
    /// dropped by a path glob (positive miss or negative match)
    files_globbed: usize,
    /// listed as changed but unreadable, so never sent to the engine
    files_unreadable: usize,
    /// hunks the engine dropped as pure imports
    hunks_import: usize,
    /// hunks the engine dropped under `only_comments`
    hunks_non_comment: usize,
}

#[derive(Clone)]
struct Filter {
    globs: PathGlobs,
    skip_generated: bool,
    /// set by `apply`: whether every path that matched a positive pattern was
    /// then excluded by a negative one — `note`'s only use for it, so the
    /// "nothing to review" message can say *why* rather than just *that*.
    negatives_emptied: std::cell::Cell<bool>,
    /// file-level accounting, filled in by `apply` (and by the gather that owns
    /// this filter, for unreadable paths) — read once the load is done.
    tally: std::cell::Cell<Ledger>,
}

/// Why a path never reached the engine — one variant per file-level count in
/// `Ledger`, so the tally and the decision can't drift apart.
enum FileDrop {
    Generated,
    Globbed,
}

impl Filter {
    /// The drop decision itself: `None` keeps the path.
    fn reject(&self, path: &str) -> Option<FileDrop> {
        if self.skip_generated && ordo::is_generated_path(path) {
            return Some(FileDrop::Generated);
        }
        (!self.globs.is_match(path)).then_some(FileDrop::Globbed)
    }

    /// `reject` read as a predicate — the glob tests' vocabulary.
    #[cfg(test)]
    fn keep(&self, path: &str) -> bool {
        self.reject(path).is_none()
    }

    /// Winnow a change list: globs and the built-in generated set first, then one
    /// `git check-attr` for whatever the repo declares generated itself. That
    /// declaration decides the cases a path can't — a committed `dist/`, a
    /// snapshot directory, a tool-written changelog.
    fn apply(&self, paths: Vec<String>) -> Vec<String> {
        let any_positive = paths
            .iter()
            .any(|p| self.globs.include.as_ref().is_none_or(|g| g.is_match(p)));
        let any_survives = paths.iter().any(|p| self.globs.is_match(p));
        self.negatives_emptied.set(any_positive && !any_survives && !paths.is_empty());
        let mut tally = self.tally.get();
        tally.files_seen += paths.len();
        let mut kept: Vec<String> = vec![];
        for p in paths {
            match self.reject(&p) {
                Some(FileDrop::Generated) => tally.files_generated += 1,
                Some(FileDrop::Globbed) => tally.files_globbed += 1,
                None => kept.push(p),
            }
        }
        if !self.skip_generated || kept.is_empty() {
            self.tally.set(tally);
            return kept;
        }
        let declared = declared_generated(&kept);
        let before = kept.len();
        kept.retain(|p| !declared.contains(p));
        tally.files_declared += before - kept.len();
        self.tally.set(tally);
        kept
    }

    /// Record a path that was listed as changed but could not be read, so it is
    /// accounted for rather than silently absent (`gather_worktree_range`).
    fn note_unreadable(&self) {
        let mut t = self.tally.get();
        t.files_unreadable += 1;
        self.tally.set(t);
    }

    // so an empty review doesn't read as "no changes" when it's the filter
    fn note(&self) -> &'static str {
        if !self.globs.is_empty() {
            return if self.negatives_emptied.get() {
                " (every matching path was excluded by a negative glob)"
            } else {
                " matching the given globs"
            };
        }
        if self.skip_generated {
            " (generated and lock files are skipped; --all keeps them)"
        } else {
            ""
        }
    }
}

// `*` deliberately crosses `/`, so `src/*` reads as everything under src/ and
// `*.rs` matches at any depth — what a shell-shaped filter is expected to mean
// here, and the shell itself never gets to expand these against the worktree.
//
// A leading `!` makes a pattern negative (exclude rather than include);
// `\!literal` escapes a leading bang for a path genuinely starting with one.
// Matching against the whole set is order-independent (see `PathGlobs`) —
// unlike `.gitignore`, a negative here can never be re-included by a later
// positive pattern.
fn build_globs(pats: &[String]) -> Result<PathGlobs, String> {
    let mut inc = globset::GlobSetBuilder::new();
    let mut exc = globset::GlobSetBuilder::new();
    let (mut any_inc, mut any_exc) = (false, false);
    for p in pats {
        // a leading `!` makes the pattern negative; `\!literal` escapes it —
        // stripping only the backslash, so the glob still starts with the
        // literal `!` character rather than losing it
        let (negative, pat): (bool, &str) = if let Some(rest) = p.strip_prefix('!') {
            (true, rest)
        } else if let Some(rest) = p.strip_prefix('\\') {
            (false, rest)
        } else {
            (false, p.as_str())
        };
        if pat.is_empty() {
            return Err(format!("bad glob '{p}': empty pattern"));
        }
        let g = globset::GlobBuilder::new(pat)
            .literal_separator(false)
            .build()
            .map_err(|e| format!("bad glob '{p}': {e}"))?;
        if negative {
            exc.add(g);
            any_exc = true;
        } else {
            inc.add(g);
            any_inc = true;
        }
    }
    let include = any_inc.then(|| inc.build()).transpose().map_err(|e| e.to_string())?;
    let exclude = any_exc.then(|| exc.build()).transpose().map_err(|e| e.to_string())?;
    Ok(PathGlobs { include, exclude })
}

fn parse_args() -> Result<(String, Keymap, Filter, bool, Theme), i32> {
    let mut rev: Option<String> = None;
    let mut globs: Vec<String> = vec![];
    let mut skip_generated = true;
    let mut only_comments = false;
    let mut preset = std::env::var("ORDO_TUI_KEYS").unwrap_or_else(|_| "vim".to_string());
    let mut theme_name = std::env::var("ORDO_TUI_THEME").unwrap_or_else(|_| "dark".to_string());
    let mut want_preset = false;
    let mut want_theme = false;
    for a in std::env::args().skip(1) {
        if want_preset {
            preset = a;
            want_preset = false;
            continue;
        }
        if want_theme {
            theme_name = a;
            want_theme = false;
            continue;
        }
        match a.as_str() {
            "--keys" => want_preset = true,
            s if s.starts_with("--keys=") => preset = s["--keys=".len()..].to_string(),
            "--theme" => want_theme = true,
            s if s.starts_with("--theme=") => theme_name = s["--theme=".len()..].to_string(),
            "--all" => skip_generated = false,
            "--only-comments" => only_comments = true,
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                return Err(0);
            }
            "-V" | "--version" | "version" => {
                println!(
                    "ordo-tui {} (ordo schema {})",
                    env!("CARGO_PKG_VERSION"),
                    ordo::SCHEMA_VERSION
                );
                return Err(0);
            }
            s if s.starts_with('-') => {
                eprintln!("ordo-tui: unknown flag '{s}'\n\n{USAGE}");
                return Err(2);
            }
            s if rev.is_some() => globs.push(s.to_string()),
            s => rev = Some(s.to_string()),
        }
    }
    if want_preset {
        eprintln!("ordo-tui: --keys needs a preset name\n\n{USAGE}");
        return Err(2);
    }
    if want_theme {
        eprintln!("ordo-tui: --theme needs a value\n\n{USAGE}");
        return Err(2);
    }
    let Some(keys) = keymap(&preset) else {
        eprintln!("ordo-tui: unknown key preset '{preset}' (want: vim, vscode)");
        return Err(2);
    };
    let Some(theme) = theme(&theme_name) else {
        eprintln!("ordo-tui: unknown theme '{theme_name}' (want: dark, light)");
        return Err(2);
    };
    let globs = build_globs(&globs).map_err(|e| {
        eprintln!("ordo-tui: {e}");
        2
    })?;
    let filter = Filter {
        globs,
        skip_generated,
        negatives_emptied: std::cell::Cell::new(false),
        tally: std::cell::Cell::new(Ledger::default()),
    };
    Ok((
        rev.unwrap_or_else(|| "HEAD".to_string()),
        keys,
        filter,
        only_comments,
        theme,
    ))
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

// one-based "(i/total)" progress messages for the load stages that iterate
// per item — pulled out of `gather_range`/`gather_uncommitted`/`load` so the
// counter arithmetic and wording are unit-testable without a worker thread
fn read_progress(path: &str, i: usize, total: usize) -> String {
    format!("reading {path} ({}/{total})", i + 1)
}

fn highlight_progress(i: usize, total: usize) -> String {
    format!("highlighting ({}/{total})", i + 1)
}

fn main() -> std::io::Result<()> {
    let (rev, keys, filter, only_comments, theme) = match parse_args() {
        Ok(v) => v,
        Err(code) => std::process::exit(code),
    };
    let Some(target) = resolve(&rev) else {
        eprintln!(
            "ordo-tui: '{rev}' is not a git revision, a commit range or a \
             GitButler CLI ID (see `but status`)"
        );
        std::process::exit(1);
    };
    // the single commit the `K` popup's history section reviews from — a
    // range's tip, or HEAD standing in for the uncommitted area (`zz`)
    let review_sha = review_commit_sha(&target);
    let uncommitted = matches!(target, Target::Uncommitted | Target::WorktreeRange(_));
    run(rev, keys, target, filter, only_comments, review_sha, uncommitted, theme)
}

/// What the worker thread reports back over the channel while `gather`,
/// `highlight_file` and `ordo::run` do their work off the draw loop.
enum LoadMsg {
    /// one line of progress, replacing whatever was shown before
    Progress(String),
    /// `build_items` came back empty — the message `main` used to print
    /// before taking the screen
    Empty(String),
    /// boxed, like `State::Ready` — keeps `LoadMsg` itself small regardless of
    /// how much a load result carries
    Done(Box<LoadResult>),
}

struct LoadResult {
    items: Vec<Item>,
    /// indices into `items` visible under the launch-time `--only-comments`
    /// state (`:only-comments`, `:all` and `:filter` narrow/widen this live
    /// from here — see `compute_view`)
    view: Vec<usize>,
    comments_only: bool,
    sources: Sources,
    highlights: Highlights,
    /// the same timing line `main` used to print before init, now printed
    /// after `ratatui::restore()` so it still lands on the shell's
    /// scrollback
    timing: String,
    /// `reviewed[i]` set from a persisted mark whose key matches item `i`
    /// (see `mark_key`) — loading is here, off the draw loop, alongside
    /// the rest of the load work
    reviewed: Vec<bool>,
    /// where a toggle gets persisted; `None` when the repo root or the
    /// cache directory couldn't be resolved, in which case marks just
    /// live for this session and are never written
    marks_path: Option<PathBuf>,
    marks: HashMap<u64, u64>,
    /// group id -> reason, for `:group`'s header rows
    groups: HashMap<String, String>,
    /// what was dropped on the way here, for `:audit`
    ledger: Ledger,
}

/// group id -> the engine's `Group::reason`, for `:group`'s header rows.
fn group_reasons(out: &Output) -> HashMap<String, String> {
    out.groups.iter().map(|g| (g.id.clone(), g.reason.clone())).collect()
}

/// Everything `gather`/`highlight_file`/`ordo::run` need, run off the draw
/// loop. `ordo::run` itself can `eprintln!` when a change is degraded to
/// positional order — unreachable here, since every `Change` this file
/// builds always sets `new` (see `gather_range`/`gather_uncommitted`), which
/// is the one branch in `build_change` that never degrades.
fn load(target: Target, filter: Filter, only_comments: bool, rev: String, tx: mpsc::Sender<LoadMsg>) {
    let progress = |msg: String| {
        let _ = tx.send(LoadMsg::Progress(msg));
    };
    let t = std::time::Instant::now();
    let input = match target {
        Target::Commit(sha) => gather(&sha, &filter, &progress),
        Target::Range(base, tip) => gather_range(&base, &tip, &filter, &progress),
        Target::Uncommitted => gather_uncommitted(&filter, &progress),
        Target::WorktreeRange(base) => gather_worktree_range(&base, &filter, &progress),
    };
    // `only_comments` becomes the starting state of the live `:only-comments`
    // filter (see `compute_view`) rather than an engine option — the engine
    // always sees every hunk, so toggling it back off has something to show.
    let (read_ms, files) = (t.elapsed().as_millis(), input.changes.len());
    let t = std::time::Instant::now();
    let sources: Sources = input
        .changes
        .iter()
        .map(|c| {
            let split = |s: Option<&String>| {
                s.map(|t| t.lines().map(String::from).collect())
                    .unwrap_or_default()
            };
            (
                c.path.clone(),
                (split(c.old.as_ref()), split(c.new.as_ref())),
            )
        })
        .collect();
    // syntax highlight each new file once, up front (indexed by path)
    let candidates: Vec<&Change> = input.changes.iter().filter(|c| c.new.is_some()).collect();
    let total = candidates.len();
    let mut highlights: Highlights = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        progress(highlight_progress(i, total));
        if let Some(h) = highlight_file(&c.path, c.new.as_ref().unwrap()) {
            highlights.insert(c.path.clone(), h);
        }
    }
    let (hl_ms, hl_files) = (t.elapsed().as_millis(), highlights.len());
    let t = std::time::Instant::now();
    progress("ordering…".to_string());
    let out = ordo::run(input);
    // the engine records what it dropped and why; fold it into the same ledger
    // the path filter has been filling in, so `:audit` reads one set of numbers
    let mut ledger = filter.tally.get();
    for d in out.files.iter().flat_map(|f| f.dropped.iter()) {
        match d.reason {
            ordo::model::DropReason::Import => ledger.hunks_import += 1,
            ordo::model::DropReason::NonComment => ledger.hunks_non_comment += 1,
        }
    }
    let mut items = build_items(&out);
    refine_items(&mut items, &sources);
    let groups = group_reasons(&out);
    let view = compute_view(&items, only_comments, true, None);
    if view.is_empty() {
        let msg = if only_comments {
            format!("ordo-tui: nothing to review in {rev} — no comment changes{}", filter.note())
        } else {
            format!("ordo-tui: nothing to review in {rev}{}", filter.note())
        };
        let _ = tx.send(LoadMsg::Empty(msg));
        return;
    }
    let timing = format!(
        "ordo-tui: read {files} file{} in {read_ms}ms · highlighted {hl_files} in {hl_ms}ms · \
         ordered {} hunks into {} groups, {} cluster{} in {}ms",
        plural(files),
        items.len(),
        out.groups.len(),
        out.clusters.len(),
        plural(out.clusters.len()),
        t.elapsed().as_millis(),
    );
    // reviewed-mark persistence: repo-scoped, best-effort — an unresolved
    // repo root or cache dir just means this session's marks live only in
    // memory (see `marks_file_path`/`load_marks`)
    let repo_root = git(&["rev-parse", "--show-toplevel"]);
    let repo_root = repo_root.trim();
    let marks_path = (!repo_root.is_empty()).then(|| marks_file_path(repo_root)).flatten();
    let mut marks = marks_path.as_deref().map(load_marks).unwrap_or_default();
    prune_marks(&mut marks, now_unix());
    let reviewed: Vec<bool> = items
        .iter()
        .map(|it| mark_key(&rev, it, &sources).is_some_and(|k| marks.contains_key(&k)))
        .collect();
    let _ = tx.send(LoadMsg::Done(Box::new(LoadResult {
        items,
        view,
        comments_only: only_comments,
        sources,
        highlights,
        timing,
        reviewed,
        marks_path,
        marks,
        groups,
        ledger,
    })));
}

// ------------------------------------------------------------------- git layer

// Empty string when git fails, so a missing blob reads as empty content. The
// status check matters: `rev-parse --verify -q` still prints on failure (a range
// echoes both endpoints), and taking that output would be read as a sha.
fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// Same, with the arg list fed on stdin — for `check-attr --stdin`, where the
// path list can outgrow what a command line takes.
fn git_stdin(args: &[&str], input: &str) -> String {
    use std::io::Write;
    let mut child = match Command::new("git")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    if let Some(mut w) = child.stdin.take() {
        let _ = w.write_all(input.as_bytes());
    }
    child
        .wait_with_output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// GitButler CLI: empty string if `but` isn't installed or the call fails, so the
// plain-git path is unaffected on non-GitButler repos.
fn but(args: &[&str]) -> String {
    Command::new("but")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// Paths the repo's own `.gitattributes` marks as generated: `linguist-generated`
// (the marker GitHub collapses a file by) or an explicit `-diff`. One batched
// call — `-z` makes both the path list and the output NUL-separated, so paths
// with colons or newlines survive the round trip.
fn declared_generated(paths: &[String]) -> std::collections::HashSet<String> {
    let mut input = paths.join("\0");
    input.push('\0');
    let out = git_stdin(
        &["check-attr", "-z", "--stdin", "linguist-generated", "diff"],
        &input,
    );
    let mut fields = out.split('\0');
    let mut generated = std::collections::HashSet::new();
    while let (Some(path), Some(attr), Some(value)) = (fields.next(), fields.next(), fields.next())
    {
        if path.is_empty() {
            break;
        }
        let declared = match attr {
            // a bare `linguist-generated` reads as "set", `=true` as "true"
            "linguist-generated" => value == "set" || value == "true",
            "diff" => value == "unset",
            _ => false,
        };
        if declared {
            generated.insert(path.to_string());
        }
    }
    generated
}

// -------------------------------------------------------------- gitbutler layer

/// `but --json status` — the whole workspace in one call: the uncommitted
/// changes, each stack's branches and their commits, and the merge base. None
/// when `but` is missing or the repo isn't GitButler-managed.
fn workspace() -> Option<serde_json::Value> {
    serde_json::from_str(&but(&["--json", "status"])).ok()
}

fn field<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

fn branches(ws: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    ws.get("stacks")
        .and_then(|s| s.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("branches")?.as_array())
        .flatten()
}

/// A workspace branch — by the CLI ID `but status` prints, or by its name —
/// reviews as the range spanning its own commits, which is what reviewing one
/// branch of a stack means. Takes precedence over git's reading of the same
/// name, where a branch is only its tip commit.
fn branch_target(ws: &serde_json::Value, arg: &str) -> Option<Target> {
    let b = branches(ws)
        .find(|b| field(b, "cliId") == arg || field(b, "name") == arg)?;
    let commits = b.get("commits")?.as_array()?;
    fn commit_id(c: &serde_json::Value) -> Option<&str> {
        Some(field(c, "commitId")).filter(|s| !s.is_empty())
    }
    // `but` lists a branch newest-first, so the range runs from below the last
    // commit up to the first
    let tip = commit_id(commits.first()?)?.to_string();
    let oldest = commit_id(commits.last()?)?;
    let base = git(&["rev-parse", "--verify", "-q", &format!("{oldest}^")]);
    let base = base.trim();
    let base = if base.is_empty() { EMPTY_TREE } else { base };
    Some(Target::Range(base.to_string(), tip))
}

/// A workspace commit by its CLI ID, or by a prefix of its change ID or commit
/// ID — the same identifiers `but status` accepts.
fn commit_target(ws: &serde_json::Value, arg: &str) -> Option<Target> {
    let c = branches(ws)
        .filter_map(|b| b.get("commits")?.as_array())
        .flatten()
        .find(|c| {
            field(c, "cliId") == arg
                || field(c, "changeId").starts_with(arg)
                || field(c, "commitId").starts_with(arg)
        })?;
    Some(Target::Commit(field(c, "commitId").to_string()))
}

enum Target {
    Commit(String),
    /// A `base..tip` / `base...tip` range, already resolved to two commit shas.
    Range(String, String),
    Uncommitted,
    /// `base..zz` / `base...zz` — a resolved base commit versus the worktree
    /// (not HEAD). `...` has already been reduced to the merge base of `base`
    /// and HEAD, same as `Range`'s symmetric case.
    WorktreeRange(String),
}

// `base..tip` is a plain two-endpoint diff; `base...tip` diffs from the merge
// base, which is what you want for a branch that trails its target. An omitted
// side means HEAD, as in git. `tip == "zz"` diffs against the worktree instead
// of a commit — see `WorktreeRange`; `zz` on the left is meaningless (nothing
// precedes the uncommitted area) and rejected.
fn resolve_range(arg: &str) -> Option<Target> {
    let (base, tip, symmetric) = match arg.split_once("...") {
        Some((b, t)) => (b, t, true),
        None => {
            let (b, t) = arg.split_once("..")?;
            (b, t, false)
        }
    };
    // a second `..` is not a range we understand (e.g. `a..b..c`)
    if tip.contains("..") {
        return None;
    }
    if base == "zz" {
        return None;
    }
    let rev = |s: &str| {
        let s = if s.is_empty() { "HEAD" } else { s };
        let sha = git(&["rev-parse", "--verify", "-q", &format!("{s}^{{commit}}")]);
        let sha = sha.trim().to_string();
        (!sha.is_empty()).then_some(sha)
    };
    if tip == "zz" {
        let base = rev(base)?;
        if !symmetric {
            return Some(Target::WorktreeRange(base));
        }
        let merge_base = git(&["merge-base", &base, "HEAD"]);
        let merge_base = merge_base.trim();
        return (!merge_base.is_empty()).then(|| Target::WorktreeRange(merge_base.to_string()));
    }
    let (base, tip) = (rev(base)?, rev(tip)?);
    if !symmetric {
        return Some(Target::Range(base, tip));
    }
    let merge_base = git(&["merge-base", &base, &tip]);
    let merge_base = merge_base.trim();
    (!merge_base.is_empty()).then(|| Target::Range(merge_base.to_string(), tip))
}

// Resolve an arg to a commit, a range, or the uncommitted area. `zz` is
// GitButler's constant for the uncommitted area and never a commit. Ranges come
// next, then a GitButler workspace *branch* — ahead of git so a branch label
// reviews the branch's commits rather than just its tip. Plain git commit-ishes
// (sha, HEAD~2, tag) follow, and finally a GitButler commit CLI ID. The
// GitButler steps are skipped entirely on a repo `but` doesn't manage.
fn resolve(arg: &str) -> Option<Target> {
    if arg == "zz" {
        return Some(Target::Uncommitted);
    }
    // `..` is illegal in a refname, so it can only mean a range here
    if arg.contains("..") {
        return resolve_range(arg);
    }
    let ws = workspace();
    if let Some(t) = ws.as_ref().and_then(|ws| branch_target(ws, arg)) {
        return Some(t);
    }
    let sha = git(&["rev-parse", "--verify", "-q", &format!("{arg}^{{commit}}")]);
    let sha = sha.trim();
    if !sha.is_empty() {
        return Some(Target::Commit(sha.to_string()));
    }
    ws.as_ref().and_then(|ws| commit_target(ws, arg))
}

// The single commit the hover popup's history section is relative to: a
// range reviews as its tip, and `zz` (the uncommitted area) has no commit of
// its own, so HEAD stands in for it. None only if HEAD itself can't resolve
// (an empty repo) — history is then just unavailable.
fn review_commit_sha(target: &Target) -> Option<String> {
    match target {
        Target::Commit(sha) => Some(sha.clone()),
        Target::Range(_, tip) => Some(tip.clone()),
        Target::Uncommitted | Target::WorktreeRange(_) => {
            let sha = git(&["rev-parse", "--verify", "-q", "HEAD"]);
            let sha = sha.trim();
            (!sha.is_empty()).then(|| sha.to_string())
        }
    }
}

fn gather(rev: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{rev}^")]);
    let parent = parent.trim();
    let parent = if parent.is_empty() {
        EMPTY_TREE
    } else {
        parent
    };
    gather_range(parent, rev, filter, progress)
}

// Both sides' full blobs (empty when a file is added or deleted), so hunks get
// full semantics instead of degrading to a context-limited patch.
fn gather_range(base: &str, tip: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let names = git(&["diff", "--name-only", base, tip]);
    let paths: Vec<String> = names
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let paths = filter.apply(paths);
    let total = paths.len();
    let changes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            progress(read_progress(&path, i, total));
            Change {
                old: Some(git(&["show", &format!("{base}:{path}")])),
                new: Some(git(&["show", &format!("{tip}:{path}")])),
                diff: None,
                path,
            }
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// The uncommitted area (`zz`): GitButler owns what counts as uncommitted (it
// includes untracked adds git-diff wouldn't show), so take the file list from
// the workspace status — which lists it per file, unlike `but diff`'s per-hunk
// entries. Changes assigned to a stack are picked up the same way. Full old/new
// content — old from HEAD, new from the worktree — so hunks get full semantics
// rather than degrading to a context-limited patch.
fn gather_uncommitted(filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let ws = workspace().unwrap_or(serde_json::Value::Null);
    let assigned = ws
        .get("stacks")
        .and_then(|s| s.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("assignedChanges")?.as_array())
        .flatten();
    let mut paths: Vec<String> = ws
        .get("uncommittedChanges")
        .and_then(|c| c.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .chain(assigned)
        .map(|c| field(c, "filePath").to_string())
        .filter(|p| !p.is_empty())
        .collect();
    paths.dedup();
    let paths = filter.apply(paths);
    let total = paths.len();
    let changes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            progress(read_progress(&path, i, total));
            Change {
                old: Some(git(&["show", &format!("HEAD:{path}")])),
                new: Some(std::fs::read_to_string(&path).unwrap_or_default()),
                diff: None,
                path,
            }
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// `base..zz` / `base...zz`: everything done on a branch including what's not
// yet committed — `base`'s tree versus the worktree. `git diff --name-only
// <base>` (one revision, no second) already compares a tree to the working
// directory, but it never lists untracked files, so the changed-file set is
// that plus untracked paths — from `but --json status`, same source
// `gather_uncommitted` uses, on a GitButler-managed repo, else `git ls-files
// --others --exclude-standard`. `old` comes from `base`'s tree (empty when
// the path didn't exist there); `new` is read straight off disk. A path
// absent from the worktree (deleted since `base`) gets an empty `new`, not a
// read error; a path that exists but can't be read as UTF-8 is dropped
// rather than folded into an empty string, which would misreport it as a
// deletion.
fn gather_worktree_range(base: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let mut paths: Vec<String> = git(&["diff", "--name-only", base])
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let untracked: Vec<String> = match workspace() {
        Some(ws) => {
            let assigned = ws
                .get("stacks")
                .and_then(|s| s.as_array())
                .map(|v| v.as_slice())
                .unwrap_or_default()
                .iter()
                .filter_map(|s| s.get("assignedChanges")?.as_array())
                .flatten();
            ws.get("uncommittedChanges")
                .and_then(|c| c.as_array())
                .map(|v| v.as_slice())
                .unwrap_or_default()
                .iter()
                .chain(assigned)
                .map(|c| field(c, "filePath").to_string())
                .filter(|p| !p.is_empty())
                .collect()
        }
        None => git(&["ls-files", "--others", "--exclude-standard"])
            .split('\n')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
    };
    paths.extend(untracked);
    paths.sort();
    paths.dedup();
    let paths = filter.apply(paths);
    let total = paths.len();
    let changes = paths
        .into_iter()
        .enumerate()
        .filter_map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let new = match std::fs::read(&path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(_) => {
                    filter.note_unreadable();
                    return None;
                }
            };
            Some(Change {
                old: Some(git(&["show", &format!("{base}:{path}")])),
                new: Some(new),
                diff: None,
                path,
            })
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// ------------------------------------------------------------------- view model

type Sources = HashMap<String, (Vec<String>, Vec<String>)>;

struct Item {
    path: String,
    old_range: [usize; 2],
    new_range: [usize; 2],
    /// the list row's parts, kept unjoined so line number and category can be
    /// coloured independently of the path
    mark: String,
    cat: String,
    rationale: String,
    details: Vec<String>,
    notes: Vec<String>,
    edges: Vec<EdgeRef>,
    advisories: Vec<(String, String, bool)>,
    noise: bool,
    /// every changed line is a comment or docstring — drives `:only-comments`
    /// (the engine's own field of the same name, see `HunkOut::comment`)
    comment: bool,
    /// the hunk's own defined symbols (name + tree-sitter kind + scope) — the
    /// identity component of a persisted reviewed-mark key (see `mark_key`)
    symbols: Vec<Symbol>,
    /// fallback identity when `symbols` is empty (a body-edit or comment-only
    /// hunk defines nothing of its own)
    enclosing: Option<String>,
    /// the engine's `HunkOut::group` id — drives `:group`'s header rows (see
    /// `App::groups` for the id -> reason lookup)
    group: String,
    /// intra-line refinement (`ordo::refine`), parallel to the hunk's lines on
    /// each side: `Some(spans)` means the line was paired with its counterpart
    /// and only those char spans changed; `None` means it renders whole. Empty
    /// when the file has no grammar or the hunk was too large to refine.
    refined: ordo::refine::Refined,
}

/// One `dep` line's rendered label plus the target hunk's resolved position in
/// `app.items` — `None` when the referenced hunk was never built into this
/// review (filtered out, `--only-comments`, or a file that wasn't sent at
/// all), so the why pane can act on it (preview, jump) without ever guessing.
struct EdgeRef {
    label: String,
    target: Option<usize>,
}

// ---------------------------------------------------------- reviewed-mark persistence

/// Hand-rolled FNV-1a 64-bit. Deliberately not `DefaultHasher` — its output is
/// explicitly unspecified across Rust releases, so a toolchain upgrade would
/// silently invalidate every mark ever written. This is ~10 lines and never
/// changes.
fn fnv1a(bytes: &[u8]) -> u64 {
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
        let mut syms = item.symbols.clone();
        syms.sort();
        syms.iter()
            .map(|s| format!("{}\u{1f}{}\u{1f}{}", s.name, s.kind, s.scope.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\u{1e}")
    } else {
        item.enclosing.clone().unwrap_or_default()
    }
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
    let mut buf = String::new();
    if o0 >= 1 && o0 <= o1 && o1 <= ol.len() {
        for l in &ol[o0 - 1..o1] {
            buf.push_str(l);
            buf.push('\n');
        }
    }
    buf.push('\u{0}');
    if n0 >= 1 && n0 <= n1 && n1 <= nl.len() {
        for l in &nl[n0 - 1..n1] {
            buf.push_str(l);
            buf.push('\n');
        }
    }
    Some(fnv1a(buf.as_bytes()))
}

/// The full persistence key: `(rev, path, symbol identity, hunk content
/// hash)`, folded into one FNV-1a hash so the file on disk records only an
/// opaque number — never a path, a symbol name, or source text. `None` when
/// there's nothing to hash the content from.
fn mark_key(rev: &str, item: &Item, sources: &Sources) -> Option<u64> {
    let content_hash = hunk_content_hash(item, sources)?;
    let sym_key = symbol_identity_key(item);
    let combined = format!("{rev}\u{0}{}\u{0}{sym_key}\u{0}{content_hash:x}", item.path);
    Some(fnv1a(combined.as_bytes()))
}

const MARK_TTL_SECS: u64 = 90 * 24 * 60 * 60;

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn cache_home() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        if !x.trim().is_empty() {
            return Some(PathBuf::from(x));
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
fn marks_file_path(repo_root: &str) -> Option<PathBuf> {
    let dir = cache_home()?.join("ordo").join("reviewed");
    Some(dir.join(format!("{:016x}.json", fnv1a(repo_root.as_bytes()))))
}

/// Reads the mark file: hex key -> unix-seconds written. Any failure at all —
/// missing file, unreadable, corrupt JSON — degrades to "no marks" rather
/// than a crash or a blocked UI; the cache is a convenience, the review is
/// the product.
fn load_marks(path: &Path) -> HashMap<u64, u64> {
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
fn prune_marks(marks: &mut HashMap<u64, u64>, now: u64) {
    marks.retain(|_, ts| now.saturating_sub(*ts) <= MARK_TTL_SECS);
}

/// Best-effort write: creates the parent directory if needed, and silently
/// gives up on any failure (read-only filesystem, permission, missing
/// `$HOME`) rather than surfacing it — a lost mark is the accepted cost, a
/// crash or a blocked review is not. The files are tiny, so this runs on
/// every toggle rather than only at quit, and a panic or killed terminal
/// never loses the session's marks.
fn save_marks(path: &Path, marks: &HashMap<u64, u64>) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let body: HashMap<String, u64> = marks.iter().map(|(k, v)| (format!("{k:016x}"), *v)).collect();
    if let Ok(text) = serde_json::to_string(&body) {
        let _ = std::fs::write(path, text);
    }
}


// ----------------------------------------------------------------------- keys

/// The three panes, in focus-cycle order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    List,
    Code,
    Why,
}

impl Pane {
    fn next(self) -> Pane {
        match self {
            Pane::List => Pane::Code,
            Pane::Code => Pane::Why,
            Pane::Why => Pane::List,
        }
    }
    fn prev(self) -> Pane {
        match self {
            Pane::List => Pane::Why,
            Pane::Code => Pane::List,
            Pane::Why => Pane::Code,
        }
    }
}

#[derive(Clone, Copy)]
enum Action {
    Quit,
    Next,
    Prev,
    First,
    Last,
    ToggleReviewed,
    PageDown,
    PageUp,
    HalfDown,
    HalfUp,
    FocusNext,
    FocusPrev,
    Focus(Pane),
    // code-pane cursor motions
    CursorLeft,
    CursorRight,
    WordNext,
    WordPrev,
    WordEnd,
    /// column line-end motion (vim 0/$, vscode Home/End); pane-dependent, see
    /// `apply` — falls back to `First`/`Last`'s meaning outside the code pane
    LineStart,
    LineEnd,
    ParaPrev,
    ParaNext,
    /// symbol under the cursor, in a floating popup
    Hover,
    /// `/` — open the text-search prompt
    SearchOpen,
    /// `*` / `#` — jump to the next/previous tree-sitter occurrence of the
    /// symbol under the cursor
    SymbolNext,
    SymbolPrev,
    /// `n` / `N` — cycle the currently active search (text or symbol)
    SearchNext,
    SearchPrev,
    /// open the selected hunk's file in `$VISUAL`/`$EDITOR`, positioned at its
    /// line — handled specially in `run` (it needs to suspend the terminal)
    OpenEditor,
    /// `?` / `F1` — open the generated keybinding-help popup
    Help,
    /// `:` (vim), `C-Shift-P` (vscode) — open the command bar empty
    CommandOpen,
    /// `C-P` (vscode's own "Go to File") — open the command bar pre-filled
    /// with `goto `
    CommandGoto,
    /// `Enter`/`gd` (vim), `C-Enter` (vscode) — jump to the why pane's current
    /// dep line's target hunk; a no-op outside the why pane, or when the
    /// current line isn't a dep line, or its target isn't part of this review
    JumpToEdge,
    /// `C-o` (vim) / `Alt-Left` (vscode) — pop the position stack `JumpToEdge`
    /// pushed, returning to where the jump was made from
    JumpBack,
    /// `zh`/`zl` (vim), shift-left/shift-right (vscode) — scroll the code
    /// pane's horizontal window without moving the cursor
    ScrollLeft,
    ScrollRight,
}

type Key = (KeyCode, KeyModifiers);
/// An optional prefix key (for chords like `C-w C-w`), the key, and its action.
type Bind = (Option<Key>, Key, Action);

fn ch(c: char) -> Key {
    (KeyCode::Char(c), KeyModifiers::NONE)
}
fn ctrl(c: char) -> Key {
    (KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn plain(c: KeyCode) -> Key {
    (c, KeyModifiers::NONE)
}
fn ctrlk(c: KeyCode) -> Key {
    (c, KeyModifiers::CONTROL)
}

/// An uppercase char already carries its case, so SHIFT is dropped there to keep
/// the tables independent of how a terminal reports it.
fn norm(code: KeyCode, mods: KeyModifiers) -> Key {
    match code {
        KeyCode::Char(_) => (code, mods.difference(KeyModifiers::SHIFT)),
        _ => (code, mods),
    }
}

#[derive(Clone)]
struct Keymap {
    name: &'static str,
    hint: &'static str,
    binds: Vec<Bind>,
}

enum Resolve {
    Act(Action),
    /// The key opens a chord; hold it and wait for the next one.
    Pending,
    Miss,
}

impl Keymap {
    fn resolve(&self, pending: Option<Key>, key: Key) -> Resolve {
        let find = |pre: Option<Key>| {
            self.binds
                .iter()
                .find(|(p, k, _)| *p == pre && *k == key)
                .map(|(_, _, a)| Resolve::Act(*a))
        };
        if let Some(p) = pending {
            return find(Some(p)).unwrap_or(Resolve::Miss);
        }
        if self.binds.iter().any(|(p, _, _)| *p == Some(key)) {
            return Resolve::Pending;
        }
        find(None).unwrap_or(Resolve::Miss)
    }
}

fn keymap(name: &str) -> Option<Keymap> {
    match name {
        "vim" => Some(Keymap {
            name: "vim",
            hint: "j/k move · h/l/w/b/e/0/$ cursor · zh/zl hscroll · K symbol/dep · / search · */# sym-occ · n/N cycle · gd/Enter jump-dep · C-o back · ge edit · : cmd · ? help · q quit",
            binds: vec![
                (None, ch('q'), Action::Quit),
                (None, plain(KeyCode::Esc), Action::Quit),
                (None, ch('?'), Action::Help),
                (None, ch(':'), Action::CommandOpen),
                (None, ch('j'), Action::Next),
                (None, plain(KeyCode::Down), Action::Next),
                (None, ch('k'), Action::Prev),
                (None, plain(KeyCode::Up), Action::Prev),
                (Some(ch('g')), ch('g'), Action::First),
                (None, ch('G'), Action::Last),
                (None, ch('x'), Action::ToggleReviewed),
                (None, ch(' '), Action::PageDown),
                (None, ch('f'), Action::PageDown),
                (None, ctrl('f'), Action::PageDown),
                (None, plain(KeyCode::PageDown), Action::PageDown),
                (None, ctrl('b'), Action::PageUp),
                (None, plain(KeyCode::PageUp), Action::PageUp),
                (None, ctrl('d'), Action::HalfDown),
                (None, ctrl('u'), Action::HalfUp),
                (Some(ctrl('w')), ctrl('w'), Action::FocusNext),
                (Some(ctrl('w')), ch('w'), Action::FocusNext),
                (Some(ctrl('w')), ch('W'), Action::FocusPrev),
                (Some(ctrl('w')), ch('p'), Action::FocusPrev),
                (Some(ctrl('w')), ch('h'), Action::Focus(Pane::List)),
                (Some(ctrl('w')), ch('l'), Action::Focus(Pane::Code)),
                (Some(ctrl('w')), ch('k'), Action::Focus(Pane::Code)),
                (Some(ctrl('w')), ch('j'), Action::Focus(Pane::Why)),
                // code-pane cursor motions — `b` (word-back) displaces the old
                // bare-b page-up shortcut; C-b/PageUp still page.
                (None, ch('h'), Action::CursorLeft),
                (None, ch('l'), Action::CursorRight),
                (None, ch('w'), Action::WordNext),
                (None, ch('b'), Action::WordPrev),
                (None, ch('e'), Action::WordEnd),
                (None, ch('0'), Action::LineStart),
                (None, ch('$'), Action::LineEnd),
                (None, ch('{'), Action::ParaPrev),
                (None, ch('}'), Action::ParaNext),
                // `z` prefix (vim's own convention for view-scrolling
                // commands, e.g. zh/zl to scroll a `nowrap` window sideways)
                (Some(ch('z')), ch('h'), Action::ScrollLeft),
                (Some(ch('z')), ch('l'), Action::ScrollRight),
                (None, ch('K'), Action::Hover),
                (None, ch('/'), Action::SearchOpen),
                (None, ch('*'), Action::SymbolNext),
                (None, ch('#'), Action::SymbolPrev),
                (None, ch('n'), Action::SearchNext),
                (None, ch('N'), Action::SearchPrev),
                // `ge` (not bare `e`, which is already word-end in the code
                // pane) — mnemonic "go edit", chorded off the same `g` prefix
                // as `gg`
                (Some(ch('g')), ch('e'), Action::OpenEditor),
                // `gd` ("go to definition"), off the same `g` prefix as
                // `gg`/`ge`; `Enter` is free in vim (toggling reviewed is `x`
                // here, not `Enter`/`Space` as in vscode) so it's bound too
                (Some(ch('g')), ch('d'), Action::JumpToEdge),
                (None, plain(KeyCode::Enter), Action::JumpToEdge),
                // vim's own jumplist key, and free here — `ge`/OpenEditor
                // took the `g` prefix's `e`, not `C-o`
                (None, ctrl('o'), Action::JumpBack),
            ],
        }),
        "vscode" => Some(Keymap {
            name: "vscode",
            hint: "↑/↓ move · ←/→/C-←/C-→ cursor · S-←/S-→ hscroll · F12 symbol/dep · C-f search · F3 next · C-Enter jump-dep · A-← back · C-o edit · C-Shift-P cmd · F1 help · C-q quit",
            binds: vec![
                (None, ctrl('q'), Action::Quit),
                (None, plain(KeyCode::Esc), Action::Quit),
                // F1 is vscode's own "show command palette / help" key;
                // Ctrl+Shift+P (its other command-palette binding) is a
                // three-key chord many terminals don't report cleanly, so F1
                // alone is the more reliable analog for "show me the keys"
                (None, plain(KeyCode::F(1)), Action::Help),
                // vscode's own command-palette key; Ctrl+P ("Go to File") is
                // free here (list navigation is Up/Down, not C-p/C-n) and
                // maps to the closest analog this reviewer has: `:goto`
                (None, ctrl('P'), Action::CommandOpen),
                (None, ctrl('p'), Action::CommandGoto),
                (None, plain(KeyCode::Down), Action::Next),
                (None, plain(KeyCode::Up), Action::Prev),
                (None, ch(' '), Action::ToggleReviewed),
                (None, plain(KeyCode::Enter), Action::ToggleReviewed),
                (None, plain(KeyCode::PageDown), Action::PageDown),
                (None, plain(KeyCode::PageUp), Action::PageUp),
                (None, plain(KeyCode::F(6)), Action::FocusNext),
                (
                    None,
                    (KeyCode::F(6), KeyModifiers::SHIFT),
                    Action::FocusPrev,
                ),
                (None, ctrl('1'), Action::Focus(Pane::List)),
                (None, ctrl('2'), Action::Focus(Pane::Code)),
                (None, ctrl('3'), Action::Focus(Pane::Why)),
                // code-pane cursor motions — Home/End move column-wise here
                // (list navigation keeps `LineStart`/`LineEnd`'s list fallback).
                (None, plain(KeyCode::Left), Action::CursorLeft),
                (None, plain(KeyCode::Right), Action::CursorRight),
                (None, ctrlk(KeyCode::Right), Action::WordNext),
                (None, ctrlk(KeyCode::Left), Action::WordPrev),
                (None, plain(KeyCode::Home), Action::LineStart),
                (None, plain(KeyCode::End), Action::LineEnd),
                // shift-left/right is otherwise unused here (no text
                // selection in this reviewer), so it's free for the
                // horizontal-scroll pair vscode has no dedicated key for
                (None, (KeyCode::Left, KeyModifiers::SHIFT), Action::ScrollLeft),
                (None, (KeyCode::Right, KeyModifiers::SHIFT), Action::ScrollRight),
                (None, plain(KeyCode::F(12)), Action::Hover),
                (None, ctrl('f'), Action::SearchOpen),
                (None, plain(KeyCode::F(3)), Action::SearchNext),
                (
                    None,
                    (KeyCode::F(3), KeyModifiers::SHIFT),
                    Action::SearchPrev,
                ),
                // F12 is already `Hover` (def signature); Ctrl/Shift+Ctrl+F12
                // is the closest free analog to vscode's own references keys
                // (Shift+F12 "Go to References") for cycling occurrences.
                (None, ctrlk(KeyCode::F(12)), Action::SymbolNext),
                (
                    None,
                    (KeyCode::F(12), KeyModifiers::CONTROL | KeyModifiers::SHIFT),
                    Action::SymbolPrev,
                ),
                // vscode's own default binding for "File: Open File" —
                // reused here for "open this location in the editor"
                (None, ctrl('o'), Action::OpenEditor),
                // `Enter` is already `ToggleReviewed` here (vscode's own
                // "activate" key); vscode's actual "Go to Definition" (F12)
                // is likewise already `Hover`, so `C-Enter` (free, and reads
                // as "Enter, but do more") is the jump-to-dep key instead
                (None, (KeyCode::Enter, KeyModifiers::CONTROL), Action::JumpToEdge),
                // vscode's own default "Go Back" binding
                (None, (KeyCode::Left, KeyModifiers::ALT), Action::JumpBack),
            ],
        }),
        _ => None,
    }
}

// ------------------------------------------------------------------ key help

/// Grouping for the generated `?`/`F1` help popup. `General` isn't among the
/// project owner's suggested categories (navigation/panes/search/review/
/// editor/help) but earns its own row for `Quit`, which fits none of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    General,
    Navigation,
    Panes,
    Search,
    Review,
    Editor,
    Help,
}

fn category_label(c: Category) -> &'static str {
    match c {
        Category::General => "general",
        Category::Navigation => "navigation",
        Category::Panes => "panes",
        Category::Search => "search",
        Category::Review => "review",
        Category::Editor => "editor",
        Category::Help => "help",
    }
}

/// Category + one-line description for an action — the only hand-written
/// prose in the help system. Everything else (which keys, in what preset)
/// comes straight from `Keymap.binds`, so this text can drift in *wording*
/// but never in *which key does what*.
fn action_help(a: Action) -> (Category, &'static str) {
    match a {
        Action::Quit => (Category::General, "quit (or dismiss an open popup)"),
        Action::Next => (Category::Navigation, "next item / move cursor down"),
        Action::Prev => (Category::Navigation, "previous item / move cursor up"),
        Action::First => (Category::Navigation, "jump to the first item / top"),
        Action::Last => (Category::Navigation, "jump to the last item / bottom"),
        Action::ToggleReviewed => (Category::Review, "toggle reviewed on the selected hunk"),
        Action::PageDown => (Category::Navigation, "page down"),
        Action::PageUp => (Category::Navigation, "page up"),
        Action::HalfDown => (Category::Navigation, "half-page down"),
        Action::HalfUp => (Category::Navigation, "half-page up"),
        Action::FocusNext => (Category::Panes, "focus the next pane"),
        Action::FocusPrev => (Category::Panes, "focus the previous pane"),
        Action::Focus(Pane::List) => (Category::Panes, "focus the reading-order pane"),
        Action::Focus(Pane::Code) => (Category::Panes, "focus the code pane"),
        Action::Focus(Pane::Why) => (Category::Panes, "focus the why pane"),
        Action::CursorLeft => (Category::Navigation, "move the code cursor left"),
        Action::CursorRight => (Category::Navigation, "move the code cursor right"),
        Action::WordNext => (Category::Navigation, "move the code cursor to the next word"),
        Action::WordPrev => (Category::Navigation, "move the code cursor to the previous word"),
        Action::WordEnd => (Category::Navigation, "move the code cursor to the end of the word"),
        Action::LineStart => (Category::Navigation, "move to line start / pane top"),
        Action::LineEnd => (Category::Navigation, "move to line end / pane bottom"),
        Action::ParaPrev => (Category::Navigation, "jump to the previous blank line"),
        Action::ParaNext => (Category::Navigation, "jump to the next blank line"),
        Action::ScrollLeft => (
            Category::Navigation,
            "scroll the code pane (or an open popup) left",
        ),
        Action::ScrollRight => (
            Category::Navigation,
            "scroll the code pane (or an open popup) right",
        ),
        Action::Hover => (
            Category::Editor,
            "code pane: show the symbol under the cursor and its history · why pane: preview the current dep line's target",
        ),
        Action::SearchOpen => (Category::Search, "open the text-search prompt"),
        Action::SymbolNext => (Category::Search, "next occurrence of the symbol under the cursor"),
        Action::SymbolPrev => (Category::Search, "previous occurrence of the symbol under the cursor"),
        Action::SearchNext => (Category::Search, "cycle to the next match"),
        Action::SearchPrev => (Category::Search, "cycle to the previous match"),
        Action::OpenEditor => (Category::Editor, "open the selected hunk's file in $VISUAL/$EDITOR"),
        Action::JumpToEdge => (
            Category::Navigation,
            "why pane: jump to the current dep line's target hunk",
        ),
        Action::JumpBack => (Category::Navigation, "jump back to the position before the last dep jump"),
        Action::Help => (Category::Help, "show this keybinding help"),
        Action::CommandOpen => (
            Category::General,
            "open the command bar (:only-comments, :all, :filter, :keys, :strategy, :goto, :help, :q)",
        ),
        Action::CommandGoto => (Category::General, "open the command bar pre-filled with `goto `"),
    }
}

/// One key, rendered legibly: `C-`/`S-`/`A-` modifier prefixes, named special
/// keys, `F<n>` for function keys, the char itself otherwise.
fn key_label(key: Key) -> String {
    let (code, mods) = key;
    let mut prefix = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        prefix.push_str("C-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        prefix.push_str("S-");
    }
    if mods.contains(KeyModifiers::ALT) {
        prefix.push_str("A-");
    }
    let body = match code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        other => format!("{other:?}"),
    };
    format!("{prefix}{body}")
}

/// A bind's full chord: `gg`/`ge` (no space — vim's own convention for a
/// plain-char chord) vs. `C-w C-w` (spaced — either half carries a modifier
/// or a named key, and vim always writes those chords spaced).
fn chord_label(prefix: Option<Key>, key: Key) -> String {
    let Some(p) = prefix else {
        return key_label(key);
    };
    let simple = |k: Key| matches!(k.0, KeyCode::Char(_)) && k.1 == KeyModifiers::NONE;
    if simple(p) && simple(key) {
        format!("{}{}", key_label(p), key_label(key))
    } else {
        format!("{} {}", key_label(p), key_label(key))
    }
}

/// The `?`/`F1` help popup body: every bind in `keys`, grouped by category and
/// collapsed onto one row per action (several keys can mean the same thing,
/// e.g. `j` and `Down` both `Next`) — generated straight from `Keymap.binds`
/// rather than hand-duplicated, so it cannot describe a key the table doesn't
/// actually bind.
fn build_help(keys: &Keymap) -> Vec<String> {
    struct Row {
        category: Category,
        desc: &'static str,
        keys: Vec<String>,
    }
    let mut rows: Vec<Row> = vec![];
    for &(prefix, key, action) in &keys.binds {
        let (category, desc) = action_help(action);
        let label = chord_label(prefix, key);
        match rows.iter_mut().find(|r| r.category == category && r.desc == desc) {
            Some(r) if !r.keys.contains(&label) => r.keys.push(label),
            Some(_) => {}
            None => rows.push(Row { category, desc, keys: vec![label] }),
        }
    }
    let order = [
        Category::General,
        Category::Navigation,
        Category::Panes,
        Category::Search,
        Category::Review,
        Category::Editor,
        Category::Help,
    ];
    let mut out = vec![];
    for &cat in &order {
        let group: Vec<&Row> = rows.iter().filter(|r| r.category == cat).collect();
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(category_label(cat).to_uppercase());
        for r in group {
            out.push(format!("  {:<16} {}", r.keys.join(", "), r.desc));
        }
    }
    out
}

/// A cursor position in the *new* content of the selected item's file — line
/// and column are both 0-based char offsets. Only meaningful for the code pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cursor {
    line: usize,
    col: usize,
}

/// A parsed file, cached per path so repeated `K` presses don't reparse. `src`
/// is the exact joined text the tree was built from, so node byte ranges slice
/// it directly.
struct ParsedFile {
    tree: Tree,
    src: String,
}

/// The floating symbol-hover popup over the code pane: kind/signature/doc,
/// then (when the definition resolves) a cross-commit history section.
struct Popup {
    title: String,
    /// styled, so a popup can carry syntax-highlighted code (the dep preview)
    /// rather than only prose
    lines: Vec<Line<'static>>,
    /// vertical scroll offset, so a popup taller than the terminal (the help
    /// popup, most likely) is reachable rather than just clipped
    scroll: u16,
    /// horizontal offset — popups do not wrap, matching the code pane, so a
    /// long line is reached by scrolling rather than reflowed
    hscroll: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchKind {
    Text,
    Symbol,
}

/// An active `/`, `*`, or `#` search over the code pane's current file.
/// `matches` are (line, start_col, end_col) char ranges in document order;
/// `index` is the current one — `n`/`N` cycle through them, wrapping.
struct Search {
    kind: SearchKind,
    pattern: String,
    matches: Vec<(usize, usize, usize)>,
    index: usize,
}

/// The `/` search prompt: keystrokes build `text` instead of running normal
/// actions (see `run`'s prompt branch). `anchor` is the cursor position to
/// restore on `Esc`.
struct Prompt {
    text: String,
    anchor: Cursor,
}

/// The `:` command bar: keystrokes build `text` instead of running normal
/// actions (see `run`'s command branch and `handle_command_key`), same
/// interception scheme as `Prompt`. `candidates` is the current completion
/// menu (command names, then — once `text` has a space — that command's own
/// argument candidates); `selected` is which of them is highlighted, `None`
/// when the user hasn't cycled to one yet.
struct CommandBar {
    text: String,
    candidates: Vec<String>,
    selected: Option<usize>,
}

struct App {
    items: Vec<Item>,
    reviewed: Vec<bool>,
    /// indices into `items`, currently visible under `comments_only`/
    /// `show_all`/`path_filter` — never empty (a filter that would empty it
    /// is rejected, see `set_filters`). Navigation and the list pane move
    /// through this rather than `items` directly, so a hidden hunk is never
    /// selected.
    view: Vec<usize>,
    /// `:only-comments` — show only `Item::comment` hunks
    comments_only: bool,
    /// `:all` — show `Item::noise` hunks too (generated paths and
    /// formatting-only changes); defaults to `true` (everything shown),
    /// matching this reviewer's behaviour before command mode existed
    show_all: bool,
    /// `:filter <glob>` — the typed pattern (for display) and its compiled
    /// matcher; `None` when no live filter is active
    path_filter: Option<(String, PathGlobs)>,
    /// the open command bar, if any — see `CommandBar`
    command: Option<CommandBar>,
    sel: usize,
    scroll: u16,
    /// horizontal scroll offset into the code pane's code column (the sign
    /// bar and line-number gutter never scroll) — column units, char-indexed
    hscroll: u16,
    why_scroll: u16,
    /// the why pane's selected-line cursor (an index into its logical lines,
    /// same units as `why_len`) — moved by `Next`/`Prev` when the why pane has
    /// focus, mirroring the code pane's `cursor`
    why_sel: usize,
    /// rendered line counts, filled in by `draw`, so the scroll actions can clamp
    code_len: usize,
    why_len: usize,
    /// code pane's visible row count, filled in by `draw`, so cursor motions can
    /// scroll to keep the cursor in view
    code_height: u16,
    /// why pane's visible row count, filled in by `draw`, so `why_sel` motions
    /// can scroll to keep it in view (mirrors `code_height`)
    why_height: u16,
    /// code pane's visible code-column width (pane width minus the sign bar
    /// and gutter), filled in by `draw`, so cursor motions can horizontally
    /// scroll to keep the cursor in view
    code_width: u16,
    focus: Pane,
    keys: Keymap,
    pending: Option<Key>,
    sources: Sources,
    highlights: Highlights,
    cursor: Cursor,
    popup: Option<Popup>,
    trees: HashMap<String, ParsedFile>,
    prompt: Option<Prompt>,
    search: Option<Search>,
    /// the commit the history section is relative to; None when it couldn't
    /// be resolved (history is then reported unavailable rather than guessed)
    review_sha: Option<String>,
    /// true when reviewing the uncommitted area (`zz`) or a `base..zz` /
    /// `base...zz` worktree range — there the worktree file IS the new
    /// content, so `ge`/C-o's line number is always exact; for a commit or
    /// commit range it may not be (see `edit_target`)
    uncommitted: bool,
    /// history-section lines, cached per (path, name, kind, scope) so
    /// repeated `K` on the same symbol doesn't re-shell-out to git
    history_cache: HashMap<(String, String, String, Option<String>), Vec<String>>,
    /// positions `JumpToEdge` jumped from, most-recent last; `JumpBack` pops
    /// one. Bounded by `JUMP_STACK_CAP`.
    jumps: Vec<(usize, Cursor)>,
    /// the reviewed rev string, as typed — part of a persisted mark's key
    /// (see `mark_key`)
    rev: String,
    /// where `ToggleReviewed` persists a mark; `None` when the repo root or
    /// cache dir couldn't be resolved, so toggles just aren't written
    marks_path: Option<PathBuf>,
    /// in-memory mirror of the mark file, kept in sync on every toggle
    marks: HashMap<u64, u64>,
    /// the five diff/selection background colours — `--theme`/`$ORDO_TUI_THEME`,
    /// carried across an `:e` reload since it's a display preference, not
    /// something indexed to the reviewed revision
    theme: Theme,
    /// `:group` — show a non-selectable group-reason header before each run
    /// of the reading-order list that shares a group id
    show_groups: bool,
    /// group id -> the engine's `Group::reason`, carried out of `load()`
    /// alongside `items` so headers can show it without re-touching the engine
    groups: HashMap<String, String>,
    /// what the filters and the engine dropped on the way here — `:audit`
    ledger: Ledger,
}

/// Bound on the position stack `JumpToEdge`/`JumpBack` maintain — generous
/// (vim's own default `'jumps'` is 100) without growing unbounded across a
/// long review session.
const JUMP_STACK_CAP: usize = 50;

/// Push a position, dropping the oldest entry first if the stack is already
/// at `JUMP_STACK_CAP`.
fn stack_push(stack: &mut Vec<(usize, Cursor)>, pos: (usize, Cursor)) {
    if stack.len() >= JUMP_STACK_CAP {
        stack.remove(0);
    }
    stack.push(pos);
}

/// Pop the most recent position whose item index still resolves against
/// `n_items` — skipping (never panicking on) any entry pointing at a hunk
/// index that's since gone out of range, so the stack survives even a jump to
/// a hunk that later becomes unreachable. `None` on an empty (or exhausted)
/// stack.
fn stack_pop_valid(stack: &mut Vec<(usize, Cursor)>, n_items: usize) -> Option<(usize, Cursor)> {
    while let Some(pos) = stack.pop() {
        if pos.0 < n_items {
            return Some(pos);
        }
    }
    None
}

fn build_items(out: &Output) -> Vec<Item> {
    let by_id: HashMap<&str, (&str, &ordo::model::HunkOut)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .map(move |h| (h.id.as_str(), (f.path.as_str(), h)))
        })
        .collect();
    let loc = |id: &str| {
        by_id
            .get(id)
            .map(|(p, h)| format!("{p}:L{}", h.new_range[0]))
            .unwrap_or_else(|| id.to_string())
    };
    // hunk id -> its index in the final `items` vec, so an edge can carry a
    // resolved target instead of just the id string it's thrown away for.
    // Same filter as the `filter_map` below (a hunk id absent from `by_id`
    // is skipped there too), so the index lines up with the vec it produces.
    let item_index: HashMap<&str, usize> = out
        .order
        .iter()
        .filter(|o| by_id.contains_key(o.hunk.as_str()))
        .enumerate()
        .map(|(i, o)| (o.hunk.as_str(), i))
        .collect();
    out.order
        .iter()
        .filter_map(|o| {
            let (path, h) = by_id.get(o.hunk.as_str())?;
            let cat = format!("{:?}", h.category).to_lowercase();
            let mark = if !h.advisories.is_empty() {
                "⚠ "
            } else if h.noise {
                "· "
            } else {
                ""
            };
            let edges = out
                .edges
                .iter()
                .filter(|e| e.from == h.id || e.to == h.id)
                .map(|e| {
                    let (label, target_id) = if e.from == h.id {
                        (format!("→ {}   {}", loc(&e.to), e.why), e.to.as_str())
                    } else {
                        (format!("← {}   {}", loc(&e.from), e.why), e.from.as_str())
                    };
                    EdgeRef { label, target: item_index.get(target_id).copied() }
                })
                .collect();
            Some(Item {
                path: path.to_string(),
                old_range: h.old_range,
                new_range: h.new_range,
                mark: mark.to_string(),
                cat: cat.clone(),
                rationale: h.rationale.clone(),
                details: h.details.clone(),
                notes: h.notes.clone(),
                edges,
                advisories: h
                    .advisories
                    .iter()
                    .map(|a| (a.construct.clone(), a.message.clone(), a.verdict))
                    .collect(),
                noise: h.noise,
                comment: h.comment,
                symbols: h.symbols.clone(),
                enclosing: h.enclosing.clone(),
                group: h.group.clone(),
                refined: ordo::refine::Refined::default(),
            })
        })
        .collect()
}

/// Fill in each item's intra-line refinement. One `Refiner` per file, not per
/// hunk: it parses both sides, which is the expensive part, and a file usually
/// carries several hunks.
fn refine_items(items: &mut [Item], sources: &Sources) {
    let mut by_path: HashMap<String, Option<ordo::refine::Refiner>> = HashMap::new();
    for it in items.iter_mut() {
        let Some((ol, nl)) = sources.get(&it.path) else { continue };
        let refiner = by_path
            .entry(it.path.clone())
            .or_insert_with(|| ordo::refine::Refiner::new(&it.path, ol, nl));
        if let Some(r) = refiner {
            it.refined = r.refine(it.old_range, it.new_range);
        }
    }
}

/// Which of `items` are currently visible, in order — the client-side view a
/// filter command narrows without touching `items` itself, so a filter can
/// always be widened back without re-running the engine or re-fetching
/// anything. Pure, so filter-change tests don't need a live `App`.
fn compute_view(
    items: &[Item],
    comments_only: bool,
    show_all: bool,
    glob: Option<&PathGlobs>,
) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| !comments_only || it.comment)
        .filter(|(_, it)| show_all || !it.noise)
        .filter(|(_, it)| glob.is_none_or(|g| g.is_match(&it.path)))
        .map(|(i, _)| i)
        .collect()
}

/// Why each currently-hidden item is hidden, attributed in `compute_view`'s own
/// order so the counts partition the hidden set exactly (an item hidden by two
/// filters is charged to the first). `unaccounted` must always be zero: it
/// counts items `compute_view` rejected for a reason this function does not
/// know about, which can only be a bug.
#[derive(Default, Debug, PartialEq, Eq)]
struct Hidden {
    comment: usize,
    noise: usize,
    glob: usize,
    unaccounted: usize,
}

fn hidden_breakdown(
    items: &[Item],
    comments_only: bool,
    show_all: bool,
    glob: Option<&PathGlobs>,
) -> Hidden {
    let mut h = Hidden::default();
    for it in items {
        if comments_only && !it.comment {
            h.comment += 1;
        } else if !show_all && it.noise {
            h.noise += 1;
        } else if glob.is_some_and(|g| !g.is_match(&it.path)) {
            h.glob += 1;
        }
    }
    let shown = compute_view(items, comments_only, show_all, glob).len();
    h.unaccounted = items.len() - shown - h.comment - h.noise - h.glob;
    h
}

// ---------------------------------------------------------- group headers

/// One row of the reading-order list's display, once `:group` is on: a
/// non-selectable header naming a group's reason, or a hunk (an index into
/// `App.items`, same as a plain `view` entry).
enum DisplayRow {
    Header(String),
    Item(usize),
}

/// The reading-order list's rows, in render order. `view` itself never grows
/// a header entry — it stays exactly the indices `compute_view` produced, so
/// navigation (`step`/`select`/`first_visible`/`last_visible`, all of which
/// walk `view`) can never land `sel` on one; this is purely a display-time
/// overlay, walked in lockstep by `display_row_of` below so a header can
/// never appear in one function's output but not the other's index math.
/// With `:group` off, it's `view` unchanged (one `DisplayRow::Item` each).
fn display_rows(
    view: &[usize],
    items: &[Item],
    groups: &HashMap<String, String>,
    show_groups: bool,
) -> Vec<DisplayRow> {
    if !show_groups {
        return view.iter().map(|&i| DisplayRow::Item(i)).collect();
    }
    let mut rows = Vec::with_capacity(view.len());
    let mut last: Option<&str> = None;
    for &i in view {
        let gid = items[i].group.as_str();
        if last != Some(gid) {
            let reason = groups.get(gid).map(String::as_str).unwrap_or(gid).to_string();
            rows.push(DisplayRow::Header(reason));
            last = Some(gid);
        }
        rows.push(DisplayRow::Item(i));
    }
    rows
}

/// The row index `display_rows` would give the hunk at `view[pos]` — how many
/// header rows precede it, plus its own position — so `ListState::select`
/// always points at an `Item` row, never a `Header` one. Doesn't need
/// `groups` (only `display_rows` renders a header's text) — same header
/// *placement* rule as `display_rows`, is all this needs to agree with it.
fn display_row_of(view: &[usize], items: &[Item], show_groups: bool, pos: usize) -> usize {
    if !show_groups {
        return pos;
    }
    let mut row = 0;
    let mut last: Option<&str> = None;
    for (i, &vi) in view.iter().enumerate() {
        let gid = items[vi].group.as_str();
        if last != Some(gid) {
            row += 1;
            last = Some(gid);
        }
        if i == pos {
            return row;
        }
        row += 1;
    }
    row
}

// position the code view so the hunk sits a few lines below the top
fn auto_scroll(it: &Item) -> u16 {
    let [o0, o1] = it.old_range;
    let removed = if o0 >= 1 && o0 <= o1 { o1 - o0 + 1 } else { 0 };
    ((it.new_range[0].saturating_sub(1) + removed).saturating_sub(3)) as u16
}

// where a freshly-selected hunk puts the cursor: its first changed new line
fn start_cursor(it: &Item) -> Cursor {
    Cursor {
        line: it.new_range[0].saturating_sub(1),
        col: 0,
    }
}

fn cursor_for(it: &Item, sources: &Sources) -> Cursor {
    match sources.get(&it.path) {
        Some((_, nl)) if !nl.is_empty() => clamp_cursor(start_cursor(it), nl),
        _ => Cursor { line: 0, col: 0 },
    }
}

// ------------------------------------------------------------------ cursor motion

// the last valid column on a line — 0 for an empty line, otherwise its last char
fn col_max(line: &str) -> usize {
    line.chars().count().saturating_sub(1)
}

fn clamp_cursor(mut c: Cursor, lines: &[String]) -> Cursor {
    if lines.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    c.line = c.line.min(lines.len() - 1);
    c.col = c.col.min(col_max(&lines[c.line]));
    c
}

fn move_col(c: Cursor, lines: &[String], delta: isize) -> Cursor {
    let max = col_max(&lines[c.line]);
    let col = (c.col as isize + delta).clamp(0, max as isize) as usize;
    Cursor { line: c.line, col }
}

fn move_line(c: Cursor, lines: &[String], delta: isize) -> Cursor {
    let last = lines.len() - 1;
    let line = (c.line as isize + delta).clamp(0, last as isize) as usize;
    Cursor {
        line,
        col: c.col.min(col_max(&lines[line])),
    }
}

fn line_start(c: Cursor, _lines: &[String]) -> Cursor {
    Cursor { line: c.line, col: 0 }
}

fn line_end(c: Cursor, lines: &[String]) -> Cursor {
    Cursor {
        line: c.line,
        col: col_max(&lines[c.line]),
    }
}

// { / } — nearest blank line, vim-style paragraph motion
fn para_prev(c: Cursor, lines: &[String]) -> Cursor {
    if c.line == 0 {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = c.line - 1;
    while i > 0 && !lines[i].trim().is_empty() {
        i -= 1;
    }
    Cursor { line: i, col: 0 }
}

fn para_next(c: Cursor, lines: &[String]) -> Cursor {
    let mut i = c.line + 1;
    while i < lines.len() && !lines[i].trim().is_empty() {
        i += 1;
    }
    Cursor {
        line: i.min(lines.len() - 1),
        col: 0,
    }
}

// word motion (w/b/e) is expressed over a single flattened char index — the
// file's lines joined by '\n' — so a word run crossing a line boundary is just
// a class transition, the same as one crossing whitespace mid-line.
fn word_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

fn joined_chars(lines: &[String]) -> Vec<char> {
    let mut v = vec![];
    for (i, l) in lines.iter().enumerate() {
        v.extend(l.chars());
        if i + 1 < lines.len() {
            v.push('\n');
        }
    }
    v
}

fn to_index(lines: &[String], c: Cursor) -> usize {
    let mut idx = 0;
    for l in &lines[..c.line] {
        idx += l.chars().count() + 1;
    }
    idx + c.col
}

fn from_index(lines: &[String], mut idx: usize) -> Cursor {
    for (i, l) in lines.iter().enumerate() {
        let len = l.chars().count();
        if idx <= len {
            return Cursor { line: i, col: idx };
        }
        idx -= len + 1;
    }
    let last = lines.len() - 1;
    Cursor {
        line: last,
        col: lines[last].chars().count(),
    }
}

fn word_next(c: Cursor, lines: &[String]) -> Cursor {
    let chars = joined_chars(lines);
    if chars.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = to_index(lines, c).min(chars.len() - 1);
    let cls = word_class(chars[i]);
    if cls != 0 {
        while i + 1 < chars.len() && word_class(chars[i + 1]) == cls {
            i += 1;
        }
    }
    while i + 1 < chars.len() && word_class(chars[i + 1]) == 0 {
        i += 1;
    }
    from_index(lines, (i + 1).min(chars.len() - 1))
}

fn word_prev(c: Cursor, lines: &[String]) -> Cursor {
    let chars = joined_chars(lines);
    if chars.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = to_index(lines, c).min(chars.len() - 1);
    if i == 0 {
        return Cursor { line: 0, col: 0 };
    }
    i -= 1;
    while i > 0 && word_class(chars[i]) == 0 {
        i -= 1;
    }
    let cls = word_class(chars[i]);
    while i > 0 && word_class(chars[i - 1]) == cls {
        i -= 1;
    }
    from_index(lines, i)
}

fn word_end(c: Cursor, lines: &[String]) -> Cursor {
    let chars = joined_chars(lines);
    if chars.is_empty() {
        return Cursor { line: 0, col: 0 };
    }
    let mut i = to_index(lines, c).min(chars.len() - 1);
    if i + 1 >= chars.len() {
        return from_index(lines, i);
    }
    i += 1;
    while i < chars.len() && word_class(chars[i]) == 0 {
        i += 1;
    }
    if i >= chars.len() {
        return from_index(lines, chars.len() - 1);
    }
    let cls = word_class(chars[i]);
    while i + 1 < chars.len() && word_class(chars[i + 1]) == cls {
        i += 1;
    }
    from_index(lines, i)
}

// keep `line` inside [scroll, scroll + height) by moving `scroll` the minimum
// amount necessary — the same rule any scrolling editor follows
fn follow_scroll(line: usize, scroll: u16, height: u16) -> u16 {
    let h = height.max(1) as usize;
    let s = scroll as usize;
    if line < s {
        line as u16
    } else if line >= s + h {
        (line + 1 - h) as u16
    } else {
        scroll
    }
}

// horizontal counterpart of `follow_scroll` — same rule, over columns and a
// pane's visible code-column width instead of rows and its visible height
fn follow_hscroll(col: usize, hscroll: u16, width: u16) -> u16 {
    follow_scroll(col, hscroll, width)
}

// ------------------------------------------------------------------- highlight

// tree-sitter highlight capture names we color, with their fg. The `Highlight`
// index a walk yields is the position of the matched name in this list.
const HL: &[(&str, Color)] = &[
    ("attribute", Color::Cyan),
    ("boolean", Color::Cyan),
    ("comment", Color::DarkGray),
    ("constant", Color::Cyan),
    ("constant.builtin", Color::Cyan),
    ("constructor", Color::Yellow),
    ("escape", Color::Cyan),
    ("function", Color::Blue),
    ("function.builtin", Color::Blue),
    ("function.method", Color::Blue),
    ("keyword", Color::Magenta),
    ("label", Color::Magenta),
    ("number", Color::Cyan),
    ("operator", Color::Gray),
    ("property", Color::LightBlue),
    ("punctuation", Color::Gray),
    ("punctuation.bracket", Color::Gray),
    ("punctuation.delimiter", Color::Gray),
    ("string", Color::Green),
    ("string.special", Color::Green),
    ("tag", Color::Blue),
    ("type", Color::Yellow),
    ("type.builtin", Color::Yellow),
    ("variable", Color::Reset),
    ("variable.builtin", Color::Red),
    ("variable.parameter", Color::LightRed),
];

type LineSpans = Vec<(String, Color)>;
type Highlights = HashMap<String, Vec<LineSpans>>;

// grammar + highlights query for a path (mirrors the engine's extension map;
// kept here because highlighting is a TUI-only presentation concern). The query
// is owned so cpp can inherit C's rules (Neovim `; inherits: c`, which
// tree-sitter-highlight doesn't resolve) by prepending the C query.
fn highlight_spec(path: &str) -> Option<(tree_sitter::Language, String)> {
    let ext = path.rsplit('.').next()?;
    let owned = |l: tree_sitter::Language, q: &str| (l, q.to_string());
    Some(match ext {
        "py" | "pyi" => owned(tree_sitter_python::LANGUAGE.into(), tree_sitter_python::HIGHLIGHTS_QUERY),
        "js" | "mjs" | "cjs" | "jsx" => {
            owned(tree_sitter_javascript::LANGUAGE.into(), tree_sitter_javascript::HIGHLIGHT_QUERY)
        }
        "rs" => owned(tree_sitter_rust::LANGUAGE.into(), tree_sitter_rust::HIGHLIGHTS_QUERY),
        "ts" | "mts" | "cts" => owned(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ),
        "tsx" => owned(tree_sitter_typescript::LANGUAGE_TSX.into(), tree_sitter_typescript::HIGHLIGHTS_QUERY),
        "go" => owned(tree_sitter_go::LANGUAGE.into(), tree_sitter_go::HIGHLIGHTS_QUERY),
        "c" | "h" => owned(tree_sitter_c::LANGUAGE.into(), tree_sitter_c::HIGHLIGHT_QUERY),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!("{}\n{}", tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY),
        ),
        "java" => owned(tree_sitter_java::LANGUAGE.into(), tree_sitter_java::HIGHLIGHTS_QUERY),
        "lua" => owned(tree_sitter_lua::LANGUAGE.into(), tree_sitter_lua::HIGHLIGHTS_QUERY),
        // the engine has no TOML grammar (nothing to order in a config file), but
        // manifests show up in most diffs and read badly unhighlighted
        "toml" => owned(
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        _ => return None,
    })
}

// Syntax-highlight `src` into per-line colored segments. None when the language
// is unsupported or the grammar/query fails to build → caller renders plain.
fn highlight_file(path: &str, src: &str) -> Option<Vec<LineSpans>> {
    let (language, query) = highlight_spec(path)?;
    let names: Vec<&str> = HL.iter().map(|(n, _)| *n).collect();
    let mut cfg = HighlightConfiguration::new(language, path, &query, "", "").ok()?;
    cfg.configure(&names);
    let mut hl = Highlighter::new();
    let events = hl.highlight(&cfg, src.as_bytes(), None, |_| None).ok()?;

    let mut lines: Vec<LineSpans> = vec![vec![]];
    let mut stack: Vec<Color> = vec![];
    for ev in events {
        match ev.ok()? {
            HighlightEvent::HighlightStart(h) => {
                stack.push(HL.get(h.0).map(|(_, c)| *c).unwrap_or(Color::Reset));
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let color = stack.last().copied().unwrap_or(Color::Reset);
                let mut first = true;
                for piece in src.get(start..end).unwrap_or("").split('\n') {
                    if !first {
                        lines.push(vec![]);
                    }
                    first = false;
                    if !piece.is_empty() {
                        lines.last_mut().unwrap().push((piece.to_string(), color));
                    }
                }
            }
        }
    }
    Some(lines)
}

// ------------------------------------------------------------------- code view

// Diff backgrounds/bars (Neovim gitsigns style): a colored sign bar plus a
// subtle full-width background tint, with the code itself syntax-colored.
// Every foreground colour in this file is a named ANSI colour and already
// follows the terminal's own palette; these five are the only truecolor
// values, and the only ones that assume a dark terminal background — hence
// `Theme`, selected once at launch (`--theme`/`$ORDO_TUI_THEME`) rather than
// detected (OSC 11 background-colour queries aren't reliably supported
// across terminals, so an explicit flag is the point, not a fallback).
#[derive(Clone, Copy)]
struct Theme {
    add_bg: Color,
    del_bg: Color,
    /// the *changed part* of a line whose counterpart was identified — the
    /// line keeps the quiet add/del tint, and only what actually changed gets
    /// these (Neovim's DiffText over DiffChange)
    add_strong_bg: Color,
    del_strong_bg: Color,
    /// reading-order list's selected-row tint — a neutral slate, subtle next
    /// to add_bg/del_bg rather than the full fg/bg swap REVERSED gives
    select_bg: Color,
    /// search-match backgrounds — current match brighter than the rest, so
    /// it reads as "here" among however many others are also highlighted
    match_bg: Color,
    match_cur_bg: Color,
}

impl Theme {
    fn dark() -> Theme {
        Theme {
            add_bg: Color::Rgb(20, 40, 25),
            del_bg: Color::Rgb(50, 24, 28),
            add_strong_bg: Color::Rgb(34, 84, 46),
            del_strong_bg: Color::Rgb(104, 40, 46),
            select_bg: Color::Rgb(45, 50, 62),
            match_bg: Color::Rgb(70, 60, 10),
            match_cur_bg: Color::Rgb(140, 110, 15),
        }
    }

    // pale tints of the same hues, at light-terminal weight — not the dark
    // values inverted, which would read as loud on a light background
    fn light() -> Theme {
        Theme {
            add_bg: Color::Rgb(214, 240, 218),
            del_bg: Color::Rgb(248, 214, 214),
            add_strong_bg: Color::Rgb(160, 220, 175),
            del_strong_bg: Color::Rgb(245, 174, 174),
            select_bg: Color::Rgb(222, 226, 236),
            match_bg: Color::Rgb(255, 236, 170),
            match_cur_bg: Color::Rgb(255, 202, 68),
        }
    }
}

fn theme(name: &str) -> Option<Theme> {
    match name {
        "dark" => Some(Theme::dark()),
        "light" => Some(Theme::light()),
        _ => None,
    }
}

const BAR: &str = "▎";

// Re-style the char range [start, end) of the whole line's rendered spans
// (prefix included), splitting spans at the boundaries as needed. `style_fn`
// maps a span's existing style to its overlaid one, so the caller decides
// whether to tint a background, reverse it, etc.
fn overlay_range(
    spans: Vec<Span<'static>>,
    start: usize,
    end: usize,
    style_fn: impl Fn(Style) -> Style,
) -> Vec<Span<'static>> {
    if start >= end {
        return spans;
    }
    let mut consumed = 0;
    let mut out = Vec::with_capacity(spans.len() + 2);
    for sp in spans {
        let text = sp.content.to_string();
        let len = text.chars().count();
        let (seg_start, seg_end) = (consumed, consumed + len);
        if end <= seg_start || start >= seg_end {
            out.push(sp);
        } else {
            let local_start = start.saturating_sub(seg_start).min(len);
            let local_end = end.saturating_sub(seg_start).min(len);
            let mut chars = text.chars();
            let before: String = chars.by_ref().take(local_start).collect();
            let mid: String = chars.by_ref().take(local_end - local_start).collect();
            let after: String = chars.collect();
            if !before.is_empty() {
                out.push(Span::styled(before, sp.style));
            }
            if !mid.is_empty() {
                out.push(Span::styled(mid, style_fn(sp.style)));
            }
            if !after.is_empty() {
                out.push(Span::styled(after, sp.style));
            }
        }
        consumed += len;
    }
    out
}

// Crop `spans` to the char range [start, start + width) — the horizontal-
// scroll counterpart of `overlay_range`'s boundary splitting: same walk over
// char-counted spans, but dropping what falls outside the window instead of
// restyling what falls inside it.
fn slice_range(spans: Vec<Span<'static>>, start: usize, width: usize) -> Vec<Span<'static>> {
    let end = start + width;
    if start >= end {
        return vec![];
    }
    let mut consumed = 0;
    let mut out = Vec::with_capacity(spans.len());
    for sp in spans {
        let text = sp.content.to_string();
        let len = text.chars().count();
        let (seg_start, seg_end) = (consumed, consumed + len);
        consumed += len;
        if end <= seg_start || start >= seg_end {
            continue;
        }
        let local_start = start.saturating_sub(seg_start).min(len);
        let local_end = end.saturating_sub(seg_start).min(len);
        if local_end > local_start {
            let mid: String = text.chars().skip(local_start).take(local_end - local_start).collect();
            out.push(Span::styled(mid, sp.style));
        }
    }
    out
}

// Reverse-style the char at `target` (a char index into the whole line's
// rendered spans, prefix included) — the cursor cell. Past the last rendered
// char (an empty line, or a column beyond it) it appends one blank reversed
// cell so the cursor is still visible.
fn overlay_cursor(spans: Vec<Span<'static>>, target: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if target >= total {
        let mut out = spans;
        out.push(Span::styled(
            " ",
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        return out;
    }
    overlay_range(spans, target, target + 1, |s| s.add_modifier(Modifier::REVERSED))
}

// The whole new file with the changed hunk highlighted in place: removed lines
// on a red-tinted row (shown at the change point), added lines on a green tint,
// the rest plain context. Code is syntax-highlighted via tree-sitter.
// prefix width: 1-char sign bar + 4-digit line number + 1 space
const GUTTER_W: usize = 1 + 5;

// rendering knobs (pane width, scroll offsets, cursor, active search) rather
// than a natural struct's worth of related data — bundling them wouldn't
// clarify the call sites below, just move the same count of fields around
#[allow(clippy::too_many_arguments)]
fn code_view(
    it: &Item,
    sources: &Sources,
    highlights: &Highlights,
    width: usize,
    hscroll: usize,
    cursor: Option<Cursor>,
    matches: &[(usize, usize, usize)],
    cur_match: Option<usize>,
    theme: &Theme,
) -> (Vec<Line<'static>>, bool) {
    let mut out = vec![];
    // shared (not exclusively-borrowed) so both `window` and `emit_removed`
    // below can set it without fighting over a unique borrow across the
    // whole function body
    let right_clip = std::cell::Cell::new(false);
    let Some((ol, nl)) = sources.get(&it.path) else {
        return (out, right_clip.get());
    };
    let hl = highlights.get(&it.path);
    let [o0, o1] = it.old_range;
    let [n0, n1] = it.new_range;
    let removed: Vec<&String> = if o0 >= 1 && o0 <= o1 && o1 <= ol.len() {
        ol[o0 - 1..o1].iter().collect()
    } else {
        vec![]
    };
    let num = Style::default().fg(Color::DarkGray);
    let avail = width.saturating_sub(GUTTER_W);
    // fill the rest of the row so the background tint spans the full width
    let pad = |spans: &mut Vec<Span<'static>>, used: usize, bg: Color| {
        if width > used {
            spans.push(Span::styled(" ".repeat(width - used), Style::default().bg(bg)));
        }
    };
    // slice `content` (the code portion only — never the gutter built ahead
    // of it) to the horizontally visible window, reporting whether this line
    // has anything past the right edge of that window (the caller folds that
    // into `right_clip` — kept a plain function, not a mutably-capturing
    // closure, so it can be called from inside another closure below)
    let window = |content: Vec<Span<'static>>, len: usize| -> (Vec<Span<'static>>, usize, bool) {
        let clipped = len > hscroll + avail;
        let shown = len.saturating_sub(hscroll).min(avail);
        (slice_range(content, hscroll, avail), shown, clipped)
    };
    // a line paired with its counterpart (see `ordo::refine`) keeps the quiet
    // tint and gets the strong one only where it actually differs; an unpaired
    // line has no counterpart to compare against and tints whole
    let emphasize = |mut spans: Vec<Span<'static>>,
                     refined: Option<&Vec<(usize, usize)>>,
                     bg: Color|
     -> Vec<Span<'static>> {
        for &(cs, ce) in refined.map(|v| v.as_slice()).unwrap_or(&[]) {
            if ce <= hscroll || cs >= hscroll + avail {
                continue;
            }
            let (ls, le) = (cs.saturating_sub(hscroll), (ce - hscroll).min(avail));
            spans = overlay_range(spans, GUTTER_W + ls, GUTTER_W + le, |st| st.bg(bg));
        }
        spans
    };
    let emit_removed = |out: &mut Vec<Line<'static>>| {
        for (k, r) in removed.iter().enumerate() {
            let content = vec![Span::styled((*r).clone(), Style::default().fg(Color::Red).bg(theme.del_bg))];
            let (visible, shown, clipped) = window(content, r.chars().count());
            right_clip.set(right_clip.get() | clipped);
            let mut spans = vec![
                Span::styled(BAR, Style::default().fg(Color::Red)),
                Span::styled("     ".to_string(), num.bg(theme.del_bg)),
            ];
            spans.extend(visible);
            pad(&mut spans, GUTTER_W + shown, theme.del_bg);
            spans = emphasize(spans, it.refined.removed.get(k).and_then(|s| s.as_ref()), theme.del_strong_bg);
            out.push(Line::from(spans));
        }
    };
    for (i, line) in nl.iter().enumerate() {
        let ln = i + 1;
        if ln == n0 {
            emit_removed(&mut out);
        }
        let added = n0 <= ln && ln <= n1;
        let bg = if added { theme.add_bg } else { Color::Reset };
        let mut spans = vec![
            Span::styled(
                if added { BAR } else { " " },
                Style::default().fg(Color::Green),
            ),
            Span::styled(format!("{ln:>4} "), num.bg(bg)),
        ];
        // syntax-colored code segments (fall back to the raw line if unhighlighted)
        let content: Vec<Span<'static>> = match hl.and_then(|h| h.get(i)) {
            Some(segs) if !segs.is_empty() => segs
                .iter()
                .map(|(text, color)| Span::styled(text.clone(), Style::default().fg(*color).bg(bg)))
                .collect(),
            _ => vec![Span::styled(line.clone(), Style::default().bg(bg))],
        };
        let (visible, shown, clipped) = window(content, line.chars().count());
        right_clip.set(right_clip.get() | clipped);
        spans.extend(visible);
        if added {
            pad(&mut spans, GUTTER_W + shown, bg);
            let k = ln - n0;
            spans = emphasize(spans, it.refined.added.get(k).and_then(|s| s.as_ref()), theme.add_strong_bg);
        }
        for (mi, &(ml, s, e)) in matches.iter().enumerate() {
            // only a match that intersects the visible horizontal window can
            // be shown at all — one further off-screen is reached by jumping
            // to it (`n`/`N`, `*`/`#`), which scrolls the window to include it
            if ml != i || e <= hscroll || s >= hscroll + avail {
                continue;
            }
            let (ls, le) = (s.saturating_sub(hscroll), (e - hscroll).min(avail));
            let mbg = if Some(mi) == cur_match { theme.match_cur_bg } else { theme.match_bg };
            spans = overlay_range(spans, GUTTER_W + ls, GUTTER_W + le, |st| st.bg(mbg));
        }
        if let Some(c) = cursor.filter(|c| c.line == i && c.col >= hscroll && c.col < hscroll + avail) {
            spans = overlay_cursor(spans, GUTTER_W + (c.col - hscroll));
        }
        out.push(Line::from(spans));
    }
    if n0 > nl.len() {
        emit_removed(&mut out); // deletion at/after EOF
    }
    (out, right_clip.get())
}

// ---------------------------------------------------------------------- hover

fn node_text(n: Node, src: &str) -> String {
    src.get(n.start_byte()..n.end_byte()).unwrap_or("").to_string()
}

// byte offset of a char column within one line — tree-sitter Points are byte-indexed
fn char_byte(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map(|(b, _)| b).unwrap_or(line.len())
}

// node kinds counted as a "definition" worth showing, by file extension —
// deliberately a small, curated set rather than every grammar's declaration
// kinds, so a hover only ever lands on something with a clear signature/body.
fn def_kinds(path: &str) -> &'static [&'static str] {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" => &["function_definition", "class_definition"],
        "rs" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
            "const_item",
            "static_item",
            "type_item",
        ],
        "js" | "jsx" | "mjs" | "cjs" => {
            &["function_declaration", "class_declaration", "method_definition"]
        }
        "ts" | "tsx" | "mts" | "cts" => &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
        ],
        "go" => &["function_declaration", "method_declaration", "type_declaration"],
        "c" | "h" => &["function_definition", "struct_specifier", "enum_specifier"],
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => {
            &["function_definition", "class_specifier", "struct_specifier"]
        }
        "java" => &["method_declaration", "class_declaration", "interface_declaration"],
        _ => &[],
    }
}

// A definition's name — the `name` field where the grammar has one; C/C++
// function definitions don't (the identifier is buried in `declarator`), so
// fall back to hunting one down there, skipping the parameter list.
fn def_name(n: Node, src: &str) -> Option<String> {
    if let Some(name) = n.child_by_field_name("name") {
        return Some(node_text(name, src));
    }
    find_identifier(n.child_by_field_name("declarator")?, src)
}

fn find_identifier(n: Node, src: &str) -> Option<String> {
    if n.kind() == "identifier" || n.kind() == "field_identifier" {
        return Some(node_text(n, src));
    }
    let mut cursor = n.walk();
    let children: Vec<Node> = n.children(&mut cursor).collect();
    children
        .into_iter()
        .filter(|c| !c.kind().contains("parameter"))
        .find_map(|c| find_identifier(c, src))
}

// depth-first search of the whole tree for a definition-kind node named `name`
fn find_definition<'a>(root: Node<'a>, kinds: &[&str], name: &str, src: &str) -> Option<Node<'a>> {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if kinds.contains(&n.kind()) && def_name(n, src).as_deref() == Some(name) {
            return Some(n);
        }
        let mut cursor = n.walk();
        stack.extend(n.children(&mut cursor));
    }
    None
}

// the def's header — its own text up to (not including) its body, so a
// multi-line signature still reads as just the signature
fn signature(n: Node, src: &str) -> String {
    let end = n
        .child_by_field_name("body")
        .map(|b| b.start_byte())
        .unwrap_or(n.end_byte());
    src.get(n.start_byte()..end).unwrap_or("").trim_end().to_string()
}

fn python_docstring(n: Node, src: &str) -> Option<String> {
    let body = n.child_by_field_name("body")?;
    let first = body.named_child(0)?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let expr = first.named_child(0)?;
    (expr.kind() == "string").then(|| node_text(expr, src))
}

// the run of `//` / `/** */` comment nodes immediately preceding the
// definition, stopping at the first blank line or non-comment sibling
fn leading_comment(n: Node, src: &str) -> Option<String> {
    let mut lines = vec![];
    let mut cur = n.prev_sibling();
    let mut expect_row = n.start_position().row;
    while let Some(c) = cur {
        if !c.kind().contains("comment") || c.end_position().row + 1 < expect_row {
            break;
        }
        lines.push(node_text(c, src).trim_end().to_string());
        expect_row = c.start_position().row;
        cur = c.prev_sibling();
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

fn doc_for(path: &str, n: Node, src: &str) -> Option<String> {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" => python_docstring(n, src),
        _ => leading_comment(n, src),
    }
}

// Ensure `path`'s new-content is parsed and cached, returning the cached tree.
// Shared by `hover` and the symbol-occurrence search so both cache the same tree.
fn parse_cached<'a>(
    trees: &'a mut HashMap<String, ParsedFile>,
    path: &str,
    nl: &[String],
    lang: tree_sitter::Language,
) -> Option<&'a ParsedFile> {
    if !trees.contains_key(path) {
        let src = nl.join("\n");
        let mut parser = Parser::new();
        parser.set_language(&lang).ok()?;
        let tree = parser.parse(&src, None)?;
        trees.insert(path.to_string(), ParsedFile { tree, src });
    }
    trees.get(path)
}

// The identifier node at `cursor`, widening from whatever node is directly
// under it until an `identifier`-kind ancestor is found.
fn identifier_at<'a>(parsed: &'a ParsedFile, nl: &[String], cursor: Cursor) -> Option<Node<'a>> {
    let point = Point {
        row: cursor.line,
        column: char_byte(&nl[cursor.line], cursor.col),
    };
    let mut node = parsed.tree.root_node().descendant_for_point_range(point, point)?;
    while !node.kind().contains("identifier") {
        node = node.parent()?;
    }
    Some(node)
}

/// `K` / `F12` — resolve the symbol under the cursor via tree-sitter (not by
/// scanning characters) and show its definition in a popup. Parses lazily,
/// caching the tree per path so repeated presses are cheap.
fn hover(app: &mut App) {
    let path = app.items[app.sel].path.clone();
    let Some((_, nl)) = app.sources.get(&path) else {
        return;
    };
    if nl.is_empty() {
        return;
    }
    let Some((lang, _)) = highlight_spec(&path) else {
        app.popup = Some(Popup {
            title: "hover".to_string(),
            lines: vec![prose("no grammar available for this file type")],
            scroll: 0,
            hscroll: 0,
        });
        return;
    };
    let Some(parsed) = parse_cached(&mut app.trees, &path, nl, lang) else {
        return;
    };
    let Some(node) = identifier_at(parsed, nl, app.cursor) else {
        app.popup = Some(Popup {
            title: "hover".to_string(),
            lines: vec![prose("no symbol here")],
            scroll: 0,
            hscroll: 0,
        });
        return;
    };
    let name = node_text(node, &parsed.src);
    let root = parsed.tree.root_node();
    let kinds = def_kinds(&path);
    // `def_id` carries owned data (kind, row, the exact source the tree was
    // built from) out of this branch so the `app.trees` borrow behind `parsed`
    // ends here — `history_lines` below needs `&mut app`.
    let (mut lines, def_id) = if kinds.is_empty() {
        (
            vec!["definition lookup not supported for this file type".to_string()],
            None,
        )
    } else if let Some(def) = find_definition(root, kinds, &name, &parsed.src) {
        let mut lines = vec![format!("kind: {}", def.kind()), String::new()];
        lines.extend(signature(def, &parsed.src).lines().map(str::to_string));
        if let Some(doc) = doc_for(&path, def, &parsed.src) {
            lines.push(String::new());
            lines.extend(doc.lines().map(str::to_string));
        }
        let id = (def.kind().to_string(), def.start_position().row, parsed.src.clone());
        (lines, Some(id))
    } else {
        (vec!["not defined in this file".to_string()], None)
    };
    // history only makes sense for a symbol whose definition resolved in the
    // current file — an unresolved lookup has nothing to look up history for
    if let Some((kind, row, content)) = def_id {
        lines.push(String::new());
        lines.extend(history_lines(app, &path, &name, &kind, row, &content));
    }
    let lines = lines.into_iter().map(prose).collect();
    app.popup = Some(Popup { title: name, lines, scroll: 0, hscroll: 0 });
}

// ------------------------------------------------------------ cross-commit history

// bound on how many candidate commits per direction (earlier/later) get run
// through the engine — a long-lived file's full history would otherwise stall
// the UI on a single `K` press
const HISTORY_WINDOW: usize = 10;

fn commit_list(args: &[&str]) -> Vec<String> {
    git(args)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// `scope.name`, or bare `name` at top level — matches the qualified form a
/// hunk's own `enclosing` field carries, so a body-edit match (found via
/// `enclosing`) and a defines-match (found via symbol identity) read the
/// same string in `label_for`.
fn qualified_name(sym: &Symbol) -> String {
    match &sym.scope {
        Some(s) => format!("{s}.{}", sym.name),
        None => sym.name.clone(),
    }
}

/// The project's identity rule, verbatim: "tree sitter type + scope for the
/// symbol must match, otherwise it's a different symbol". Name alone is never
/// enough — a method `run` on class `A` and a module-level `run` don't match,
/// nor do two same-named, same-scope defs of different tree-sitter kinds.
fn symbol_eq(a: &Symbol, b: &Symbol) -> bool {
    a.name == b.name && a.kind == b.kind && a.scope == b.scope
}

/// Bound `git rev-list <rev> -- <file>`'s output (`rev` itself first, if it
/// touched the file, then its nearest ancestor, then the next, ...) to the
/// nearest `window` ancestors, oldest-of-the-window first — so the entry
/// nearest `current` prints right above the CURRENT line.
fn bound_earlier(mut shas: Vec<String>, current: &str, window: usize) -> Vec<String> {
    if shas.first().map(String::as_str) == Some(current) {
        shas.remove(0);
    }
    shas.truncate(window);
    shas.reverse();
    shas
}

/// Bound `git rev-list <rev>..HEAD -- <file>`'s output (HEAD first, walking
/// back to the nearest descendant of `rev` last) to the nearest `window`
/// descendants, nearest-to-`rev` first — so the entry nearest `rev` prints
/// right below the CURRENT line.
fn bound_later(mut shas: Vec<String>, window: usize) -> Vec<String> {
    shas.reverse();
    shas.truncate(window);
    shas
}

/// The engine's identity (name + tree-sitter kind + qualified scope) for the
/// definition named `name` (tree-sitter kind `kind`) at 0-based `row` in
/// `content`. Runs the engine as though the whole file were freshly added, so
/// every definition in it — not just ones inside a hunk that happens to be
/// selected — shows up in some hunk's `symbols`, keyed to its own row.
fn symbol_identity(path: &str, content: &str, name: &str, kind: &str, row: usize) -> Option<Symbol> {
    let out = ordo::run(Input {
        changes: vec![Change {
            path: path.to_string(),
            old: Some(String::new()),
            new: Some(content.to_string()),
            diff: None,
        }],
        options: Options::default(),
    });
    let file = out.files.iter().find(|f| f.path == path)?;
    let line = row + 1;
    file.hunks
        .iter()
        .find(|h| h.new_range[0] <= line && line <= h.new_range[1])
        .and_then(|h| h.symbols.iter().find(|s| s.name == name && s.kind == kind))
        // a hunk-boundary mismatch in the synthetic whole-file diff shouldn't
        // lose the def entirely — any hunk carrying the right name+kind is it
        .or_else(|| {
            file.hunks
                .iter()
                .flat_map(|h| h.symbols.iter())
                .find(|s| s.name == name && s.kind == kind)
        })
        .cloned()
}

/// Pull the fragment of a (possibly multi-symbol) rationale that names this
/// symbol — "adds helper; changes signature of run" reads as just "changes
/// signature of run" for `run`'s own history line. Falls back to the whole
/// rationale, which covers the enclosing-only body-edit case ("edits
/// A.run") where the fragment already names nothing else.
fn label_for(h: &HunkOut, target: &Symbol) -> String {
    let qualified = qualified_name(target);
    h.rationale
        .split("; ")
        .find(|frag| frag.contains(target.name.as_str()) || frag.contains(qualified.as_str()))
        .unwrap_or(&h.rationale)
        .to_string()
}

/// What commit `sha` did to `target`, in ordo's own rationale wording — or
/// None when this commit touched `path` but never reached `target` itself (a
/// path-based window can't help but include unrelated changes to the file;
/// see `earlier`/`later` in `compute_history`).
fn classify_commit(sha: &str, path: &str, target: &Symbol) -> Option<String> {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{sha}^")]);
    let parent = parent.trim();
    let old = if parent.is_empty() {
        String::new()
    } else {
        git(&["show", &format!("{parent}:{path}")])
    };
    let new = git(&["show", &format!("{sha}:{path}")]);
    let out = ordo::run(Input {
        changes: vec![Change {
            path: path.to_string(),
            old: Some(old),
            new: Some(new),
            diff: None,
        }],
        options: Options::default(),
    });
    let file = out.files.iter().find(|f| f.path == path)?;
    let qualified = qualified_name(target);
    file.hunks.iter().find_map(|h| {
        let defines_it = h.symbols.iter().any(|s| symbol_eq(s, target));
        let body_edit = h.enclosing.as_deref() == Some(qualified.as_str());
        (defines_it || body_edit).then(|| label_for(h, target))
    })
}

// Direction semantics (decided): `earlier` = ancestors of the reviewed rev,
// `later` = descendants — topological, not date-based, so this is correct
// when reviewing an old or mid-stack commit. No `--follow`: `git rev-list`
// doesn't support it at all (only `git log` does — passing it errors), so a
// rename before the window's oldest commit silently ends this symbol's
// history there. This engine call is single-file, one commit at a time, so it
// sees only same-file edits — a call site added in another file never shows.
fn compute_history(review_sha: &str, path: &str, target: &Symbol) -> Vec<String> {
    let earlier_all = commit_list(&["rev-list", review_sha, "--", path]);
    let earlier = bound_earlier(earlier_all, review_sha, HISTORY_WINDOW);
    let later_all = commit_list(&["rev-list", &format!("{review_sha}..HEAD"), "--", path]);
    let later = bound_later(later_all, HISTORY_WINDOW);

    let mut out = vec![];
    let mut first = true;
    for sha in &earlier {
        if let Some(label) = classify_commit(sha, path, target) {
            let tag = if first { "earlier" } else { "" };
            first = false;
            out.push(format!("{:<7}  {}  {label}", tag, short_sha(sha)));
        }
    }
    out.push(format!("{:<7}  {}", "CURRENT", short_sha(review_sha)));
    let mut first = true;
    for sha in &later {
        if let Some(label) = classify_commit(sha, path, target) {
            let tag = if first { "later" } else { "" };
            first = false;
            out.push(format!("{:<7}  {}  {label}", tag, short_sha(sha)));
        }
    }
    out
}

/// The `K` popup's history section: resolves the hovered def's identity, then
/// its cross-commit history (cached per identity so a repeat `K` is instant).
/// Computed synchronously on this keypress — no async restructure, kept fast
/// by `HISTORY_WINDOW` and the single-file-per-commit engine calls.
fn history_lines(
    app: &mut App,
    path: &str,
    name: &str,
    kind: &str,
    row: usize,
    content: &str,
) -> Vec<String> {
    let Some(review_sha) = app.review_sha.clone() else {
        return vec!["history: unavailable (no commit to review from)".to_string()];
    };
    let Some(target) = symbol_identity(path, content, name, kind, row) else {
        return vec!["history: could not resolve this symbol's identity".to_string()];
    };
    let key = (path.to_string(), target.name.clone(), target.kind.clone(), target.scope.clone());
    if let Some(cached) = app.history_cache.get(&key) {
        return cached.clone();
    }
    let lines = compute_history(&review_sha, path, &target);
    app.history_cache.insert(key, lines.clone());
    lines
}

// --------------------------------------------------------------------- search

// inverse of `char_byte`: the char index a byte offset falls at within one line
fn byte_to_char_col(line: &str, byte_col: usize) -> usize {
    line.char_indices().take_while(|(b, _)| *b < byte_col).count()
}

/// Every identifier node in the tree whose text equals `name`, as document-order
/// (line, start_col, end_col) char ranges. Never a text scan: a short name that
/// occurs as a substring of a longer identifier (`may_refine` inside
/// `may_refine_camber_span`) is never counted, because node text equality is
/// exact, not a substring test.
fn symbol_matches(root: Node, name: &str, src: &str, lines: &[String]) -> Vec<(usize, usize, usize)> {
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind().contains("identifier") && node_text(n, src) == name {
            let (sp, ep) = (n.start_position(), n.end_position());
            if sp.row == ep.row && sp.row < lines.len() {
                let scol = byte_to_char_col(&lines[sp.row], sp.column);
                let ecol = byte_to_char_col(&lines[sp.row], ep.column);
                out.push((sp.row, scol, ecol));
            }
        }
        let mut cursor = n.walk();
        stack.extend(n.children(&mut cursor));
    }
    out.sort();
    out
}

/// `/` text search: literal substring occurrences of `pattern`, one per
/// character position. This IS a text scan — the one place that's correct,
/// since the user is typing characters, not asking about a symbol.
fn text_matches(lines: &[String], pattern: &str) -> Vec<(usize, usize, usize)> {
    if pattern.is_empty() {
        return vec![];
    }
    let plen = pattern.chars().count();
    let mut out = vec![];
    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < plen {
            continue;
        }
        for start in 0..=chars.len() - plen {
            if chars[start..start + plen].iter().collect::<String>() == pattern {
                out.push((i, start, start + plen));
            }
        }
    }
    out
}

// first match at/after (inclusive) or strictly after (!inclusive) `cursor`,
// wrapping to the first match when nothing qualifies
fn seek_forward(matches: &[(usize, usize, usize)], cursor: Cursor, inclusive: bool) -> Option<usize> {
    let after = |l: usize, s: usize| {
        if inclusive {
            (l, s) >= (cursor.line, cursor.col)
        } else {
            (l, s) > (cursor.line, cursor.col)
        }
    };
    matches
        .iter()
        .position(|&(l, s, _)| after(l, s))
        .or((!matches.is_empty()).then_some(0))
}

// mirror of `seek_forward`, wrapping to the last match when nothing qualifies
fn seek_backward(matches: &[(usize, usize, usize)], cursor: Cursor, inclusive: bool) -> Option<usize> {
    let before = |l: usize, s: usize| {
        if inclusive {
            (l, s) <= (cursor.line, cursor.col)
        } else {
            (l, s) < (cursor.line, cursor.col)
        }
    };
    matches
        .iter()
        .rposition(|&(l, s, _)| before(l, s))
        .or_else(|| matches.len().checked_sub(1))
}

// `n`/`N` index arithmetic: advance/retreat by one, wrapping. `len == 0` never
// panics (rem_euclid by zero would) — it's the empty-result case.
fn cycle_index(index: usize, len: usize, dir: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + dir).rem_euclid(len as isize) as usize
}

// move the cursor to `search`'s current match and scroll to follow, then store it
fn jump_search(app: &mut App, mut search: Search, forward: bool, inclusive: bool) {
    if !search.matches.is_empty() {
        let idx = if forward {
            seek_forward(&search.matches, app.cursor, inclusive)
        } else {
            seek_backward(&search.matches, app.cursor, inclusive)
        };
        search.index = idx.unwrap_or(0);
        let (line, col, _) = search.matches[search.index];
        app.cursor = Cursor { line, col };
        app.scroll = follow_scroll(line, app.scroll, app.code_height);
        app.hscroll = follow_hscroll(col, app.hscroll, app.code_width);
    }
    app.search = Some(search);
}

/// `*` / `#` — the symbol under the cursor, resolved via tree-sitter (sharing
/// the `K` popup's parse cache), jumping to the next/previous occurrence.
fn symbol_search(app: &mut App, forward: bool) {
    let path = app.items[app.sel].path.clone();
    let Some((_, nl)) = app.sources.get(&path) else {
        return;
    };
    if nl.is_empty() {
        return;
    }
    let Some((lang, _)) = highlight_spec(&path) else {
        return;
    };
    let Some(parsed) = parse_cached(&mut app.trees, &path, nl, lang) else {
        return;
    };
    let Some(node) = identifier_at(parsed, nl, app.cursor) else {
        return;
    };
    let name = node_text(node, &parsed.src);
    let matches = symbol_matches(parsed.tree.root_node(), &name, &parsed.src, nl);
    let search = Search {
        kind: SearchKind::Symbol,
        pattern: name,
        matches,
        index: 0,
    };
    // strict inequality: the occurrence the cursor is already on doesn't count
    // as "next"
    jump_search(app, search, forward, false);
}

/// `n` / `N` — cycle through the currently active search's matches, wrapping.
fn cycle_search(app: &mut App, dir: isize) {
    let Some(search) = app.search.as_mut() else {
        return;
    };
    if search.matches.is_empty() {
        return;
    }
    search.index = cycle_index(search.index, search.matches.len(), dir);
    let (line, col, _) = search.matches[search.index];
    app.cursor = Cursor { line, col };
    app.scroll = follow_scroll(line, app.scroll, app.code_height);
    app.hscroll = follow_hscroll(col, app.hscroll, app.code_width);
}

/// `Enter` on the `/` prompt: run the text search and jump to the first match
/// at or after the cursor.
fn accept_search(app: &mut App) {
    let Some(prompt) = app.prompt.take() else {
        return;
    };
    let path = app.items[app.sel].path.clone();
    let matches = match app.sources.get(&path) {
        Some((_, nl)) => text_matches(nl, &prompt.text),
        None => vec![],
    };
    let search = Search {
        kind: SearchKind::Text,
        pattern: prompt.text,
        matches,
        index: 0,
    };
    jump_search(app, search, true, true);
}

/// `Esc` on the `/` prompt: cancel, restoring the cursor position from before
/// the prompt opened.
fn cancel_search(app: &mut App) {
    if let Some(prompt) = app.prompt.take() {
        app.cursor = prompt.anchor;
        app.scroll = follow_scroll(app.cursor.line, app.scroll, app.code_height);
        app.hscroll = follow_hscroll(app.cursor.col, app.hscroll, app.code_width);
    }
}

// ----------------------------------------------------------------------- edit

/// `$VISUAL` then `$EDITOR`, whitespace-split so trailing arguments (`code
/// --wait`, `emacsclient -nw`) survive; `vi` when neither is set or both are
/// blank — a POSIX-guaranteed binary rather than a guess.
fn resolve_editor() -> Vec<String> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vi".to_string());
    split_command(&raw)
}

fn split_command(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// The location arguments for one program, keyed by its basename (not the
/// full `$EDITOR` string, which may carry flags ahead of the program). An
/// editor outside this curated set gets just the file — passing a guessed
/// `+LINE` or `file:LINE` syntax to something that doesn't understand it risks
/// it being read as a second filename.
fn editor_args(basename: &str, path: &str, line: usize) -> Vec<String> {
    match basename {
        "vim" | "nvim" | "vi" | "nano" | "emacs" | "kak" => {
            vec![format!("+{line}"), path.to_string()]
        }
        "hx" => vec![format!("{path}:{line}")],
        "code" | "codium" => vec!["-g".to_string(), format!("{path}:{line}")],
        _ => vec![path.to_string()],
    }
}

/// The full `(program, args)` invocation for `spec` (as returned by
/// `resolve_editor`/`split_command`) opening `path` at 1-based `line`. Any
/// arguments already in `spec` (e.g. `--wait`) are kept ahead of the location
/// arguments. None only for an empty `spec`.
fn build_command(spec: &[String], path: &str, line: usize) -> Option<(String, Vec<String>)> {
    let (program, extra) = spec.split_first()?;
    let basename = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program.as_str());
    let mut args = extra.to_vec();
    args.extend(editor_args(basename, path, line));
    Some((program.clone(), args))
}

/// The selected hunk's `(path, 1-based line, approximate)` to open in the
/// editor. For `zz` the worktree file IS the new content, so the line is
/// exact. For a historical commit or range the worktree may have moved on
/// since — cheap to check by comparing that revision's blob for the path
/// against the file on disk; a mismatch is reported as approximate rather
/// than silently sending the user to an unrelated line.
fn edit_target(app: &App) -> (String, usize, bool) {
    let it = &app.items[app.sel];
    let line = it.new_range[0].max(1);
    if app.uncommitted {
        return (it.path.clone(), line, false);
    }
    let approx = match &app.review_sha {
        Some(sha) => {
            let blob = git(&["show", &format!("{sha}:{}", it.path)]);
            let disk = std::fs::read_to_string(&it.path).unwrap_or_default();
            blob != disk
        }
        None => true, // no revision resolved at all — can't vouch for the line
    };
    (it.path.clone(), line, approx)
}

// --------------------------------------------------------------------- tui loop

/// `ge` / `C-o` — hand the terminal to `$VISUAL`/`$EDITOR` for the selected
/// hunk's file and line, then take it back. Leaves the alternate screen
/// before spawning and re-enters it afterward regardless of outcome — a
/// missing or misbehaving editor must never leave the terminal broken, so
/// failures are reported in the existing popup instead of propagated. The
/// popup also carries the exact/approximate line note from `edit_target`, so
/// the user is never silently sent to a line that may not be right.
fn open_editor(app: &mut App, terminal: &mut ratatui::DefaultTerminal) {
    let (path, line, approx) = edit_target(app);
    let spec = resolve_editor();
    let Some((program, args)) = build_command(&spec, &path, line) else {
        app.popup = Some(Popup {
            title: "edit".to_string(),
            lines: vec![prose("no editor command to run ($VISUAL/$EDITOR)")],
            scroll: 0,
            hscroll: 0,
        });
        return;
    };
    ratatui::restore();
    let outcome = Command::new(&program).args(&args).status();
    *terminal = ratatui::init();
    let _ = terminal.clear(); // the screen underneath may have changed; force a full redraw

    let note = if approx {
        " (approximate — this file has changed since the reviewed revision)"
    } else {
        ""
    };
    let msg = match outcome {
        Ok(status) if status.success() => format!("opened {path}:{line}{note}"),
        Ok(status) => format!("{program} exited with {status}{note}"),
        Err(e) => format!("failed to launch '{program}': {e}"),
    };
    app.popup = Some(Popup {
        title: "edit".to_string(),
        lines: vec![prose(msg)],
        scroll: 0,
        hscroll: 0,
    });
}

/// While loading: just a status line under the pane border, same idiom as
/// the review panes. Nothing else can be drawn yet — no items, no sources.
fn draw_loading(f: &mut Frame, rev: &str, status: &str) {
    let area = f.area();
    let block = Block::bordered().title(format!(" {rev} — loading… "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let p = Paragraph::new(Text::from(vec![Line::from(status.to_string())]));
    f.render_widget(p, inner);
}

enum State {
    Loading(String),
    Ready(Box<App>),
}

/// Take the screen first, then load off the draw loop: a background thread
/// runs `gather`/`highlight_file`/`ordo::run` and reports progress over
/// `LoadMsg`, while this loop keeps drawing and polling input so `q`/`Esc`
/// (or `C-q`/`Esc`) abort a slow load without waiting for the worker. A
/// custom panic hook restores the terminal first even if the worker (or a
/// later draw) panics, so a broken terminal never outlives the crash.
#[allow(clippy::too_many_arguments)]
fn run(
    rev: String,
    keys: Keymap,
    target: Target,
    filter: Filter,
    only_comments: bool,
    review_sha: Option<String>,
    uncommitted: bool,
    theme: Theme,
) -> std::io::Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        prev_hook(info);
    }));

    let mut terminal = ratatui::init();
    // kept around (rather than consumed by the first load) so `:e` can spawn
    // a fresh worker later without re-parsing CLI globs — the launch-time
    // path filter carries across a reload, same as the keymap and theme
    let base_filter = filter.clone();
    let (tx, rx) = mpsc::channel();
    let mut rx = rx;
    let worker_rev = rev.clone();
    thread::spawn(move || load(target, filter, only_comments, worker_rev, tx));

    let mut rev = rev;
    let mut review_sha = review_sha;
    let mut uncommitted = uncommitted;
    let mut state = State::Loading("starting…".to_string());
    let mut keys = Some(keys);
    let mut theme = theme;
    let mut timing: Option<String> = None;
    let mut post_msg: Option<String> = None;
    let result: std::io::Result<()> = 'outer: loop {
        loop {
            match rx.try_recv() {
                Ok(LoadMsg::Progress(s)) => {
                    if let State::Loading(status) = &mut state {
                        *status = s;
                    }
                }
                Ok(LoadMsg::Empty(msg)) => {
                    post_msg = Some(msg);
                    break 'outer Ok(());
                }
                Ok(LoadMsg::Done(r)) => {
                    let LoadResult {
                        items,
                        view,
                        comments_only,
                        sources,
                        highlights,
                        timing: t,
                        reviewed,
                        marks_path,
                        marks,
                        groups,
                        ledger,
                    } = *r;
                    let sel0 = view[0];
                    let scroll = auto_scroll(&items[sel0]);
                    let cursor = cursor_for(&items[sel0], &sources);
                    timing = Some(t);
                    state = State::Ready(Box::new(App {
                        reviewed,
                        items,
                        view,
                        comments_only,
                        show_all: true,
                        path_filter: None,
                        command: None,
                        sel: sel0,
                        scroll,
                        hscroll: 0,
                        why_scroll: 0,
                        why_sel: 0,
                        code_len: 0,
                        why_len: 0,
                        code_height: 10,
                        why_height: 10,
                        code_width: 10,
                        focus: Pane::List,
                        keys: keys.take().expect("keys is set again before every reload back into Loading"),
                        pending: None,
                        sources,
                        highlights,
                        cursor,
                        popup: None,
                        trees: HashMap::new(),
                        prompt: None,
                        search: None,
                        review_sha: review_sha.clone(),
                        uncommitted,
                        history_cache: HashMap::new(),
                        jumps: Vec::new(),
                        rev: rev.clone(),
                        marks_path,
                        marks,
                        theme,
                        show_groups: false,
                        groups,
                        ledger,
                    }));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // the worker dropped its sender without a Done/Empty —
                    // only possible if it panicked; abort rather than spin
                    if matches!(state, State::Loading(_)) {
                        post_msg =
                            Some("ordo-tui: loading failed unexpectedly".to_string());
                        break 'outer Ok(());
                    }
                    break;
                }
            }
        }

        if let Err(e) = terminal.draw(|f| match &mut state {
            State::Loading(status) => draw_loading(f, &rev, status),
            State::Ready(app) => draw(f, app, &rev),
        }) {
            break 'outer Err(e);
        }

        match event::poll(Duration::from_millis(50)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break 'outer Err(e),
        }
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match &mut state {
                State::Loading(_) => {
                    let key = norm(k.code, k.modifiers);
                    if let Resolve::Act(Action::Quit) =
                        keys.as_ref().expect("keys not yet taken while Loading").resolve(None, key)
                    {
                        break 'outer Ok(());
                    }
                }
                State::Ready(app) => {
                    // input mode: every key edits the search prompt instead of
                    // running an action, so chords (C-w C-w, gg) can't leak in
                    // while it's open
                    if app.prompt.is_some() {
                        match (k.code, k.modifiers) {
                            (KeyCode::Enter, _) => accept_search(app),
                            (KeyCode::Esc, _) => cancel_search(app),
                            (KeyCode::Backspace, _) => {
                                if let Some(p) = app.prompt.as_mut() {
                                    p.text.pop();
                                }
                            }
                            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                                if let Some(p) = app.prompt.as_mut() {
                                    p.text.push(c);
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    // same interception scheme as the search prompt above,
                    // for the `:` command bar — every key drives it instead
                    // of resolving through the keymap
                    if app.command.is_some() {
                        match handle_command_key(app, k.code, k.modifiers) {
                            CommandOutcome::Quit => break 'outer Ok(()),
                            CommandOutcome::None => {}
                            CommandOutcome::Reload(target, new_rev) => {
                                // `:e`: everything indexing the *old* review —
                                // selection, cursor, scroll, search, the jump
                                // stack, any open popup — is dropped by simply
                                // not carrying `app` forward into the new
                                // `App` the next `Done` builds (same as a
                                // fresh launch); the reviewed marks come back
                                // from the on-disk cache keyed by the new rev
                                // (see `mark_key`/`load`), not from `app.marks`.
                                // The keymap and theme are carried forward —
                                // display preferences independent of which
                                // hunks are loaded — and the path filter is
                                // re-read from `base_filter` rather than
                                // `app.path_filter`, so a live `:filter`
                                // resets along with everything else.
                                let (carried_keys, carried_theme) = carry_across_reload(app);
                                review_sha = review_commit_sha(&target);
                                uncommitted = matches!(target, Target::Uncommitted | Target::WorktreeRange(_));
                                rev = new_rev.clone();
                                keys = Some(carried_keys);
                                theme = carried_theme;
                                state = State::Loading(format!("switching to {new_rev}…"));
                                let (new_tx, new_rx) = mpsc::channel();
                                rx = new_rx;
                                let filt = base_filter.clone();
                                thread::spawn(move || load(target, filt, only_comments, new_rev, new_tx));
                            }
                        }
                        continue;
                    }
                    let key = norm(k.code, k.modifiers);
                    match app.keys.resolve(app.pending, key) {
                        Resolve::Pending => app.pending = Some(key),
                        Resolve::Miss => app.pending = None,
                        // handled here rather than in `apply`: it must leave the
                        // alternate screen for the editor and come back
                        Resolve::Act(Action::OpenEditor) => {
                            app.pending = None;
                            open_editor(app, &mut terminal);
                        }
                        Resolve::Act(a) => {
                            app.pending = None;
                            if apply(app, a) {
                                break 'outer Ok(());
                            }
                        }
                    }
                }
            },
            Ok(_) => {}
            Err(e) => break 'outer Err(e),
        }
    };
    ratatui::restore();
    result?;
    // The TUI takes the alternate screen, so whichever of these applies
    // stays on the shell's scrollback and is what's left on screen after
    // quitting — the same visible behaviour as printing it before init.
    if let Some(msg) = post_msg {
        eprintln!("{msg}");
    } else if let Some(t) = timing {
        eprintln!("{t}");
    }
    Ok(())
}

/// Run one action against the focused pane; true means quit.
fn apply(app: &mut App, a: Action) -> bool {
    // An open popup consumes input itself: Esc/q dismiss it (without
    // quitting), Next/Prev/PageUp/PageDown scroll its own body rather than
    // the pane underneath, and everything else is a no-op while it's up.
    if app.popup.is_some() {
        match a {
            Action::Quit => app.popup = None,
            Action::Next => popup_scroll(app, 1),
            Action::Prev => popup_scroll(app, -1),
            Action::PageDown => popup_scroll(app, PAGE as isize),
            Action::PageUp => popup_scroll(app, -(PAGE as isize)),
            Action::ScrollLeft => popup_hscroll(app, -1),
            Action::ScrollRight => popup_hscroll(app, 1),
            _ => {}
        }
        return false;
    }
    match a {
        Action::Quit => return true,
        Action::Focus(p) => app.focus = p,
        Action::FocusNext => app.focus = app.focus.next(),
        Action::FocusPrev => app.focus = app.focus.prev(),
        Action::ToggleReviewed => {
            let i = app.sel;
            app.reviewed[i] = !app.reviewed[i];
            persist_mark(app, i);
            let pos = view_pos(&app.view, i);
            if pos + 1 < app.view.len() {
                select(app, app.view[pos + 1]);
            }
        }
        Action::Next => step(app, 1),
        Action::Prev => step(app, -1),
        Action::PageDown => page(app, PAGE as isize),
        Action::PageUp => page(app, -(PAGE as isize)),
        Action::HalfDown => page(app, (PAGE / 2) as isize),
        Action::HalfUp => page(app, -((PAGE / 2) as isize)),
        Action::First => match app.focus {
            Pane::List => select(app, first_visible(app)),
            Pane::Code => app.scroll = 0,
            Pane::Why => app.why_scroll = 0,
        },
        Action::Last => match app.focus {
            Pane::List => select(app, last_visible(app)),
            Pane::Code => app.scroll = last_line(app.code_len),
            Pane::Why => app.why_scroll = last_line(app.why_len),
        },
        // column line-end motion: cursor in the code pane, list/why fall back
        // to `First`/`Last`'s meaning so vscode's Home/End still works there
        Action::LineStart => match app.focus {
            Pane::List => select(app, first_visible(app)),
            Pane::Code => cursor_move(app, line_start),
            Pane::Why => app.why_scroll = 0,
        },
        Action::LineEnd => match app.focus {
            Pane::List => select(app, last_visible(app)),
            Pane::Code => cursor_move(app, line_end),
            Pane::Why => app.why_scroll = last_line(app.why_len),
        },
        Action::CursorLeft if app.focus == Pane::Code => {
            cursor_move(app, |c, lines| move_col(c, lines, -1))
        }
        Action::CursorRight if app.focus == Pane::Code => {
            cursor_move(app, |c, lines| move_col(c, lines, 1))
        }
        Action::WordNext if app.focus == Pane::Code => cursor_move(app, word_next),
        Action::WordPrev if app.focus == Pane::Code => cursor_move(app, word_prev),
        Action::WordEnd if app.focus == Pane::Code => cursor_move(app, word_end),
        Action::ParaPrev if app.focus == Pane::Code => cursor_move(app, para_prev),
        Action::ParaNext if app.focus == Pane::Code => cursor_move(app, para_next),
        Action::CursorLeft
        | Action::CursorRight
        | Action::WordNext
        | Action::WordPrev
        | Action::WordEnd
        | Action::ParaPrev
        | Action::ParaNext => {} // only meaningful with the code pane focused
        // `K`/`F12` dispatches on the focused pane rather than adding a
        // second key: the code pane's symbol hover and the why pane's dep
        // preview are the same "show me more about what's under the cursor"
        // gesture, just aimed at a different cursor.
        Action::Hover => match app.focus {
            Pane::Code => hover(app),
            Pane::Why => preview_edge(app),
            Pane::List => {}
        },
        Action::SearchOpen if app.focus == Pane::Code => {
            app.prompt = Some(Prompt {
                text: String::new(),
                anchor: app.cursor,
            });
        }
        Action::SymbolNext if app.focus == Pane::Code => symbol_search(app, true),
        Action::SymbolPrev if app.focus == Pane::Code => symbol_search(app, false),
        Action::SearchNext if app.focus == Pane::Code => cycle_search(app, 1),
        Action::SearchPrev if app.focus == Pane::Code => cycle_search(app, -1),
        Action::SearchOpen
        | Action::SymbolNext
        | Action::SymbolPrev
        | Action::SearchNext
        | Action::SearchPrev => {} // only meaningful with the code pane focused
        Action::ScrollLeft if app.focus == Pane::Code => {
            app.hscroll = app.hscroll.saturating_sub(1);
        }
        Action::ScrollRight if app.focus == Pane::Code => {
            app.hscroll = app.hscroll.saturating_add(1);
        }
        Action::ScrollLeft | Action::ScrollRight => {} // only meaningful with the code pane focused
        Action::Help => {
            app.popup = Some(Popup {
                title: format!("keybindings — {}", app.keys.name),
                lines: build_help(&app.keys).into_iter().map(prose).collect(),
                scroll: 0,
                hscroll: 0,
            });
        }
        Action::CommandOpen => open_command_bar(app, String::new()),
        Action::CommandGoto => open_command_bar(app, "goto ".to_string()),
        // intercepted in `run` before it reaches here (needs the terminal)
        Action::OpenEditor => {}
        Action::JumpToEdge if app.focus == Pane::Why => jump_to_edge(app),
        Action::JumpToEdge => {} // only meaningful with the why pane focused
        Action::JumpBack => jump_back(app),
    }
    false
}

/// Writes a toggle to the mark cache immediately — not only on quit — so a
/// panic or a killed terminal never loses the session's marks. A no-op when
/// the item's content isn't loaded or `marks_path` never resolved (see
/// `mark_key`/`marks_file_path`); the checkmark still holds for this session
/// either way, only the write is skipped.
fn persist_mark(app: &mut App, i: usize) {
    let Some(key) = mark_key(&app.rev, &app.items[i], &app.sources) else {
        return;
    };
    if app.reviewed[i] {
        app.marks.insert(key, now_unix());
    } else {
        app.marks.remove(&key);
    }
    let Some(path) = app.marks_path.clone() else { return };
    prune_marks(&mut app.marks, now_unix());
    save_marks(&path, &app.marks);
}

/// Popup body line with no styling — most popups are prose.
fn prose(s: impl Into<String>) -> Line<'static> {
    Line::from(s.into())
}

/// Widest popup body line, in characters — what `hscroll` clamps against.
fn popup_width(lines: &[Line<'static>]) -> usize {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.chars().count()).sum::<usize>())
        .max()
        .unwrap_or(0)
}

// scroll the open popup's body sideways by `by` columns, clamped to its content
fn popup_hscroll(app: &mut App, by: isize) {
    if let Some(p) = app.popup.as_mut() {
        let max = last_line(popup_width(&p.lines)) as isize;
        p.hscroll = (p.hscroll as isize + by).clamp(0, max.max(0)) as u16;
    }
}

// scroll the open popup's body by `by` lines, clamped to its content
fn popup_scroll(app: &mut App, by: isize) {
    if let Some(p) = app.popup.as_mut() {
        p.scroll = (p.scroll as isize + by).clamp(0, last_line(p.lines.len()) as isize) as u16;
    }
}

/// Move the code cursor with `f`, clamp it to the file, and scroll to follow.
/// No-op when the selected item's file content isn't loaded.
fn cursor_move(app: &mut App, f: impl FnOnce(Cursor, &[String]) -> Cursor) {
    let Some((_, lines)) = app.sources.get(&app.items[app.sel].path) else {
        return;
    };
    if lines.is_empty() {
        return;
    }
    app.cursor = clamp_cursor(f(app.cursor, lines), lines);
    app.scroll = follow_scroll(app.cursor.line, app.scroll, app.code_height);
    app.hscroll = follow_hscroll(app.cursor.col, app.hscroll, app.code_width);
}

fn last_line(len: usize) -> u16 {
    len.saturating_sub(1).min(u16::MAX as usize) as u16
}

/// One-line motion: moves the selection in the list, the cursor in the code
/// pane (scroll follows), scrolls the why pane.
fn step(app: &mut App, by: isize) {
    match app.focus {
        Pane::List => {
            let pos = view_pos(&app.view, app.sel);
            let to = (pos as isize + by).clamp(0, app.view.len() as isize - 1) as usize;
            select(app, app.view[to]);
        }
        Pane::Code => cursor_move(app, |c, lines| move_line(c, lines, by)),
        Pane::Why => why_cursor_move(app, by),
    }
}

/// The list-pane position of `sel` within `view` — 0 when it isn't found
/// (shouldn't happen: `set_filters`/`select` keep `sel` in `view`, but a
/// missing position degrading to the top is safer than panicking).
fn view_pos(view: &[usize], sel: usize) -> usize {
    view.iter().position(|&i| i == sel).unwrap_or(0)
}

/// The first/last currently visible item — `view` is never empty once
/// loading finishes (`set_filters` rejects any change that would empty it).
fn first_visible(app: &App) -> usize {
    app.view[0]
}
fn last_visible(app: &App) -> usize {
    *app.view.last().expect("view is never empty")
}

/// Move the why pane's line cursor by `by`, clamp to its content, and scroll
/// to follow — the why-pane counterpart of `cursor_move`.
fn why_cursor_move(app: &mut App, by: isize) {
    app.why_sel = scrolled(app.why_sel as u16, by, app.why_len) as usize;
    app.why_scroll = follow_scroll(app.why_sel, app.why_scroll, app.why_height);
}

/// Paging drives the code pane even when the list has focus — list rows are one
/// line each, so paging them is useless while the code is what needs scrolling.
fn page(app: &mut App, by: isize) {
    match app.focus {
        Pane::Why => app.why_scroll = scrolled(app.why_scroll, by, app.why_len),
        _ => app.scroll = scrolled(app.scroll, by, app.code_len),
    }
}

fn scrolled(cur: u16, by: isize, len: usize) -> u16 {
    (cur as isize + by).clamp(0, last_line(len) as isize) as u16
}

fn select(app: &mut App, to: usize) {
    app.sel = to;
    app.scroll = auto_scroll(&app.items[to]);
    app.hscroll = 0;
    app.why_scroll = 0;
    app.why_sel = 0;
    app.cursor = cursor_for(&app.items[to], &app.sources);
    app.popup = None; // a new hunk invalidates whatever the popup was showing
    // match positions are per-file line/col — a different hunk (possibly a
    // different file entirely) invalidates them, so re-anchor by dropping the
    // search rather than trying to remap it
    app.search = None;
}

// --------------------------------------------------------------- why pane

/// One line of the why pane's content, independent of rendering — shared by
/// `draw` and the edge actions (`preview_edge`/`jump_to_edge`) below so both
/// agree on what line `why_sel` is pointing at.
enum WhyKind {
    Text,
    /// a `dep` line; `Some(idx)` is its target hunk's index into `app.items`,
    /// `None` when the referenced hunk isn't part of this review
    Edge(Option<usize>),
}

struct WhyRow {
    text: String,
    style: Style,
    kind: WhyKind,
}

// Actionable dep lines get a distinct look: bold+underlined when the target
// resolves to a hunk in this review, dimmed when it doesn't — honest at a
// glance about there being nothing to jump to.
fn edge_style(target: Option<usize>) -> Style {
    if target.is_some() {
        Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// The why pane's content, in render order — reason, details, notes, dep
/// (edge) lines, then advisories. A pure function of `Item` (plus `view`, so
/// a dep line whose target is currently filtered out renders — and resolves
/// — the same as one that was never part of the review) so it doubles as the
/// source of truth for what `why_sel` is currently sitting on.
fn why_rows(it: &Item, view: &[usize]) -> Vec<WhyRow> {
    let mut rows = vec![];
    // The engine's terminal fallback rationale: it found nothing to say about
    // the hunk, so a "reason: change" line says nothing either — leave it out.
    if it.rationale != "change" {
        rows.push(WhyRow {
            text: format!("reason: {}", it.rationale),
            style: Style::default().fg(Color::Cyan),
            kind: WhyKind::Text,
        });
    }
    for d in &it.details {
        rows.push(WhyRow {
            text: format!("- {d}"),
            style: Style::default().fg(Color::Blue),
            kind: WhyKind::Text,
        });
    }
    if !it.notes.is_empty() {
        rows.push(WhyRow {
            text: format!("notes: {}", it.notes.join("; ")),
            style: Style::default().fg(Color::Yellow),
            kind: WhyKind::Text,
        });
    }
    for e in &it.edges {
        let target = e.target.filter(|t| view.contains(t));
        rows.push(WhyRow {
            text: format!("dep {}", e.label),
            style: edge_style(target),
            kind: WhyKind::Edge(target),
        });
    }
    for (construct, message, verdict) in &it.advisories {
        let (head, color) = if *verdict {
            (format!("⚠ {construct}"), Color::Red)
        } else {
            (construct.clone(), Color::Magenta)
        };
        rows.push(WhyRow {
            text: head,
            style: Style::default().fg(color).add_modifier(Modifier::BOLD),
            kind: WhyKind::Text,
        });
        for ml in message.lines() {
            rows.push(WhyRow {
                text: format!("  {ml}"),
                style: Style::default(),
                kind: WhyKind::Text,
            });
        }
    }
    rows
}

// the dep-line target at `app.why_sel`, if the cursor is on one at all
fn edge_at_cursor(app: &App) -> Option<Option<usize>> {
    let rows = why_rows(&app.items[app.sel], &app.view);
    match rows.get(app.why_sel)?.kind {
        WhyKind::Edge(target) => Some(target),
        WhyKind::Text => None,
    }
}

/// `K`/`F12` while the why pane is focused: preview the dep line's target
/// hunk — its location, rationale, and a code excerpt — without leaving the
/// current hunk. No-op when `why_sel` isn't on a dep line. When the target
/// isn't part of this review, says so plainly rather than showing nothing.
/// A syntax-highlighted excerpt of `lines[n0..=n1]` (1-based, inclusive) with a
/// line-number gutter, for the dep preview. Falls back to unstyled text for a
/// file with no grammar, the same way the code pane does.
fn excerpt(
    lines: &[String],
    hl: Option<&Vec<LineSpans>>,
    n0: usize,
    n1: usize,
) -> Vec<Line<'static>> {
    let num = Style::default().fg(Color::DarkGray);
    (n0..=n1)
        .filter_map(|ln| {
            let text = lines.get(ln - 1)?;
            let mut spans = vec![Span::styled(format!("{ln:>5} "), num)];
            match hl.and_then(|h| h.get(ln - 1)) {
                Some(segs) if !segs.is_empty() => spans.extend(
                    segs.iter()
                        .map(|(t, c)| Span::styled(t.clone(), Style::default().fg(*c))),
                ),
                _ => spans.push(Span::raw(text.clone())),
            }
            Some(Line::from(spans))
        })
        .collect()
}

fn preview_edge(app: &mut App) {
    let Some(target) = edge_at_cursor(app) else { return };
    let Some(idx) = target else {
        app.popup = Some(Popup {
            title: "dep".to_string(),
            lines: vec![
                prose("the referenced change isn't part of this review"),
                prose("(excluded by a glob, --only-comments, or a file not sent to ordo)"),
            ],
            scroll: 0,
            hscroll: 0,
        });
        return;
    };
    let t = &app.items[idx];
    let mut lines = vec![prose(format!("{}:L{}", t.path, t.new_range[0]))];
    if t.rationale != "change" {
        lines.push(prose(""));
        lines.push(Line::from(Span::styled(
            format!("reason: {}", t.rationale),
            Style::default().fg(Color::Cyan),
        )));
    }
    if let Some((_, nl)) = app.sources.get(&t.path) {
        let [n0, n1] = t.new_range;
        if n0 >= 1 && n0 <= nl.len() {
            lines.push(prose(""));
            lines.extend(excerpt(nl, app.highlights.get(&t.path), n0, n1.min(nl.len())));
        }
    }
    app.popup = Some(Popup { title: "dep".to_string(), lines, scroll: 0, hscroll: 0 });
}

/// `Enter`/`gd` (vim), `C-Enter` (vscode) while the why pane is focused:
/// select the dep line's target hunk and focus the code pane, pushing the
/// current position first so `JumpBack` can return. No-op — never a guess —
/// when `why_sel` isn't on a dep line, or its target isn't part of this
/// review; `preview_edge` (`K`/`F12`) is what explains why in that case.
fn jump_to_edge(app: &mut App) {
    let Some(Some(idx)) = edge_at_cursor(app) else { return };
    stack_push(&mut app.jumps, (app.sel, app.cursor));
    select(app, idx);
    app.focus = Pane::Code;
}

/// `C-o` (vim) / `Alt-Left` (vscode): pop the position stack and return
/// there — skipping (not just the out-of-range entries `stack_pop_valid`
/// already drops, but also) any entry a live filter currently hides, the
/// same "not part of this review" treatment `jump_to_edge` gives it. A
/// no-op once the stack is exhausted.
fn jump_back(app: &mut App) {
    let (idx, cursor) = loop {
        match stack_pop_valid(&mut app.jumps, app.items.len()) {
            Some((idx, cursor)) if app.view.contains(&idx) => break (idx, cursor),
            Some(_) => continue, // hidden by a live filter — try the next one
            None => return,
        }
    };
    select(app, idx);
    if let Some((_, nl)) = app.sources.get(&app.items[idx].path) {
        if !nl.is_empty() {
            app.cursor = clamp_cursor(cursor, nl);
        }
    }
    app.focus = Pane::Code;
}

/// The focused pane gets a cyan border.
fn pane_block(title: String, focused: bool) -> Block<'static> {
    let b = Block::bordered().title(title);
    if focused {
        b.border_style(Style::default().fg(Color::Cyan))
    } else {
        b
    }
}

fn draw(f: &mut Frame, app: &mut App, rev: &str) {
    let cols = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(f.area());

    // left — reading order
    let symbol = "▶ ";
    let text_w = (cols[0].width as usize).saturating_sub(2 + symbol.chars().count());
    let display = display_rows(&app.view, &app.items, &app.groups, app.show_groups);
    let rows: Vec<ListItem> = display
        .iter()
        .map(|row| match row {
            DisplayRow::Header(reason) => {
                let spans = vec![Span::styled(
                    format!("· {reason}"),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )];
                ListItem::new(Line::from(slice_range(spans, 0, text_w)))
            }
            DisplayRow::Item(i) => {
                let i = *i;
                let it = &app.items[i];
                let style = if it.noise {
                    Style::default().fg(Color::DarkGray)
                } else if app.reviewed[i] {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                // head only — the full rationale lives in the "why" pane.
                // Path, line number and category are separate spans so each
                // reads at a glance; `style` (noise/reviewed) still tints
                // the row as a whole.
                let dim = |c: Color| {
                    if style.fg.is_some() { style } else { style.fg(c) }
                };
                let spans = vec![
                    Span::styled(it.mark.clone(), dim(Color::Yellow)),
                    Span::styled(it.path.clone(), style),
                    Span::styled(format!(":L{}", it.new_range[0]), dim(Color::Blue)),
                    Span::styled(format!(" [{}]", it.cat), dim(Color::Magenta)),
                ];
                ListItem::new(Line::from(slice_range(spans, 0, text_w)))
            }
        })
        .collect();
    let done = app.view.iter().filter(|&&i| app.reviewed[i]).count();
    let mut state = ListState::default();
    let sel_row = display_row_of(&app.view, &app.items, app.show_groups, view_pos(&app.view, app.sel));
    state.select(Some(sel_row));
    let filtered = app.view.len() < app.items.len();
    let list = List::new(rows)
        .block(pane_block(
            format!(
                " {rev} — {done}/{} reviewed{} · {} ",
                app.view.len(),
                if filtered { " (filtered)" } else { "" },
                app.keys.name
            ),
            app.focus == Pane::List,
        ))
        .highlight_style(Style::default().bg(app.theme.select_bg))
        .highlight_symbol(symbol);
    f.render_stateful_widget(list, cols[0], &mut state);

    // right — code (top) + why (bottom)
    let rhs =
        Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).split(cols[1]);
    let it = &app.items[app.sel];

    let code_w = (rhs[0].width as usize).saturating_sub(2);
    app.code_height = rhs[0].height.saturating_sub(2);
    app.code_width = code_w.saturating_sub(GUTTER_W).min(u16::MAX as usize) as u16;
    // clamp to the selected file's longest line so hscroll can't run away
    // past any content it could ever bring into view
    let max_col = app
        .sources
        .get(&it.path)
        .map(|(_, nl)| nl.iter().map(|l| l.chars().count()).max().unwrap_or(0))
        .unwrap_or(0);
    app.hscroll = app.hscroll.min(max_col.min(u16::MAX as usize) as u16);
    let (search_matches, cur_match): (&[(usize, usize, usize)], Option<usize>) = match &app.search {
        Some(s) => (&s.matches, Some(s.index)),
        None => (&[], None),
    };
    let (code, right_clip) = code_view(
        it,
        &app.sources,
        &app.highlights,
        code_w,
        app.hscroll as usize,
        Some(app.cursor),
        search_matches,
        cur_match,
        &app.theme,
    );
    // `‹`/`›` mark content clipped off the left/right of the horizontal
    // window — truncation must never be silent, so this is always shown
    // rather than only surfaced by scrolling into it.
    let clip = match (app.hscroll > 0, right_clip) {
        (true, true) => " ‹›",
        (true, false) => " ‹",
        (false, true) => " ›",
        (false, false) => "",
    };
    // The `/` prompt and the active-search status both reuse the code pane's
    // border title rather than a separate widget — one line is enough for
    // either, and it keeps the layout unchanged while typing or searching.
    let code_title = if let Some(p) = &app.prompt {
        format!(" search: {}▏ ", p.text)
    } else if let Some(s) = &app.search {
        let glyph = match s.kind {
            SearchKind::Text => '/',
            SearchKind::Symbol => '*',
        };
        let status = if s.matches.is_empty() {
            format!("no matches for {glyph}{}", s.pattern)
        } else {
            format!("[{}/{}] {glyph}{}", s.index + 1, s.matches.len(), s.pattern)
        };
        format!(" {}{clip}  {status}  ({}) ", it.path, app.keys.hint)
    } else {
        format!(" {}{clip}  ({}) ", it.path, app.keys.hint)
    };

    let why_content = why_rows(it, &app.view);
    // `why` wraps, so this counts logical lines — enough to keep the scroll in range
    app.code_len = code.len();
    app.why_len = why_content.len();
    app.why_height = rhs[1].height.saturating_sub(2);
    app.scroll = app.scroll.min(last_line(app.code_len));
    app.why_scroll = app.why_scroll.min(last_line(app.why_len));
    app.why_sel = app.why_sel.min(last_line(app.why_len) as usize);
    // the current line reverses, same idiom as the list's selection and the
    // code pane's cursor cell
    let why: Vec<Line> = why_content
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if app.focus == Pane::Why && i == app.why_sel {
                r.style.add_modifier(Modifier::REVERSED)
            } else {
                r.style
            };
            Line::from(Span::styled(r.text.clone(), style))
        })
        .collect();

    let code_view = Paragraph::new(Text::from(code))
        .block(pane_block(code_title, app.focus == Pane::Code))
        .scroll((app.scroll, 0));
    f.render_widget(code_view, rhs[0]);

    let info = Paragraph::new(Text::from(why))
        .block(pane_block(" why ".to_string(), app.focus == Pane::Why))
        .scroll((app.why_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(info, rhs[1]);

    if let Some(popup) = app.popup.as_mut() {
        popup.scroll = popup.scroll.min(last_line(popup.lines.len()));
        popup.hscroll = popup.hscroll.min(last_line(popup_width(&popup.lines)));
    }
    if let Some(popup) = &app.popup {
        let rect = popup_rect(rhs[0], popup.lines.len());
        f.render_widget(Clear, rect);
        let text: Vec<Line> = popup.lines.clone();
        let clipped = popup_width(&popup.lines) > rect.width.saturating_sub(2) as usize
            || popup.hscroll > 0;
        let block = Block::bordered()
            .title(format!(
                " {}{} ",
                popup.title,
                if clipped { " ‹›" } else { "" }
            ))
            .border_style(Style::default().fg(Color::Yellow));
        let p = Paragraph::new(Text::from(text))
            .block(block)
            .scroll((popup.scroll, popup.hscroll));
        f.render_widget(p, rect);
    }

    if let Some(bar) = &app.command {
        let area = f.area();
        let bar_rect = command_bar_rect(area);
        f.render_widget(Clear, bar_rect);
        let line = format!(":{}▏", bar.text);
        let p = Paragraph::new(Text::from(vec![Line::from(line)])).block(
            Block::bordered()
                .title(" command ")
                .border_style(Style::default().fg(Color::Cyan)),
        );
        f.render_widget(p, bar_rect);

        if !bar.candidates.is_empty() {
            let menu_rect = command_menu_rect(bar_rect, area, bar.candidates.len());
            f.render_widget(Clear, menu_rect);
            let entries: Vec<ListItem> = bar
                .candidates
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let style = if bar.selected == Some(i) {
                        Style::default().bg(app.theme.select_bg)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(Span::styled(c.clone(), style)))
                })
                .collect();
            f.render_widget(List::new(entries).block(Block::bordered()), menu_rect);
        }
    }
}

// centered floating box over `area`, sized to the popup's content
fn popup_rect(area: Rect, n_lines: usize) -> Rect {
    let w = (area.width.saturating_sub(4)).clamp(20, 90);
    let h = ((n_lines as u16) + 2).clamp(3, area.height.saturating_sub(2).max(3));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

// the command bar itself: one content row, centered, ~60% of the terminal's
// width, a couple of rows down from the top
fn command_bar_rect(area: Rect) -> Rect {
    let w = ((area.width as u32 * 3 / 5) as u16).clamp(20, area.width.saturating_sub(4).max(20));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + 1,
        width: w,
        height: 3,
    }
}

// the completion menu: left-aligned to `bar`, directly below it, sized to
// its candidate rows but never past the bottom of the terminal
fn command_menu_rect(bar: Rect, area: Rect, n_candidates: usize) -> Rect {
    let below = area.y + area.height;
    let avail = below.saturating_sub(bar.y + bar.height).max(3);
    let h = ((n_candidates as u16) + 2).clamp(3, avail);
    Rect {
        x: bar.x,
        y: bar.y + bar.height,
        width: bar.width,
        height: h,
    }
}

// -------------------------------------------------------------- command mode

/// One `:` command — name, a short argument hint for `:help` (empty when it
/// takes none), and the help line itself.
struct Cmd {
    name: &'static str,
    args: &'static str,
    help: &'static str,
}

/// The command table — the single source of truth for command names,
/// `:help`'s listing, and command-name completion. Adding a command here is
/// the only step that makes it discoverable; `execute_command`'s `match`
/// still has to know what to *do* with it, but a name present here with no
/// matching arm there falls into that match's `unknown command` case rather
/// than silently doing nothing.
const COMMANDS: &[Cmd] = &[
    Cmd { name: "only-comments", args: "", help: "toggle showing only comment/docstring hunks" },
    Cmd { name: "all", args: "", help: "toggle showing generated/formatting-noise hunks" },
    Cmd {
        name: "filter",
        args: "<glob>",
        help: "narrow the review to paths matching <glob>; no argument clears it",
    },
    Cmd { name: "keys", args: "<preset>", help: "swap the keymap live (vim, vscode)" },
    Cmd {
        name: "strategy",
        args: "<name>",
        help: "re-order the review (comprehension, defs-first, file)",
    },
    Cmd { name: "group", args: "", help: "toggle group-reason headers in the reading-order list" },
    Cmd { name: "goto", args: "<path>", help: "select the first hunk of <path>, focus the code pane" },
    Cmd { name: "e", args: "<rev>", help: "review a different revision, without restarting" },
    Cmd {
        name: "audit",
        args: "",
        help: "account for every hunk and file not on screen, and why",
    },
    Cmd { name: "q", args: "", help: "quit" },
    Cmd { name: "help", args: "", help: "list these commands" },
];

/// `:audit`'s body — every hunk and file that isn't on screen, charged to the
/// thing that removed it. The point is the last line: if the reasons don't add
/// up to what's missing, it says so instead of implying the review is complete.
fn build_audit(
    items: &[Item],
    view_len: usize,
    hidden: &Hidden,
    ledger: &Ledger,
    path_filter: Option<&str>,
) -> Vec<String> {
    let mut out = vec![
        format!("{view_len} of {} hunks shown", items.len()),
        String::new(),
        "hidden in the view".to_string(),
    ];
    let row = |n: usize, what: &str| format!("  {n:>4}  {what}");
    out.push(row(hidden.noise, "generated/formatting noise (:all shows them)"));
    out.push(row(hidden.comment, "not a comment change (:only-comments)"));
    out.push(row(
        hidden.glob,
        &match path_filter {
            Some(g) => format!("outside the path filter '{g}'"),
            None => "outside the path filter".to_string(),
        },
    ));

    out.push(String::new());
    out.push("dropped by the engine before ordering".to_string());
    out.push(row(ledger.hunks_import, "pure import hunk"));
    out.push(row(ledger.hunks_non_comment, "not a comment change (--only-comments)"));

    out.push(String::new());
    out.push(format!("files: {} changed, of which", ledger.files_seen));
    out.push(row(ledger.files_generated, "generated or lock file (--all keeps them)"));
    out.push(row(ledger.files_declared, "declared generated by .gitattributes"));
    out.push(row(ledger.files_globbed, "never fetched: excluded by a launch-time glob"));
    out.push(row(ledger.files_unreadable, "listed as changed but unreadable"));

    out.push(String::new());
    out.push(if hidden.unaccounted == 0 {
        "every hidden hunk is accounted for".to_string()
    } else {
        format!(
            "{} hidden hunk{} unaccounted for — this is a bug, please report it",
            hidden.unaccounted,
            plural(hidden.unaccounted)
        )
    });
    out
}

fn command_names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

/// `:help`'s body, generated straight from `COMMANDS` — same discipline as
/// `build_help` for keybindings, so the listing can't name a command that
/// doesn't exist or omit one that does.
fn build_command_help() -> Vec<String> {
    COMMANDS
        .iter()
        .map(|c| {
            let head = if c.args.is_empty() {
                format!(":{}", c.name)
            } else {
                format!(":{} {}", c.name, c.args)
            };
            format!("  {head:<20} {}", c.help)
        })
        .collect()
}

/// Splits a typed command-bar line (without its leading `:`) into the command
/// name and its argument, trimming both — `"filter   src/*  "` ->
/// `("filter", "src/*")`. A bare name or an empty line gets an empty argument.
fn parse_command_line(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim()),
        None => (line, ""),
    }
}

/// The completion matcher: prefix match first (case-insensitive), substring
/// as a fallback when nothing prefixes. Empty input lists every candidate
/// unfiltered; no match at all — of either kind — yields an empty menu
/// rather than a stale or wrong one.
fn complete(input: &str, candidates: &[String]) -> Vec<String> {
    if input.is_empty() {
        return candidates.to_vec();
    }
    let needle = input.to_lowercase();
    let prefix: Vec<String> = candidates
        .iter()
        .filter(|c| c.to_lowercase().starts_with(&needle))
        .cloned()
        .collect();
    if !prefix.is_empty() {
        return prefix;
    }
    candidates.iter().filter(|c| c.to_lowercase().contains(&needle)).cloned().collect()
}

/// The argument candidates for one command name — empty for a command that
/// takes none, which is how `command_completions` knows not to show a menu.
/// `goto_paths`/`filter_dirs`/`rev_candidates` are handed in rather than
/// computed here so this stays a pure function of plain data (see
/// `recompute_candidates` for where they come from).
fn arg_candidates(
    cmd: &str,
    goto_paths: &[String],
    filter_dirs: &[String],
    rev_candidates: &[String],
) -> Vec<String> {
    match cmd {
        "keys" => vec!["vim".to_string(), "vscode".to_string()],
        "strategy" => vec!["comprehension".to_string(), "defs-first".to_string(), "file".to_string()],
        "goto" => goto_paths.to_vec(),
        "filter" => filter_dirs.to_vec(),
        "e" => rev_candidates.to_vec(),
        _ => vec![],
    }
}

/// The completion menu for the command bar's current text: command names
/// while the first word is still being typed, that command's own argument
/// candidates once a space follows it.
fn command_completions(
    text: &str,
    goto_paths: &[String],
    filter_dirs: &[String],
    rev_candidates: &[String],
) -> Vec<String> {
    match text.find(char::is_whitespace) {
        None => complete(text, &command_names()),
        Some(pos) => {
            let name = &text[..pos];
            let arg = text[pos..].trim_start();
            let pool = arg_candidates(name, goto_paths, filter_dirs, rev_candidates);
            if pool.is_empty() {
                vec![]
            } else {
                complete(arg, &pool)
            }
        }
    }
}

/// Splice an accepted completion into the command line, replacing whichever
/// word is being completed and leaving a trailing space so the next word (an
/// argument, once the command name itself was just completed) can start
/// right away.
fn apply_completion(text: &str, candidate: &str) -> String {
    match text.find(char::is_whitespace) {
        None => format!("{candidate} "),
        Some(pos) => format!("{}{candidate} ", &text[..=pos]),
    }
}

fn distinct_sorted<'a>(vals: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<String> = vals.map(str::to_string).collect();
    v.sort();
    v.dedup();
    v
}

fn dir_prefix(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

fn open_command_bar(app: &mut App, text: String) {
    app.command = Some(CommandBar { text, candidates: vec![], selected: None });
    recompute_candidates(app);
}

/// `:e`'s own argument pool: `zz` and `HEAD` plus every branch and tag, via
/// one `git for-each-ref` — shelled out here, lazily, only when
/// `recompute_candidates` is actually completing an `:e ` argument, never at
/// startup.
fn rev_completions() -> Vec<String> {
    let mut v = vec!["zz".to_string(), "HEAD".to_string()];
    let out = git(&["for-each-ref", "--format=%(refname:short)"]);
    v.extend(out.lines().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
    v
}

/// Recomputes the open command bar's completion menu for its current text —
/// called after every edit (typing, backspace, accepting a completion).
fn recompute_candidates(app: &mut App) {
    let text = match &app.command {
        Some(bar) => bar.text.clone(),
        None => return,
    };
    // `:goto` only offers currently visible paths (jumping to a hidden one
    // would strand `sel` outside `view`); `:filter` offers every loaded
    // path's directory, since narrowing is the point of typing one.
    let goto_paths = distinct_sorted(app.view.iter().map(|&i| app.items[i].path.as_str()));
    let filter_dirs = distinct_sorted(app.items.iter().filter_map(|it| dir_prefix(&it.path)));
    let rev_candidates = match text.find(char::is_whitespace) {
        Some(pos) if &text[..pos] == "e" => rev_completions(),
        _ => vec![],
    };
    let candidates = command_completions(&text, &goto_paths, &filter_dirs, &rev_candidates);
    if let Some(bar) = app.command.as_mut() {
        bar.candidates = candidates;
        bar.selected = None;
    }
}

fn cycle_candidate(app: &mut App, dir: isize) {
    let Some(bar) = app.command.as_mut() else { return };
    if bar.candidates.is_empty() {
        return;
    }
    let len = bar.candidates.len();
    bar.selected = Some(match bar.selected {
        None if dir >= 0 => 0,
        None => len - 1,
        Some(i) => cycle_index(i, len, dir),
    });
}

/// `:e <rev>`'s carry-forward: the keymap and theme, both display
/// preferences independent of which revision is loaded. Everything else a
/// fresh `App` would otherwise not have is deliberately left behind (see
/// `run`'s `CommandOutcome::Reload` handling, which this feeds): selection,
/// cursor, scroll, search, the jump stack, any open popup, and the
/// comment/noise/path filters all reset to the same defaults a plain launch
/// starts with, because the new revision's `App` is built by the exact same
/// `Ok(LoadMsg::Done)` path a first load takes — there's no second
/// construction site to keep in sync. The reviewed marks aren't "reset" so
/// much as recomputed: they're keyed by `(rev, ...)` (see `mark_key`), so the
/// new rev's marks come back from the on-disk cache on their own, without
/// referencing the old session's `app.marks` at all.
fn carry_across_reload(app: &App) -> (Keymap, Theme) {
    (app.keys.clone(), app.theme)
}

/// What running a command line does to the outer session — most just mutate
/// `app` in place and report `None`; `:q` quits, and `:e` needs to leave
/// `State::Ready` entirely and go back through the progressive loader, which
/// `execute_command`/`accept_command`/`handle_command_key` can't do on their
/// own (they only ever see `&mut App`) — so it's handed back to `run` as data.
enum CommandOutcome {
    None,
    Quit,
    /// `:e <rev>` resolved: the resolved target and the rev string as typed
    Reload(Target, String),
}

/// `Enter` on the command bar: with a candidate highlighted, splice it into
/// the line (doesn't run anything yet — a second `Enter` does); with nothing
/// highlighted, run the line and close the bar.
fn accept_command(app: &mut App) -> CommandOutcome {
    let Some(bar) = app.command.as_ref() else { return CommandOutcome::None };
    if let Some(i) = bar.selected {
        let candidate = bar.candidates[i].clone();
        let new_text = apply_completion(&bar.text, &candidate);
        if let Some(bar) = app.command.as_mut() {
            bar.text = new_text;
        }
        recompute_candidates(app);
        return CommandOutcome::None;
    }
    let line = bar.text.clone();
    app.command = None;
    match execute_command(app, &line) {
        Ok(outcome) => outcome,
        Err(msg) => {
            app.popup = Some(Popup { title: "command".to_string(), lines: vec![prose(msg)], scroll: 0, hscroll: 0 });
            CommandOutcome::None
        }
    }
}

/// Command-bar key handling while `app.command` is open — every key edits or
/// drives the bar instead of resolving through the keymap, mirroring the `/`
/// prompt's interception.
fn handle_command_key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> CommandOutcome {
    if app.command.is_none() {
        return CommandOutcome::None;
    }
    match (code, mods) {
        (KeyCode::Esc, _) => app.command = None,
        (KeyCode::Backspace, _) => {
            if let Some(bar) = app.command.as_mut() {
                bar.text.pop();
            }
            recompute_candidates(app);
        }
        (KeyCode::Tab, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
            cycle_candidate(app, 1);
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => cycle_candidate(app, -1),
        (KeyCode::Enter, _) => return accept_command(app),
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            if let Some(bar) = app.command.as_mut() {
                bar.text.push(c);
            }
            recompute_candidates(app);
        }
        _ => {}
    }
    CommandOutcome::None
}

fn parse_strategy(s: &str) -> Option<Strategy> {
    match s {
        "comprehension" => Some(Strategy::Comprehension),
        "defs-first" => Some(Strategy::DefsFirst),
        "file" => Some(Strategy::File),
        _ => None,
    }
}

/// Rebuild the `Change` list `ordo::run` needs from `app.sources` — the full
/// old/new content per path, already in memory from the initial load, so
/// `:strategy` never re-reads git or re-fetches anything. Sorted by path for
/// determinism: `Sources` is a `HashMap`, so the launch's own file order
/// (which `Strategy::File` uses as its tiebreak) isn't preserved either way;
/// alphabetical is a stable, predictable substitute.
fn changes_from_sources(sources: &Sources) -> Vec<Change> {
    let mut paths: Vec<&String> = sources.keys().collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let (old, new) = &sources[path];
            Change { path: path.clone(), old: Some(old.join("\n")), new: Some(new.join("\n")), diff: None }
        })
        .collect()
}

/// `:strategy <name>` — re-runs `ordo::run` synchronously on the main thread
/// rather than the worker-thread/`State::Loading` path `load` uses. The
/// content is already fully in memory (no git, and the only re-parsing is
/// the engine's own hunk analysis, which the task explicitly allows for this
/// one command); a synchronous call keeps every other piece of session state
/// (marks, keymap, scroll positions elsewhere) untouched, where reusing the
/// loading screen would mean tearing the whole `App` down and rebuilding it.
/// Builds the replacement items/view/reviewed into locals first and only
/// commits them to `app` once every step has succeeded, so a rejected
/// re-order (nothing to review, or nothing visible under the current
/// filters) never leaves `app.sel`/`app.view` pointing past a shrunk
/// `app.items`.
fn run_strategy(app: &mut App, name: &str) -> Result<(), String> {
    let strategy = parse_strategy(name)
        .ok_or_else(|| format!("unknown strategy '{name}' (want: comprehension, defs-first, file)"))?;
    let input = Input {
        changes: changes_from_sources(&app.sources),
        options: Options { strategy, cross_file: true, full_context: false, only_comments: false },
    };
    let out = ordo::run(input);
    let items = build_items(&out);
    let groups = group_reasons(&out);
    if items.is_empty() {
        return Err("that strategy leaves nothing to review".to_string());
    }
    let reviewed: Vec<bool> = items
        .iter()
        .map(|it| mark_key(&app.rev, it, &app.sources).is_some_and(|k| app.marks.contains_key(&k)))
        .collect();
    let glob = app.path_filter.as_ref().map(|(_, g)| g);
    let view = compute_view(&items, app.comments_only, app.show_all, glob);
    if view.is_empty() {
        return Err("that strategy leaves nothing visible under the current filters".to_string());
    }
    app.items = items;
    app.reviewed = reviewed;
    app.view = view;
    app.groups = groups;
    app.jumps.clear();
    app.popup = None;
    app.search = None;
    select(app, app.view[0]);
    Ok(())
}

/// Recomputes `app.view` under new filter state and commits it — rejecting
/// (leaving `app` untouched) a combination that would leave nothing to
/// review, so a stray `:filter`/`:only-comments`/`:all` never strands the
/// review on an empty pane. Clamps `app.sel` onto the new view when the
/// selected hunk itself got filtered out.
fn set_filters(
    app: &mut App,
    comments_only: bool,
    show_all: bool,
    path_filter: Option<(String, PathGlobs)>,
) -> Result<(), String> {
    let glob = path_filter.as_ref().map(|(_, g)| g);
    let view = compute_view(&app.items, comments_only, show_all, glob);
    if view.is_empty() {
        return Err("that filter combination leaves nothing to review".to_string());
    }
    app.comments_only = comments_only;
    app.show_all = show_all;
    app.path_filter = path_filter;
    app.view = view;
    if !app.view.contains(&app.sel) {
        select(app, app.view[0]);
    }
    Ok(())
}

fn run_goto(app: &mut App, path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("usage: :goto <path>".to_string());
    }
    let target = app.view.iter().copied().find(|&i| app.items[i].path == path);
    let Some(idx) = target else {
        return Err(format!(
            "no visible hunk for path '{path}' (clear filters with :filter, :all, :only-comments)"
        ));
    };
    select(app, idx);
    app.focus = Pane::Code;
    Ok(())
}

/// Runs a full command line (without its leading `:`) — the command bar's
/// `Enter` handler once nothing is selected in its completion menu. `Err`
/// carries a message shown in a popup rather than silently doing nothing, so
/// a typo or a bad argument is never mistaken for "nothing happened".
fn execute_command(app: &mut App, line: &str) -> Result<CommandOutcome, String> {
    let (name, arg) = parse_command_line(line);
    match name {
        "" => Ok(CommandOutcome::None),
        "q" => Ok(CommandOutcome::Quit),
        "help" => {
            app.popup = Some(Popup {
                title: "commands".to_string(),
                lines: build_command_help().into_iter().map(prose).collect(),
                scroll: 0,
                hscroll: 0,
            });
            Ok(CommandOutcome::None)
        }
        "audit" => {
            let hidden = hidden_breakdown(
                &app.items,
                app.comments_only,
                app.show_all,
                app.path_filter.as_ref().map(|(_, g)| g),
            );
            app.popup = Some(Popup {
                title: "audit".to_string(),
                lines: build_audit(
                    &app.items,
                    app.view.len(),
                    &hidden,
                    &app.ledger,
                    app.path_filter.as_ref().map(|(p, _)| p.as_str()),
                )
                .into_iter()
                .map(prose)
                .collect(),
                scroll: 0,
                hscroll: 0,
            });
            Ok(CommandOutcome::None)
        }
        "only-comments" => {
            set_filters(app, !app.comments_only, app.show_all, app.path_filter.clone())?;
            Ok(CommandOutcome::None)
        }
        "all" => {
            set_filters(app, app.comments_only, !app.show_all, app.path_filter.clone())?;
            Ok(CommandOutcome::None)
        }
        "filter" => {
            let arg = arg.trim();
            let next = if arg.is_empty() {
                None
            } else {
                let globs = build_globs(&[arg.to_string()])?;
                Some((arg.to_string(), globs))
            };
            set_filters(app, app.comments_only, app.show_all, next)?;
            Ok(CommandOutcome::None)
        }
        "keys" => {
            let preset = arg.trim();
            let Some(km) = keymap(preset) else {
                return Err(format!("unknown key preset '{preset}' (want: vim, vscode)"));
            };
            app.keys = km;
            Ok(CommandOutcome::None)
        }
        "strategy" => {
            run_strategy(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "group" => {
            app.show_groups = !app.show_groups;
            Ok(CommandOutcome::None)
        }
        "goto" => {
            run_goto(app, arg.trim())?;
            Ok(CommandOutcome::None)
        }
        "e" => {
            let rev_arg = arg.trim();
            if rev_arg.is_empty() {
                return Err("usage: :e <rev>".to_string());
            }
            let Some(target) = resolve(rev_arg) else {
                return Err(format!(
                    "'{rev_arg}' is not a git revision, a commit range or a \
                     GitButler CLI ID (see `but status`)"
                ));
            };
            Ok(CommandOutcome::Reload(target, rev_arg.to_string()))
        }
        other => Err(format!("unknown command '{other}' — :help lists them")),
    }
}

// The TUI can't be driven headless (`ratatui::init()` needs a real tty), so
// what's testable here is factored into small pure functions above — cursor
// arithmetic, scroll-follow clamping, and symbol/signature/docstring
// extraction — and exercised directly.
#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    // ---- background-load progress messages ----

    #[test]
    fn read_progress_is_one_based() {
        assert_eq!(read_progress("src/foo.rs", 0, 30), "reading src/foo.rs (1/30)");
        assert_eq!(read_progress("src/foo.rs", 11, 30), "reading src/foo.rs (12/30)");
        assert_eq!(read_progress("src/foo.rs", 29, 30), "reading src/foo.rs (30/30)");
    }

    #[test]
    fn highlight_progress_is_one_based() {
        assert_eq!(highlight_progress(0, 27), "highlighting (1/27)");
        assert_eq!(highlight_progress(7, 27), "highlighting (8/27)");
    }

    // ---- cursor column/line arithmetic ----

    #[test]
    fn move_col_clamps_within_line() {
        let ls = lines(&["abc"]);
        let c = Cursor { line: 0, col: 0 };
        assert_eq!(move_col(c, &ls, -1), Cursor { line: 0, col: 0 });
        assert_eq!(move_col(c, &ls, 1), Cursor { line: 0, col: 1 });
        assert_eq!(move_col(Cursor { line: 0, col: 2 }, &ls, 5), Cursor { line: 0, col: 2 });
    }

    #[test]
    fn move_line_clamps_and_carries_column() {
        let ls = lines(&["hello", "hi", ""]);
        let c = Cursor { line: 0, col: 4 };
        assert_eq!(move_line(c, &ls, 1), Cursor { line: 1, col: 1 }); // "hi" only has col 0/1
        assert_eq!(move_line(c, &ls, 2), Cursor { line: 2, col: 0 }); // empty line -> col 0
        assert_eq!(move_line(c, &ls, -5), Cursor { line: 0, col: 4 });
    }

    #[test]
    fn line_start_end() {
        let ls = lines(&["abcdef"]);
        let c = Cursor { line: 0, col: 3 };
        assert_eq!(line_start(c, &ls), Cursor { line: 0, col: 0 });
        assert_eq!(line_end(c, &ls), Cursor { line: 0, col: 5 });
    }

    #[test]
    fn clamp_cursor_handles_shrunk_or_empty_files() {
        let ls = lines(&["ab"]);
        assert_eq!(clamp_cursor(Cursor { line: 5, col: 5 }, &ls), Cursor { line: 0, col: 1 });
        assert_eq!(clamp_cursor(Cursor { line: 0, col: 0 }, &[]), Cursor { line: 0, col: 0 });
    }

    // ---- word motion ----

    #[test]
    fn word_next_skips_word_then_whitespace() {
        let ls = lines(&["foo bar  baz"]);
        let c = Cursor { line: 0, col: 0 };
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 4 }); // "bar"
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 9 }); // "baz"
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 11 }); // no next word: last char of buffer
    }

    #[test]
    fn word_next_crosses_lines() {
        let ls = lines(&["foo", "bar"]);
        let c = word_next(Cursor { line: 0, col: 0 }, &ls);
        assert_eq!(c, Cursor { line: 1, col: 0 });
    }

    #[test]
    fn word_prev_mirrors_word_next() {
        let ls = lines(&["foo bar  baz"]);
        let c = word_prev(Cursor { line: 0, col: 9 }, &ls);
        assert_eq!(c, Cursor { line: 0, col: 4 });
        let c = word_prev(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 0 });
        let c = word_prev(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 0 });
    }

    #[test]
    fn word_end_lands_on_last_char_of_word() {
        let ls = lines(&["foo bar"]);
        let c = word_end(Cursor { line: 0, col: 0 }, &ls);
        assert_eq!(c, Cursor { line: 0, col: 2 }); // end of "foo"
        let c = word_end(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 6 }); // end of "bar"
    }

    #[test]
    fn word_motion_treats_punctuation_as_its_own_run() {
        let ls = lines(&["foo(bar)"]);
        let c = word_next(Cursor { line: 0, col: 0 }, &ls);
        assert_eq!(c, Cursor { line: 0, col: 3 }); // "("
        let c = word_next(c, &ls);
        assert_eq!(c, Cursor { line: 0, col: 4 }); // "bar"
    }

    // ---- paragraph motion ----

    #[test]
    fn para_next_prev_find_blank_lines() {
        let ls = lines(&["a", "b", "", "c", "d"]);
        assert_eq!(para_next(Cursor { line: 0, col: 0 }, &ls), Cursor { line: 2, col: 0 });
        assert_eq!(para_prev(Cursor { line: 4, col: 0 }, &ls), Cursor { line: 2, col: 0 });
        assert_eq!(para_prev(Cursor { line: 0, col: 0 }, &ls), Cursor { line: 0, col: 0 });
    }

    // ---- scroll-follow clamping ----

    #[test]
    fn follow_scroll_keeps_cursor_in_view() {
        assert_eq!(follow_scroll(5, 0, 10), 0); // already visible
        assert_eq!(follow_scroll(0, 5, 10), 0); // scrolled past top -> jump up
        assert_eq!(follow_scroll(20, 5, 10), 11); // scrolled past bottom -> jump down
        assert_eq!(follow_scroll(9, 0, 10), 0); // last visible row stays put
        assert_eq!(follow_scroll(10, 0, 10), 1); // one past the last visible row
    }

    // ---- byte/char column conversion ----

    #[test]
    fn char_byte_handles_multibyte() {
        let line = "héllo";
        assert_eq!(char_byte(line, 0), 0);
        assert_eq!(char_byte(line, 1), 1); // 'é' starts at byte 1
        assert_eq!(char_byte(line, 2), 3); // 'l' starts after the 2-byte 'é'
        assert_eq!(char_byte(line, 99), line.len());
    }

    // ---- symbol resolution + signature/docstring extraction ----

    fn parse(lang: tree_sitter::Language, src: &str) -> tree_sitter::Tree {
        let mut p = Parser::new();
        p.set_language(&lang).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn resolves_rust_call_site_to_its_definition() {
        let src = "/// adds two numbers\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    add(1, 2);\n}\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
        let root = tree.root_node();
        // cursor on `add` inside `add(1, 2)` — row 6, "    add(1, 2);"
        let point = Point { row: 6, column: 5 };
        let mut node = root.descendant_for_point_range(point, point).unwrap();
        while !node.kind().contains("identifier") {
            node = node.parent().unwrap();
        }
        let name = node_text(node, src);
        assert_eq!(name, "add");
        let kinds = def_kinds("f.rs");
        let def = find_definition(root, kinds, &name, src).unwrap();
        assert_eq!(def.kind(), "function_item");
        assert_eq!(signature(def, src), "fn add(a: i32, b: i32) -> i32");
        assert_eq!(doc_for("f.rs", def, src).as_deref(), Some("/// adds two numbers"));
    }

    #[test]
    fn reports_missing_rust_definition_plainly() {
        let src = "fn main() {\n    unknown_fn();\n}\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
        let root = tree.root_node();
        let point = Point { row: 1, column: 5 };
        let mut node = root.descendant_for_point_range(point, point).unwrap();
        while !node.kind().contains("identifier") {
            node = node.parent().unwrap();
        }
        let name = node_text(node, src);
        assert_eq!(name, "unknown_fn");
        assert!(find_definition(root, def_kinds("f.rs"), &name, src).is_none());
    }

    #[test]
    fn extracts_python_docstring_and_signature() {
        let src = "def greet(name):\n    \"\"\"Say hello.\"\"\"\n    print(name)\n";
        let tree = parse(tree_sitter_python::LANGUAGE.into(), src);
        let root = tree.root_node();
        let def = find_definition(root, def_kinds("f.py"), "greet", src).unwrap();
        assert_eq!(def.kind(), "function_definition");
        assert_eq!(signature(def, src), "def greet(name):");
        assert_eq!(doc_for("f.py", def, src).as_deref(), Some("\"\"\"Say hello.\"\"\""));
    }

    #[test]
    fn c_function_definition_name_comes_from_declarator() {
        // C has no `name` field on function_definition — the identifier is
        // nested in `declarator`, past the parameter_list.
        let src = "// squares x\nint square(int x) {\n    return x * x;\n}\n";
        let tree = parse(tree_sitter_c::LANGUAGE.into(), src);
        let root = tree.root_node();
        let def = find_definition(root, def_kinds("f.c"), "square", src).unwrap();
        assert_eq!(def.kind(), "function_definition");
        assert_eq!(signature(def, src), "int square(int x)");
        assert_eq!(doc_for("f.c", def, src).as_deref(), Some("// squares x"));
    }

    // ---- search: symbol occurrences vs. text search ----

    #[test]
    fn byte_to_char_col_is_the_inverse_of_char_byte() {
        let line = "héllo";
        for col in 0..line.chars().count() {
            assert_eq!(byte_to_char_col(line, char_byte(line, col)), col);
        }
    }

    // A real case from a test corpus: a local `may_refine` bound once and used
    // twice, alongside `may_refine_camber_span` (which contains `may_refine`
    // as a substring) appearing 4 times. Symbol-occurrence search must find
    // exactly the 3 identifier nodes named `may_refine` and none of the 4
    // longer-named ones; text search, by contrast, matches the substring
    // everywhere it appears — including inside the longer name.
    #[test]
    fn symbol_occurrences_ignore_substring_matches() {
        let src = "def refine(may_refine_camber_span):\n\
                   \x20   may_refine = may_refine_camber_span > 0\n\
                   \x20   if may_refine:\n\
                   \x20       return may_refine_camber_span\n\
                   \x20   return may_refine or may_refine_camber_span\n";
        let lines: Vec<String> = src.lines().map(str::to_string).collect();
        let tree = parse(tree_sitter_python::LANGUAGE.into(), src);
        let root = tree.root_node();

        let matches = symbol_matches(root, "may_refine", src, &lines);
        assert_eq!(matches.len(), 3); // 1 binding + 2 uses
        for &(line, s, e) in &matches {
            assert_eq!(&lines[line][s..e], "may_refine");
        }

        let long_matches = symbol_matches(root, "may_refine_camber_span", src, &lines);
        assert_eq!(long_matches.len(), 4);
    }

    #[test]
    fn text_search_matches_substrings_unlike_symbol_search() {
        let src = "def refine(may_refine_camber_span):\n\
                   \x20   may_refine = may_refine_camber_span > 0\n\
                   \x20   if may_refine:\n\
                   \x20       return may_refine_camber_span\n\
                   \x20   return may_refine or may_refine_camber_span\n";
        let lines: Vec<String> = src.lines().map(str::to_string).collect();
        // every occurrence of the literal substring, standalone or embedded —
        // 3 standalone `may_refine` + 4 embedded in `may_refine_camber_span`
        let matches = text_matches(&lines, "may_refine");
        assert_eq!(matches.len(), 7);
    }

    #[test]
    fn text_matches_empty_pattern_finds_nothing() {
        let lines = lines(&["abc"]);
        assert!(text_matches(&lines, "").is_empty());
    }

    // ---- next/previous match: seeking and wrap-around ----

    #[test]
    fn cycle_index_wraps_both_directions() {
        assert_eq!(cycle_index(0, 3, 1), 1);
        assert_eq!(cycle_index(2, 3, 1), 0); // wraps forward
        assert_eq!(cycle_index(0, 3, -1), 2); // wraps backward
        assert_eq!(cycle_index(0, 0, 1), 0); // empty: no panic, degenerate 0
    }

    #[test]
    fn seek_forward_wraps_when_nothing_ahead() {
        let matches = vec![(0, 0, 3), (2, 0, 3)];
        let cursor = Cursor { line: 5, col: 0 };
        assert_eq!(seek_forward(&matches, cursor, true), Some(0));
        // strict (non-inclusive) seek skips a match starting exactly at cursor
        let cursor = Cursor { line: 0, col: 0 };
        assert_eq!(seek_forward(&matches, cursor, false), Some(1));
        assert_eq!(seek_forward(&matches, cursor, true), Some(0));
    }

    #[test]
    fn seek_backward_wraps_when_nothing_behind() {
        let matches = vec![(0, 0, 3), (2, 0, 3)];
        let cursor = Cursor { line: 0, col: 0 };
        assert_eq!(seek_backward(&matches, cursor, false), Some(1)); // wraps to last
        let cursor = Cursor { line: 2, col: 0 };
        assert_eq!(seek_backward(&matches, cursor, true), Some(1));
        assert_eq!(seek_backward(&matches, cursor, false), Some(0));
    }

    #[test]
    fn search_seek_on_empty_matches_returns_none() {
        let cursor = Cursor { line: 0, col: 0 };
        assert_eq!(seek_forward(&[], cursor, true), None);
        assert_eq!(seek_backward(&[], cursor, false), None);
    }

    // ---- cross-commit history: identity, windowing, labeling ----

    fn sym(name: &str, kind: &str, scope: Option<&str>) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: kind.to_string(),
            scope: scope.map(str::to_string),
        }
    }

    // The project owner's rule, verbatim: "ordo tree sitter type + scope for
    // the symbol must match, otherwise it's a different symbol". A module-level
    // `run` must NOT match a method `run` on class `A`, nor a same-scope `run`
    // of a different tree-sitter kind — name alone is never enough.
    #[test]
    fn symbol_eq_requires_matching_name_kind_and_scope() {
        let module_level = sym("run", "function_definition", None);
        let method_a = sym("run", "function_definition", Some("A"));
        assert!(!symbol_eq(&module_level, &method_a));

        let different_kind = sym("run", "async_function_definition", None);
        assert!(!symbol_eq(&module_level, &different_kind));

        let same = sym("run", "function_definition", None);
        assert!(symbol_eq(&module_level, &same));
    }

    #[test]
    fn qualified_name_joins_scope_and_name() {
        assert_eq!(qualified_name(&sym("run", "function_definition", None)), "run");
        assert_eq!(
            qualified_name(&sym("run", "function_definition", Some("A"))),
            "A.run"
        );
    }

    #[test]
    fn bound_earlier_drops_current_and_orders_oldest_of_window_first() {
        let shas: Vec<String> = ["current", "a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let got = bound_earlier(shas, "current", 2);
        assert_eq!(got, vec!["b".to_string(), "a".to_string()]); // nearest 2, oldest first
    }

    #[test]
    fn bound_earlier_bounds_a_long_lived_files_history() {
        let shas: Vec<String> = (0..20).map(|i| format!("c{i}")).collect(); // c0 nearest
        let got = bound_earlier(shas, "none-of-these", 10);
        assert_eq!(got.len(), 10);
        assert_eq!(got.first().unwrap(), "c9"); // oldest of the nearest-10 window
        assert_eq!(got.last().unwrap(), "c0"); // nearest to the reviewed rev
    }

    #[test]
    fn bound_later_orders_nearest_to_rev_first_and_bounds_the_window() {
        // rev-list's own order: HEAD first, nearest descendant of rev last
        let shas: Vec<String> = ["head", "b", "a"].iter().map(|s| s.to_string()).collect();
        let got = bound_later(shas, 10);
        assert_eq!(got, vec!["a".to_string(), "b".to_string(), "head".to_string()]);

        let many: Vec<String> = (0..20).map(|i| format!("c{i}")).collect(); // c19 nearest to rev
        let got = bound_later(many, 10);
        assert_eq!(got.len(), 10);
        assert_eq!(got.first().unwrap(), "c19");
    }

    fn test_hunk(rationale: &str, enclosing: Option<&str>, symbols: Vec<Symbol>) -> HunkOut {
        HunkOut {
            id: "h1".to_string(),
            old_range: [0, 0],
            new_range: [1, 1],
            category: ordo::model::Category::Definition,
            enclosing: enclosing.map(str::to_string),
            enclosing_kind: None,
            defines: vec![],
            uses: vec![],
            group: "g".to_string(),
            order_index: 0,
            rationale: rationale.to_string(),
            noise: false,
            comment: false,
            details: vec![],
            notes: vec![],
            advisories: vec![],
            symbols,
        }
    }

    #[test]
    fn label_for_picks_the_fragment_naming_the_symbol() {
        let target = sym("run", "function_definition", None);
        let h = test_hunk("adds helper; changes signature of run", None, vec![]);
        assert_eq!(label_for(&h, &target), "changes signature of run");
    }

    #[test]
    fn label_for_falls_back_to_the_whole_rationale_for_a_body_edit() {
        let target = sym("run", "function_definition", Some("A"));
        let h = test_hunk("edits A.run", Some("A.run"), vec![]);
        assert_eq!(label_for(&h, &target), "edits A.run");
    }

    // ---- open-in-editor: command building ----

    #[test]
    fn split_command_separates_program_and_flags() {
        assert_eq!(split_command("code --wait"), vec!["code", "--wait"]);
        assert_eq!(split_command("vim"), vec!["vim"]);
        assert_eq!(split_command("emacsclient  -nw  -a ''"), vec!["emacsclient", "-nw", "-a", "''"]);
    }

    #[test]
    fn editor_args_covers_each_line_argument_shape() {
        assert_eq!(editor_args("vim", "path", 120), vec!["+120".to_string(), "path".to_string()]);
        assert_eq!(editor_args("nvim", "path", 120), vec!["+120".to_string(), "path".to_string()]);
        assert_eq!(editor_args("hx", "path", 120), vec!["path:120".to_string()]);
        assert_eq!(
            editor_args("code", "path", 120),
            vec!["-g".to_string(), "path:120".to_string()]
        );
        // unknown editor: file only — never a guessed syntax it might read as
        // another filename
        assert_eq!(editor_args("subl", "path", 120), vec!["path".to_string()]);
    }

    #[test]
    fn build_command_keeps_editor_flags_ahead_of_the_location() {
        let spec = split_command("code --wait");
        let (program, args) = build_command(&spec, "path", 120).unwrap();
        assert_eq!(program, "code");
        assert_eq!(args, vec!["--wait".to_string(), "-g".to_string(), "path:120".to_string()]);
    }

    #[test]
    fn build_command_matches_on_basename_not_full_path() {
        let spec = split_command("/usr/local/bin/hx");
        let (program, args) = build_command(&spec, "path", 120).unwrap();
        assert_eq!(program, "/usr/local/bin/hx");
        assert_eq!(args, vec!["path:120".to_string()]);
    }

    #[test]
    fn build_command_is_none_for_an_empty_spec() {
        assert!(build_command(&[], "path", 120).is_none());
    }

    // ---- key/chord rendering ----

    #[test]
    fn key_label_renders_plain_and_modified_keys() {
        assert_eq!(key_label(ch('j')), "j");
        assert_eq!(key_label(ctrl('w')), "C-w");
        assert_eq!(key_label(plain(KeyCode::F(12))), "F12");
        assert_eq!(key_label((KeyCode::F(3), KeyModifiers::SHIFT)), "S-F3");
        assert_eq!(key_label(plain(KeyCode::Esc)), "Esc");
    }

    #[test]
    fn chord_label_renders_known_chords() {
        // plain-char chords render tight, like vim's own "gg"/"ge" spelling
        assert_eq!(chord_label(Some(ch('g')), ch('g')), "gg");
        assert_eq!(chord_label(Some(ch('g')), ch('e')), "ge");
        // a modified prefix or key renders spaced, like "C-w C-w"
        assert_eq!(chord_label(Some(ctrl('w')), ctrl('w')), "C-w C-w");
        assert_eq!(chord_label(Some(ctrl('w')), ch('h')), "C-w h");
        assert_eq!(chord_label(None, ch('q')), "q");
    }

    // ---- generated keybinding help ----

    #[test]
    fn help_text_is_generated_from_the_bind_table() {
        let km = keymap("vim").unwrap();
        let help = build_help(&km);
        // `Hover` is bound to `K` in vim's own bind table (see `keymap`) — if
        // that binding ever changes, this row (built from the table, not
        // hand-copied) changes with it, and this assertion breaks.
        let (_, desc) = action_help(Action::Hover);
        let row = help.iter().find(|l| l.contains(desc)).expect("hover row present");
        assert!(row.contains('K'), "expected the hover row to list K: {row}");
    }

    #[test]
    fn help_collapses_multiple_keys_bound_to_the_same_action() {
        let km = keymap("vim").unwrap();
        let help = build_help(&km);
        // `Next` is bound to both `j` and `Down` — one row, both keys.
        let (_, desc) = action_help(Action::Next);
        let row = help.iter().find(|l| l.contains(desc)).expect("next row present");
        assert!(row.contains('j') && row.contains("Down"), "expected both keys on one row: {row}");
    }

    #[test]
    fn help_covers_both_presets_without_panicking() {
        for name in ["vim", "vscode"] {
            let km = keymap(name).unwrap();
            let help = build_help(&km);
            assert!(!help.is_empty());
        }
    }

    #[test]
    fn excerpt_numbers_lines_and_uses_highlight_segments_when_present() {
        let lines: Vec<String> = vec!["fn a() {}".into(), "let x = 1;".into(), "done".into()];
        // no grammar: falls back to raw text, still gutter-numbered
        let plainly = excerpt(&lines, None, 2, 3);
        assert_eq!(plainly.len(), 2);
        let first: String = plainly[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(first, "    2 let x = 1;");

        // with highlights: the segments are used, each carrying its own colour
        let hl: Vec<LineSpans> = vec![
            vec![],
            vec![("let ".to_string(), Color::Magenta), ("x = 1;".to_string(), Color::Reset)],
            vec![],
        ];
        let lit = excerpt(&lines, Some(&hl), 2, 2);
        assert_eq!(lit.len(), 1);
        assert_eq!(lit[0].spans.len(), 3, "gutter + two coloured segments");
        assert_eq!(lit[0].spans[1].style.fg, Some(Color::Magenta));
    }

    #[test]
    fn popup_width_measures_the_widest_line_across_its_spans() {
        let lines = vec![
            Line::from("short"),
            Line::from(vec![Span::raw("12345"), Span::raw("67890")]),
            Line::from("mid"),
        ];
        assert_eq!(popup_width(&lines), 10, "spans on one line sum");
        assert_eq!(popup_width(&[]), 0);
    }

    #[test]
    fn excerpt_stops_at_the_end_of_the_file() {
        let lines: Vec<String> = vec!["only".into()];
        assert_eq!(excerpt(&lines, None, 1, 9).len(), 1);
        assert!(excerpt(&lines, None, 5, 9).is_empty());
    }

    #[test]
    fn no_preset_binds_the_same_chord_twice() {
        // `Keymap::resolve` takes the first match, so a duplicate (prefix, key)
        // is silently shadowed rather than rejected — the second binding just
        // never fires. These tables have been edited by many separate changes,
        // so the invariant is worth asserting rather than assuming.
        for name in ["vim", "vscode"] {
            let km = keymap(name).unwrap();
            let mut seen: Vec<(Option<Key>, Key)> = vec![];
            for (prefix, key, _) in &km.binds {
                let pair = (*prefix, *key);
                assert!(
                    !seen.contains(&pair),
                    "{name}: {} is bound twice",
                    chord_label(*prefix, *key)
                );
                seen.push(pair);
            }
        }
    }

    #[test]
    fn every_bound_action_has_a_help_entry() {
        // The `?` help is generated from the bind table, so an Action without a
        // description would render a blank row instead of documenting the key.
        for name in ["vim", "vscode"] {
            let km = keymap(name).unwrap();
            for (prefix, key, action) in &km.binds {
                let (_, desc) = action_help(*action);
                assert!(
                    !desc.trim().is_empty(),
                    "{name}: {} has no help description",
                    chord_label(*prefix, *key)
                );
            }
        }
    }

    // ---- command mode: line parsing ----

    #[test]
    fn parse_command_line_splits_name_and_argument() {
        assert_eq!(parse_command_line("strategy defs-first"), ("strategy", "defs-first"));
        assert_eq!(parse_command_line("filter   src/*   "), ("filter", "src/*"));
        assert_eq!(parse_command_line("q"), ("q", ""));
        assert_eq!(parse_command_line("  q  "), ("q", ""));
        assert_eq!(parse_command_line(""), ("", ""));
    }

    // ---- command mode: the completion matcher ----

    #[test]
    fn complete_prefers_a_prefix_match_over_a_substring_one() {
        let candidates = lines(&["comprehension", "defs-first", "file"]);
        // "f" prefixes "file" but is also a substring of "defs-first" — the
        // prefix match must win outright, not just sort first
        assert_eq!(complete("f", &candidates), vec!["file".to_string()]);
    }

    #[test]
    fn complete_falls_back_to_substring_when_no_prefix_matches() {
        let candidates = lines(&["comprehension", "defs-first", "file"]);
        // "first" isn't a prefix of anything, but is a substring of "defs-first"
        assert_eq!(complete("first", &candidates), vec!["defs-first".to_string()]);
    }

    #[test]
    fn complete_is_case_insensitive() {
        let candidates = lines(&["vim", "vscode"]);
        assert_eq!(complete("VS", &candidates), vec!["vscode".to_string()]);
    }

    #[test]
    fn complete_empty_input_lists_everything_unfiltered() {
        let candidates = lines(&["vim", "vscode"]);
        assert_eq!(complete("", &candidates), candidates);
    }

    #[test]
    fn complete_no_match_yields_an_empty_menu() {
        let candidates = lines(&["vim", "vscode"]);
        assert!(complete("zzz", &candidates).is_empty());
    }

    // ---- command mode: argument completion per command ----

    #[test]
    fn command_completions_lists_command_names_before_any_space() {
        let got = command_completions("str", &[], &[], &[]);
        assert_eq!(got, vec!["strategy".to_string()]);
    }

    #[test]
    fn command_completions_lists_keys_presets() {
        let got = command_completions("keys ", &[], &[], &[]);
        assert_eq!(got, vec!["vim".to_string(), "vscode".to_string()]);
    }

    #[test]
    fn command_completions_lists_strategy_names() {
        let got = command_completions("strategy ", &[], &[], &[]);
        assert_eq!(got, vec!["comprehension".to_string(), "defs-first".to_string(), "file".to_string()]);
    }

    #[test]
    fn command_completions_lists_goto_paths_and_filter_dirs_from_their_own_pools() {
        let goto_paths = lines(&["src/a.rs", "src/b.rs"]);
        let filter_dirs = lines(&["src", "tests"]);
        assert_eq!(command_completions("goto ", &goto_paths, &filter_dirs, &[]), goto_paths);
        assert_eq!(command_completions("filter ", &goto_paths, &filter_dirs, &[]), filter_dirs);
    }

    #[test]
    fn command_completions_narrows_the_argument_by_its_own_partial_word() {
        let goto_paths = lines(&["src/a.rs", "src/b.rs"]);
        assert_eq!(command_completions("goto src/b", &goto_paths, &[], &[]), vec!["src/b.rs".to_string()]);
    }

    #[test]
    fn command_completions_is_empty_for_a_no_argument_command() {
        assert!(command_completions("q ", &[], &[], &[]).is_empty());
        assert!(command_completions("only-comments ", &[], &[], &[]).is_empty());
    }

    #[test]
    fn apply_completion_replaces_the_word_being_completed_and_adds_a_trailing_space() {
        assert_eq!(apply_completion("str", "strategy"), "strategy ");
        assert_eq!(apply_completion("strategy defs", "defs-first"), "strategy defs-first ");
    }

    #[test]
    fn dir_prefix_and_distinct_sorted_derive_stable_glob_candidates() {
        assert_eq!(dir_prefix("src/bin/ordo-tui.rs"), Some("src/bin"));
        assert_eq!(dir_prefix("Cargo.toml"), None);
        let got = distinct_sorted(["src/b.rs", "src/a.rs", "src/a.rs"].into_iter());
        assert_eq!(got, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    }

    // ---- command mode: `:help` generated from the command table ----

    #[test]
    fn command_help_is_generated_from_the_command_table() {
        let help = build_command_help();
        // if `strategy`'s entry in `COMMANDS` ever changes, this row (built
        // from the table, not hand-copied) changes with it, and this breaks
        let strategy = COMMANDS.iter().find(|c| c.name == "strategy").unwrap();
        let row = help.iter().find(|l| l.contains(strategy.help)).expect("strategy row present");
        assert!(row.contains(":strategy"), "expected the command name in the row: {row}");
    }

    #[test]
    fn every_command_has_a_non_empty_help_line() {
        for c in COMMANDS {
            assert!(!c.help.trim().is_empty(), ":{} has no help text", c.name);
        }
    }

    // ---- command mode: filter view + index clamping ----

    #[test]
    fn hidden_breakdown_partitions_the_hidden_set_exactly() {
        let mut noisy = test_item("src/a.rs");
        noisy.noise = true;
        let mut both = test_item("tests/b.rs"); // noise AND outside the glob
        both.noise = true;
        let outside = test_item("tests/c.rs");
        let shown = test_item("src/d.rs");
        let items = vec![noisy, both, outside, shown];
        let globs = build_globs(&["src/*".to_string()]).unwrap();

        let h = hidden_breakdown(&items, false, false, Some(&globs));
        // an item hidden twice is charged once, to the first reason
        assert_eq!(h, Hidden { comment: 0, noise: 2, glob: 1, unaccounted: 0 });
        assert_eq!(
            compute_view(&items, false, false, Some(&globs)).len() + h.noise + h.glob,
            items.len()
        );
    }

    #[test]
    fn hidden_breakdown_charges_only_comments_before_noise() {
        let mut a = test_item("a.rs");
        a.comment = true;
        let mut b = test_item("b.rs");
        b.noise = true; // hidden by only-comments first, not by noise
        let items = vec![a, b];

        let h = hidden_breakdown(&items, true, false, None);
        assert_eq!(h, Hidden { comment: 1, noise: 0, glob: 0, unaccounted: 0 });
    }

    #[test]
    fn build_audit_reports_every_reason_and_flags_an_unaccounted_remainder() {
        let items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
        let ledger = Ledger {
            files_seen: 9,
            files_generated: 2,
            files_declared: 1,
            files_globbed: 3,
            files_unreadable: 1,
            hunks_import: 4,
            hunks_non_comment: 0,
        };
        let clean = Hidden { comment: 0, noise: 1, glob: 0, unaccounted: 0 };
        let text = build_audit(&items, 2, &clean, &ledger, None).join("\n");
        assert!(text.contains("2 of 3 hunks shown"), "{text}");
        assert!(text.contains("   4  pure import hunk"), "{text}");
        assert!(text.contains("files: 9 changed"), "{text}");
        assert!(text.contains("never fetched: excluded by a launch-time glob"), "{text}");
        assert!(text.contains("every hidden hunk is accounted for"), "{text}");

        let leak = Hidden { comment: 0, noise: 0, glob: 0, unaccounted: 1 };
        let text = build_audit(&items, 2, &leak, &ledger, Some("src/*")).join("\n");
        assert!(text.contains("1 hidden hunk unaccounted for"), "{text}");
        assert!(text.contains("outside the path filter 'src/*'"), "{text}");
    }

    #[test]
    fn compute_view_applies_only_comments_show_all_and_glob_independently() {
        let mut a = test_item("src/a.rs");
        a.comment = true;
        let mut b = test_item("src/b.rs");
        b.noise = true;
        let c = test_item("tests/c.rs");
        let items = vec![a, b, c];

        assert_eq!(compute_view(&items, false, true, None), vec![0, 1, 2]);
        assert_eq!(compute_view(&items, true, true, None), vec![0]); // only-comments
        assert_eq!(compute_view(&items, false, false, None), vec![0, 2]); // hide noise

        let globs = build_globs(&["src/*".to_string()]).unwrap();
        assert_eq!(compute_view(&items, false, true, Some(&globs)), vec![0, 1]);
    }

    #[test]
    fn set_filters_clamps_selection_off_a_hunk_that_falls_out_of_view() {
        let mut app = test_app(0);
        app.items = vec![test_item("a.rs"), test_item("b.rs"), test_item("c.rs")];
        app.view = vec![0, 1, 2];
        app.reviewed = vec![false, false, false];
        app.sel = 1; // b.rs — about to be filtered out
        app.popup = Some(Popup { title: "x".to_string(), lines: vec![], scroll: 0, hscroll: 0 });

        let globs = build_globs(&["a.rs".to_string()]).unwrap();
        set_filters(&mut app, false, true, Some(("a.rs".to_string(), globs))).unwrap();

        assert_eq!(app.view, vec![0]);
        assert_eq!(app.sel, 0, "selection must move off the now-hidden item");
        assert!(app.popup.is_none(), "select() clears a stale popup");
    }

    #[test]
    fn set_filters_rejects_a_combination_that_would_empty_the_view() {
        let mut app = test_app(0);
        app.items = vec![test_item("a.rs")];
        app.view = vec![0];
        app.reviewed = vec![false];

        let globs = build_globs(&["nope/*".to_string()]).unwrap();
        let err = set_filters(&mut app, false, true, Some(("nope/*".to_string(), globs)));
        assert!(err.is_err());
        // rejected: state is untouched, still pointing at the one real item
        assert_eq!(app.view, vec![0]);
        assert!(app.path_filter.is_none());
    }

    #[test]
    fn view_pos_and_first_last_visible_read_the_view_not_the_raw_item_list() {
        assert_eq!(view_pos(&[3, 5, 8], 5), 1);
        assert_eq!(view_pos(&[3, 5, 8], 99), 0); // not found: degrades to 0
        let mut app = test_app(0);
        app.view = vec![3, 5, 8];
        assert_eq!(first_visible(&app), 3);
        assert_eq!(last_visible(&app), 8);
    }

    // ---- horizontal scroll: cursor-follow ----

    #[test]
    fn follow_hscroll_keeps_cursor_in_view() {
        assert_eq!(follow_hscroll(5, 0, 10), 0); // already visible: no movement
        assert_eq!(follow_hscroll(15, 0, 10), 6); // right of the window: scrolls right
        assert_eq!(follow_hscroll(2, 6, 10), 2); // left of the window: scrolls back
    }

    // ---- horizontal scroll: span slicing ----

    #[test]
    fn slice_range_crops_by_char_column_not_byte() {
        // "héllo world" — é is 2 bytes, so a byte-based slice would misalign
        // every column after it.
        let spans = vec![Span::styled("héllo world".to_string(), Style::default())];
        let sliced = slice_range(spans, 1, 3);
        let text: String = sliced.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "éll");
    }

    #[test]
    fn slice_range_drops_content_outside_the_window() {
        let spans = vec![Span::styled("hello".to_string(), Style::default())];
        assert!(slice_range(spans.clone(), 10, 5).is_empty());
        let sliced = slice_range(spans, 0, 2);
        let text: String = sliced.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "he");
    }

    // ---- horizontal scroll: the gutter stays fixed ----

    fn test_item(path: &str) -> Item {
        Item {
            path: path.to_string(),
            old_range: [0, 0],
            new_range: [0, 0],
            mark: String::new(),
            cat: "other".to_string(),
            rationale: String::new(),
            details: vec![],
            notes: vec![],
            edges: vec![],
            advisories: vec![],
            noise: false,
            comment: false,
            symbols: vec![],
            enclosing: None,
            group: String::new(),
            refined: ordo::refine::Refined::default(),
        }
    }

    fn edge(label: &str, target: Option<usize>) -> EdgeRef {
        EdgeRef { label: label.to_string(), target }
    }

    #[test]
    fn code_view_horizontal_scroll_keeps_gutter_fixed_and_handles_multibyte() {
        let it = test_item("f.rs");
        // a multi-byte line, long enough to be clipped on the right at a
        // narrow width
        let line = "let héllo_world = 1234567890abcdef;".to_string();
        let mut sources: Sources = HashMap::new();
        sources.insert("f.rs".to_string(), (vec![], vec![line]));
        let highlights: Highlights = HashMap::new();
        // width = gutter (6) + 10 cols of code
        let (unscrolled, right_clip_0) =
            code_view(&it, &sources, &highlights, 16, 0, None, &[], None, &Theme::dark());
        let (scrolled, right_clip_5) =
            code_view(&it, &sources, &highlights, 16, 5, None, &[], None, &Theme::dark());
        assert_eq!(unscrolled.len(), 1);
        assert_eq!(scrolled.len(), 1);
        // the sign-bar and line-number gutter (the row's first two spans)
        // never move, regardless of horizontal scroll
        let gutter = |line: &Line<'static>| -> Vec<String> {
            line.spans.iter().take(2).map(|s| s.content.to_string()).collect()
        };
        assert_eq!(gutter(&unscrolled[0]), gutter(&scrolled[0]));
        // but the code past the gutter does shift with hscroll
        let rest = |line: &Line<'static>| -> String {
            line.spans[2..].iter().map(|s| s.content.to_string()).collect()
        };
        assert_ne!(rest(&unscrolled[0]), rest(&scrolled[0]));
        // both directions were clipped at this width, so both report it
        assert!(right_clip_0);
        assert!(right_clip_5);
    }

    #[test]
    fn code_view_tints_only_the_refined_span_of_a_paired_line() {
        let old = "fn f(a: A) {}".to_string();
        let new = "fn f(a: A, b: B) {}".to_string();
        let mut it = test_item("f.rs");
        it.old_range = [1, 1];
        it.new_range = [1, 1];
        let mut sources: Sources = HashMap::new();
        sources.insert("f.rs".to_string(), (vec![old.clone()], vec![new.clone()]));
        let mut items = vec![it];
        refine_items(&mut items, &sources);
        let it = &items[0];
        assert_eq!(
            it.refined.added[0],
            Some(vec![(9, 15)]),
            "expected only `, b: B` to be refined"
        );

        let theme = Theme::dark();
        let (rows, _) = code_view(it, &sources, &HashMap::new(), 60, 0, None, &[], None, &theme);
        // the added row is the one carrying the add tint (the removed row
        // comes first, on the del tint)
        let added = rows.last().expect("an added row");
        // walk the row char by char: the strong tint covers `, b: B` and
        // nothing else
        let mut strong = String::new();
        for span in &added.spans {
            if span.style.bg == Some(theme.add_strong_bg) {
                strong.push_str(&span.content);
            }
        }
        assert_eq!(strong, ", b: B");
    }

    #[test]
    fn code_view_tints_a_whole_unpaired_line() {
        // nothing in common with the removed line, so no span is singled out
        let mut it = test_item("f.rs");
        it.old_range = [1, 1];
        it.new_range = [1, 1];
        let mut sources: Sources = HashMap::new();
        sources.insert(
            "f.rs".to_string(),
            (vec!["use std::io;".to_string()], vec!["fn totally(different: X) {}".to_string()]),
        );
        let mut items = vec![it];
        refine_items(&mut items, &sources);
        assert_eq!(items[0].refined.added[0], None);

        let theme = Theme::dark();
        let (rows, _) =
            code_view(&items[0], &sources, &HashMap::new(), 60, 0, None, &[], None, &theme);
        let added = rows.last().unwrap();
        assert!(
            added.spans.iter().all(|s| s.style.bg != Some(theme.add_strong_bg)),
            "an unpaired line must not be partially tinted"
        );
    }

    #[test]
    fn code_view_reports_no_clipping_when_the_line_fits() {
        let it = test_item("f.rs");
        let mut sources: Sources = HashMap::new();
        sources.insert("f.rs".to_string(), (vec![], vec!["short".to_string()]));
        let highlights: Highlights = HashMap::new();
        let (_, right_clip) = code_view(&it, &sources, &highlights, 40, 0, None, &[], None, &Theme::dark());
        assert!(!right_clip);
    }

    // ---- def→use edges: target resolution ----

    fn id_hunk(id: &str) -> HunkOut {
        let mut h = test_hunk("does a thing", None, vec![]);
        h.id = id.to_string();
        h
    }

    fn test_file_out(path: &str, hunks: Vec<HunkOut>) -> ordo::model::FileOut {
        ordo::model::FileOut {
            path: path.to_string(),
            hunks,
            degraded: false,
            unsupported: false,
            dropped: vec![],
        }
    }

    #[test]
    fn build_items_resolves_edge_targets_within_the_review_and_flags_ones_outside_it() {
        let out = Output {
            schema: 1,
            order: vec![
                ordo::model::OrderItem { path: "a.rs".to_string(), hunk: "h1".to_string() },
                ordo::model::OrderItem { path: "a.rs".to_string(), hunk: "h2".to_string() },
            ],
            files: vec![test_file_out("a.rs", vec![id_hunk("h1"), id_hunk("h2")])],
            groups: vec![],
            edges: vec![
                // resolves: h2 is item index 1
                ordo::model::Edge {
                    from: "h1".to_string(),
                    to: "h2".to_string(),
                    why: "uses it".to_string(),
                },
                // doesn't resolve: "ghost" was never built into an item (e.g.
                // filtered out, or a cross-file edge to an unreviewed file)
                ordo::model::Edge {
                    from: "h1".to_string(),
                    to: "ghost".to_string(),
                    why: "calls it".to_string(),
                },
            ],
            clusters: vec![],
        };
        let items = build_items(&out);
        assert_eq!(items.len(), 2);
        assert!(items[0].edges.iter().any(|e| e.target == Some(1)));
        assert!(items[0].edges.iter().any(|e| e.target.is_none()));
    }

    // ---- position stack ----

    #[test]
    fn stack_pop_on_an_empty_stack_is_a_no_op() {
        let mut stack: Vec<(usize, Cursor)> = vec![];
        assert_eq!(stack_pop_valid(&mut stack, 10), None);
        assert!(stack.is_empty());
    }

    #[test]
    fn stack_push_then_pop_returns_you_to_the_pushed_position() {
        let mut stack: Vec<(usize, Cursor)> = vec![];
        let pos = (2, Cursor { line: 5, col: 1 });
        stack_push(&mut stack, pos);
        assert_eq!(stack_pop_valid(&mut stack, 10), Some(pos));
        assert!(stack.is_empty());
    }

    #[test]
    fn stack_push_is_bounded_dropping_the_oldest_entry_first() {
        let mut stack: Vec<(usize, Cursor)> = vec![];
        for i in 0..JUMP_STACK_CAP + 10 {
            stack_push(&mut stack, (i, Cursor { line: i, col: 0 }));
        }
        assert_eq!(stack.len(), JUMP_STACK_CAP);
        // the oldest 10 pushes were dropped to keep the cap
        assert_eq!(stack.first().unwrap().0, 10);
        assert_eq!(stack.last().unwrap().0, JUMP_STACK_CAP + 9);
    }

    #[test]
    fn stack_pop_valid_skips_an_entry_whose_index_no_longer_resolves() {
        // "99" is stale (out of range for a 5-item review) and sits on top —
        // it must be skipped, not returned, and the stack must not panic.
        let mut stack = vec![(0, Cursor { line: 1, col: 1 }), (99, Cursor { line: 0, col: 0 })];
        assert_eq!(stack_pop_valid(&mut stack, 5), Some((0, Cursor { line: 1, col: 1 })));
        assert!(stack.is_empty());
    }

    // ---- why pane: dep-line resolution and cursor ----

    #[test]
    fn why_rows_marks_edge_lines_and_carries_their_target() {
        let mut it = test_item("a.rs");
        it.edges = vec![edge("→ a.rs:L10   uses it", Some(3)), edge("→ b.rs:L1   calls it", None)];
        // target 3 must be in `view` to resolve — same as being part of the
        // review at all; a 4-item view (0..=3) covers it here
        let rows = why_rows(&it, &[0, 1, 2, 3]);
        let edges: Vec<&WhyKind> = rows.iter().map(|r| &r.kind).filter(|k| matches!(k, WhyKind::Edge(_))).collect();
        assert!(matches!(edges[0], WhyKind::Edge(Some(3))));
        assert!(matches!(edges[1], WhyKind::Edge(None)));
    }

    #[test]
    fn why_rows_treats_a_filtered_out_target_as_not_part_of_the_review() {
        let mut it = test_item("a.rs");
        it.edges = vec![edge("→ a.rs:L10   uses it", Some(3))];
        // target 3 exists (it's a valid item index) but isn't in `view`
        let rows = why_rows(&it, &[0, 1, 2]);
        let edges: Vec<&WhyKind> = rows.iter().map(|r| &r.kind).filter(|k| matches!(k, WhyKind::Edge(_))).collect();
        assert!(matches!(edges[0], WhyKind::Edge(None)));
    }

    fn test_app(why_len: usize) -> App {
        App {
            items: vec![test_item("a.rs")],
            reviewed: vec![false],
            view: vec![0],
            comments_only: false,
            show_all: true,
            path_filter: None,
            command: None,
            sel: 0,
            scroll: 0,
            hscroll: 0,
            why_scroll: 0,
            why_sel: 0,
            code_len: 0,
            why_len,
            code_height: 10,
            why_height: 5,
            code_width: 10,
            focus: Pane::Why,
            keys: keymap("vim").unwrap(),
            pending: None,
            sources: HashMap::new(),
            highlights: HashMap::new(),
            cursor: Cursor { line: 0, col: 0 },
            popup: None,
            trees: HashMap::new(),
            prompt: None,
            search: None,
            review_sha: None,
            uncommitted: false,
            history_cache: HashMap::new(),
            jumps: vec![],
            rev: "HEAD".to_string(),
            marks_path: None,
            marks: HashMap::new(),
            theme: Theme::dark(),
            show_groups: false,
            groups: HashMap::new(),
            ledger: Ledger::default(),
        }
    }

    #[test]
    fn why_cursor_move_clamps_at_both_ends() {
        let mut app = test_app(3); // 3 logical why-pane lines: indices 0..=2
        why_cursor_move(&mut app, -1);
        assert_eq!(app.why_sel, 0, "can't move above the first line");
        why_cursor_move(&mut app, 1);
        assert_eq!(app.why_sel, 1);
        why_cursor_move(&mut app, 10);
        assert_eq!(app.why_sel, 2, "clamps at the last line");
        why_cursor_move(&mut app, 10);
        assert_eq!(app.why_sel, 2, "stays clamped past the last line");
    }

    // ---- reviewed-mark persistence ----

    // A known FNV-1a 64-bit test vector, hard-coded so a future refactor that
    // swaps in a different hash (or a different offset/prime) fails loudly
    // instead of silently invalidating every mark ever written.
    #[test]
    fn fnv1a_is_stable_for_a_known_input() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"hello"), 0xa430d84680aabd0b);
    }

    fn item_with(path: &str, old: [usize; 2], new: [usize; 2], symbols: Vec<Symbol>, enclosing: Option<&str>) -> Item {
        let mut it = test_item(path);
        it.old_range = old;
        it.new_range = new;
        it.symbols = symbols;
        it.enclosing = enclosing.map(str::to_string);
        it
    }

    fn sources_for(path: &str, old: &[&str], new: &[&str]) -> Sources {
        let mut s: Sources = HashMap::new();
        s.insert(path.to_string(), (lines(old), lines(new)));
        s
    }

    #[test]
    fn mark_key_changes_when_hunk_content_changes() {
        let it_a = item_with("f.rs", [1, 1], [1, 1], vec![sym("run", "function_item", None)], None);
        let src_a = sources_for("f.rs", &["fn run() {}"], &["fn run() { 1 }"]);
        let src_b = sources_for("f.rs", &["fn run() {}"], &["fn run() { 2 }"]);
        let ka = mark_key("HEAD", &it_a, &src_a).unwrap();
        let kb = mark_key("HEAD", &it_a, &src_b).unwrap();
        assert_ne!(ka, kb, "a body edit must drop the mark, never carry it over silently");
    }

    #[test]
    fn mark_key_covers_both_old_and_new_sides() {
        // same new content, different old content (e.g. a reformat that
        // happens to converge) — still a different key
        let it = item_with("f.rs", [1, 1], [1, 1], vec![], Some("run"));
        let src_a = sources_for("f.rs", &["fn run() { 1 }"], &["fn run() {\n    1\n}"]);
        let src_b = sources_for("f.rs", &["fn run() { 2 }"], &["fn run() {\n    1\n}"]);
        assert_ne!(
            mark_key("HEAD", &it, &src_a).unwrap(),
            mark_key("HEAD", &it, &src_b).unwrap()
        );
    }

    #[test]
    fn mark_key_is_unchanged_by_reordering_symbols() {
        let syms_a = vec![sym("a", "function_item", None), sym("b", "function_item", None)];
        let syms_b = vec![sym("b", "function_item", None), sym("a", "function_item", None)];
        let it_a = item_with("f.rs", [1, 1], [1, 2], syms_a, None);
        let it_b = item_with("f.rs", [1, 1], [1, 2], syms_b, None);
        let src = sources_for("f.rs", &["old"], &["fn a() {}", "fn b() {}"]);
        assert_eq!(
            mark_key("HEAD", &it_a, &src).unwrap(),
            mark_key("HEAD", &it_b, &src).unwrap()
        );
    }

    #[test]
    fn mark_key_falls_back_to_enclosing_when_no_symbols() {
        // a body-edit hunk (no `symbols`) still gets a key, via `enclosing`
        let it = item_with("f.rs", [1, 1], [1, 1], vec![], Some("A.run"));
        let src = sources_for("f.rs", &["old"], &["new"]);
        assert!(mark_key("HEAD", &it, &src).is_some());
    }

    #[test]
    fn mark_key_none_without_source_content() {
        let it = item_with("f.rs", [1, 1], [1, 1], vec![], None);
        let empty: Sources = HashMap::new();
        assert!(mark_key("HEAD", &it, &empty).is_none());
    }

    #[test]
    fn mark_key_differs_across_rev_and_path() {
        let it = item_with("f.rs", [1, 1], [1, 1], vec![sym("run", "function_item", None)], None);
        let src = sources_for("f.rs", &["old"], &["new"]);
        let k1 = mark_key("HEAD", &it, &src).unwrap();
        let k2 = mark_key("abc123", &it, &src).unwrap();
        assert_ne!(k1, k2, "different revs must not collide");

        let it2 = item_with("g.rs", [1, 1], [1, 1], vec![sym("run", "function_item", None)], None);
        let mut src2 = src.clone();
        src2.insert("g.rs".to_string(), src2["f.rs"].clone());
        let k3 = mark_key("HEAD", &it2, &src2).unwrap();
        assert_ne!(k1, k3, "different paths must not collide even with identical content/symbol");
    }

    #[test]
    fn prune_marks_drops_old_entries_and_keeps_recent() {
        let now = 1_000_000_000u64;
        let mut marks: HashMap<u64, u64> = HashMap::new();
        marks.insert(1, now); // just written
        marks.insert(2, now - MARK_TTL_SECS + 10); // just inside the window
        marks.insert(3, now - MARK_TTL_SECS - 10); // just outside — dropped
        marks.insert(4, 0); // ancient — dropped
        prune_marks(&mut marks, now);
        assert_eq!(marks.len(), 2);
        assert!(marks.contains_key(&1));
        assert!(marks.contains_key(&2));
        assert!(!marks.contains_key(&3));
        assert!(!marks.contains_key(&4));
    }

    #[test]
    fn load_marks_degrades_to_empty_on_a_missing_or_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("ordo-tui-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let missing = dir.join("missing.json");
        assert!(load_marks(&missing).is_empty());

        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, b"not json").unwrap();
        assert!(load_marks(&corrupt).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_marks_then_load_marks_round_trips() {
        let dir = std::env::temp_dir().join(format!("ordo-tui-test-roundtrip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("marks.json");
        let mut marks: HashMap<u64, u64> = HashMap::new();
        marks.insert(0xdeadbeef, 123);
        marks.insert(0x1, 456);
        save_marks(&path, &marks);
        let got = load_marks(&path);
        assert_eq!(got, marks);

        // the file on disk reveals nothing about the code under review — no
        // path, symbol name, or source text, only hex keys and timestamps
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        for (k, v) in obj {
            assert!(u64::from_str_radix(k, 16).is_ok(), "key must be plain hex: {k}");
            assert!(v.is_u64(), "value must be a plain timestamp: {v}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- negative globs ----

    fn filt(pats: &[&str]) -> Filter {
        let globs = build_globs(&pats.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        Filter {
            globs,
            skip_generated: false,
            negatives_emptied: std::cell::Cell::new(false),
            tally: std::cell::Cell::new(Ledger::default()),
        }
    }

    #[test]
    fn positives_only_keep_only_matching_paths() {
        let f = filt(&["src/*"]);
        assert!(f.keep("src/a.rs"));
        assert!(!f.keep("tests/a.rs"));
    }

    #[test]
    fn negatives_only_keep_everything_except_those() {
        let f = filt(&["!tests/*"]);
        assert!(f.keep("src/a.rs"));
        assert!(f.keep("README.md"));
        assert!(!f.keep("tests/a.rs"));
    }

    #[test]
    fn positive_and_negative_combine_as_and() {
        let f = filt(&["src/*", "!src/generated/*"]);
        assert!(f.keep("src/a.rs"));
        assert!(!f.keep("src/generated/x.rs"));
        assert!(!f.keep("tests/a.rs"), "outside the positive set entirely");
    }

    #[test]
    fn a_negative_excludes_a_path_a_positive_also_matches_regardless_of_order() {
        // deliberately unlike .gitignore: order on the command line never
        // matters, and a later positive can never re-include what a negative
        // excluded
        let f = filt(&["!src/a.rs", "src/*"]);
        assert!(!f.keep("src/a.rs"));
        let f2 = filt(&["src/*", "!src/a.rs"]);
        assert!(!f2.keep("src/a.rs"));
    }

    #[test]
    fn escaped_bang_is_a_literal_positive_pattern() {
        let f = filt(&["\\!weird"]);
        assert!(f.keep("!weird"));
        assert!(!f.keep("weird"));
    }

    #[test]
    fn apply_note_distinguishes_negatives_emptying_it_from_no_positive_match() {
        let f = filt(&["!src/*"]);
        let kept = f.apply(vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
        assert!(kept.is_empty());
        assert_eq!(f.note(), " (every matching path was excluded by a negative glob)");

        let f2 = filt(&["nomatch/*"]);
        let kept2 = f2.apply(vec!["src/a.rs".to_string()]);
        assert!(kept2.is_empty());
        assert_eq!(f2.note(), " matching the given globs");
    }

    #[test]
    fn filter_command_glob_type_supports_negatives_too() {
        // `:filter` reuses `build_globs`, so a single negative pattern like
        // `!tests/*` narrows to "everything except tests" the same as the CLI
        let globs = build_globs(&["!tests/*".to_string()]).unwrap();
        let mut a = test_item("src/a.rs");
        a.comment = false;
        let items = vec![test_item("src/a.rs"), test_item("tests/b.rs")];
        assert_eq!(compute_view(&items, false, true, Some(&globs)), vec![0]);
    }

    // ---- theme selection ----

    #[test]
    fn theme_selects_dark_and_light_by_name() {
        assert!(theme("dark").is_some());
        assert!(theme("light").is_some());
    }

    #[test]
    fn theme_rejects_an_unknown_name() {
        assert!(theme("nonsense").is_none());
    }

    #[test]
    fn light_theme_is_not_the_dark_values_inverted() {
        let dark = Theme::dark();
        let light = Theme::light();
        // a real, distinct palette — not a placeholder equal to dark, and not
        // literally 255-x of dark's channels either
        assert!(!colors_eq(dark.add_bg, light.add_bg));
        let Color::Rgb(dr, dg, db) = dark.add_bg else { panic!("dark add_bg not Rgb") };
        let Color::Rgb(lr, lg, lb) = light.add_bg else { panic!("light add_bg not Rgb") };
        assert!(!(lr == 255 - dr && lg == 255 - dg && lb == 255 - db), "not a bitwise inversion");
    }

    fn colors_eq(a: Color, b: Color) -> bool {
        matches!((a, b), (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) if ar == br && ag == bg && ab == bb)
    }

    // ---- :group header rows ----

    fn grouped_item(path: &str, group: &str) -> Item {
        let mut it = test_item(path);
        it.group = group.to_string();
        it
    }

    #[test]
    fn display_rows_inserts_one_header_per_contiguous_group_run() {
        let items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        let mut groups = HashMap::new();
        groups.insert("g0".to_string(), "same definition: run".to_string());
        groups.insert("g1".to_string(), "same scope: top-level".to_string());
        let view = vec![0, 1, 2];

        let rows = display_rows(&view, &items, &groups, true);
        let kinds: Vec<&str> = rows
            .iter()
            .map(|r| match r {
                DisplayRow::Header(_) => "header",
                DisplayRow::Item(_) => "item",
            })
            .collect();
        assert_eq!(kinds, vec!["header", "item", "item", "header", "item"]);
        let DisplayRow::Header(reason) = &rows[0] else { panic!("expected a header") };
        assert_eq!(reason, "same definition: run");
    }

    #[test]
    fn display_rows_is_flat_view_when_groups_are_off() {
        let items = vec![grouped_item("a.rs", "g0"), grouped_item("b.rs", "g1")];
        let groups = HashMap::new();
        let view = vec![0, 1];
        let rows = display_rows(&view, &items, &groups, false);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| matches!(r, DisplayRow::Item(_))));
    }

    #[test]
    fn display_row_of_skips_headers_so_selection_never_lands_on_one() {
        let items = vec![
            grouped_item("a.rs", "g0"),
            grouped_item("a.rs", "g0"),
            grouped_item("b.rs", "g1"),
        ];
        let groups = HashMap::new(); // reason lookup irrelevant to row placement
        let view = vec![0, 1, 2];
        let rows = display_rows(&view, &items, &groups, true);

        for pos in 0..view.len() {
            let row = display_row_of(&view, &items, true, pos);
            assert!(
                matches!(rows[row], DisplayRow::Item(_)),
                "selection at view pos {pos} landed on row {row}, which is a header"
            );
        }
        // and the header count lines up: 2 groups among 3 items -> 2 headers,
        // so row indices for view positions [0,1,2] are [1,2,4]
        assert_eq!(
            (0..view.len()).map(|p| display_row_of(&view, &items, true, p)).collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
    }

    #[test]
    fn display_row_of_is_identity_when_groups_are_off() {
        let items = vec![grouped_item("a.rs", "g0"), grouped_item("b.rs", "g1")];
        assert_eq!(display_row_of(&[0, 1], &items, false, 0), 0);
        assert_eq!(display_row_of(&[0, 1], &items, false, 1), 1);
    }

    // ---- `:e` — reload carry-forward ----

    #[test]
    fn carry_across_reload_keeps_keymap_and_theme() {
        let mut app = test_app(0);
        app.keys = keymap("vscode").unwrap();
        app.theme = Theme::light();
        let (keys, theme) = carry_across_reload(&app);
        assert_eq!(keys.name, "vscode");
        assert!(colors_eq(theme.add_bg, Theme::light().add_bg));
    }

    #[test]
    fn e_command_rejects_an_empty_argument() {
        let mut app = test_app(0);
        match execute_command(&mut app, "e") {
            Err(msg) => assert!(msg.contains("usage")),
            Ok(_) => panic!("expected an error for a missing rev argument"),
        }
    }

    #[test]
    fn e_command_rejects_an_unresolvable_revision() {
        let mut app = test_app(0);
        match execute_command(&mut app, "e not-a-real-rev-xyzzy-12345") {
            Err(msg) => assert!(msg.contains("not-a-real-rev-xyzzy-12345")),
            Ok(_) => panic!("expected an error for an unresolvable rev"),
        }
    }

    #[test]
    fn e_command_resolves_head_to_a_reload_outcome() {
        // relies on the test binary running inside the ordo git checkout,
        // same assumption `resolve`'s own callers make
        let mut app = test_app(0);
        match execute_command(&mut app, "e HEAD") {
            Ok(CommandOutcome::Reload(_, rev)) => assert_eq!(rev, "HEAD"),
            other => panic!("expected a Reload outcome, got {}", other.is_ok()),
        }
    }

    #[test]
    fn arg_candidates_e_offers_the_given_rev_pool() {
        let revs = lines(&["zz", "HEAD", "main"]);
        assert_eq!(arg_candidates("e", &[], &[], &revs), revs);
        assert_eq!(command_completions("e H", &[], &[], &revs), vec!["HEAD".to_string()]);
    }

    // ---- `<base>..zz` / `<base>...zz` parsing (relies on the test binary
    // running inside the ordo git checkout, same assumption `resolve`'s own
    // callers make) ----

    #[test]
    fn base_dotdot_zz_resolves_to_a_worktree_range_at_base() {
        let base = git(&["rev-parse", "--verify", "-q", "main^{commit}"]);
        let base = base.trim().to_string();
        match resolve("main..zz") {
            Some(Target::WorktreeRange(b)) => assert_eq!(b, base),
            _ => panic!("expected a WorktreeRange at main"),
        }
    }

    #[test]
    fn base_dotdotdot_zz_resolves_to_the_merge_base_with_head() {
        let merge_base = git(&["merge-base", "main", "HEAD"]);
        let merge_base = merge_base.trim().to_string();
        match resolve("main...zz") {
            Some(Target::WorktreeRange(b)) => assert_eq!(b, merge_base),
            _ => panic!("expected a WorktreeRange at the merge base"),
        }
    }

    #[test]
    fn zz_on_the_left_is_rejected() {
        assert!(resolve_range("zz..main").is_none());
        assert!(resolve_range("zz...main").is_none());
        // resolve() as a whole also refuses it, same as any unresolvable rev
        assert!(resolve("zz..main").is_none());
    }

    #[test]
    fn plain_zz_is_unchanged() {
        assert!(matches!(resolve("zz"), Some(Target::Uncommitted)));
    }

    #[test]
    fn ordinary_commit_range_is_unchanged() {
        match resolve("main..HEAD") {
            Some(Target::Range(_, tip)) => {
                let head = git(&["rev-parse", "--verify", "-q", "HEAD"]);
                assert_eq!(tip, head.trim());
            }
            _ => panic!("expected a plain commit Range"),
        }
    }
}
